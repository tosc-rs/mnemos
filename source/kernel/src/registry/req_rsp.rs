use super::Service;
use crate::mnemos_alloc::containers::Arc;
use crate::Kernel;
use calliope::{
    message,
    req_rsp::{Request, Response},
    tricky_pipe::mpsc::{
        error::{RecvError, SendError, TrySendError},
        Receiver, Sender,
    },
};
use futures::pin_mut;
use maitake::sync::wait_map::{WaitError, WaitMap, WakeOutcome};
use portable_atomic::{AtomicUsize, Ordering};
use tracing::Instrument;

pub struct KernelReqRspHandle<S: ReqRspService> {
    tx: Sender<Request<S::Request>, message::Reset>,
    state: Arc<SharedState<S::Response>>,
}

struct SharedState<Rsp> {
    seq: AtomicUsize,
    dispatcher: WaitMap<usize, Rsp>,
}

pub trait ReqRspService:
    Service<ClientMsg = Request<Self::Request>, ServerMsg = Response<Self::Response>>
{
    type Request: 'static;
    type Response: 'static;
}

enum RequestError {
    Reset(message::Reset),
    SeqInUse,
}

// === impl Client ===

impl<S> KernelReqRspHandle<S>
where
    S: ReqRspService,
{
    pub async fn new(k: &'static Kernel, handle: super::KernelHandle<S>) -> Self {
        let (tx, rx) = handle.chan.split();
        let state = Arc::new(SharedState {
            seq: AtomicUsize::new(0),
            dispatcher: WaitMap::new(),
        })
        .await;
        k.spawn(state.clone().dispatch(rx).instrument(
            tracing::debug_span!("ReqRsp::dispatch", svc = %core::any::type_name::<S>()),
        ))
        .await;

        Self { tx, state }
    }

    pub async fn request(&self, req: S::Request) -> Result<S::Response, message::Reset> {
        #[cfg_attr(debug_assertions, allow(unreachable_code))]
        let handle_wait_error = |err: WaitError| match err {
            WaitError::Closed => {
                let error = self.tx.try_reserve().expect_err(
                    "if the waitmap was closed, then the channel should \
                        have been closed with an error!",
                );
                if let TrySendError::Error { error, .. } = error {
                    return RequestError::Reset(error);
                }

                #[cfg(debug_assertions)]
                unreachable!(
                    "closing the channel with an error should have priority \
                    over full/disconnected errors."
                );

                RequestError::Reset(message::Reset::BecauseISaidSo)
            }
            WaitError::Duplicate => RequestError::SeqInUse,
            WaitError::AlreadyConsumed => {
                unreachable!("data should not already be consumed, this is a bug")
            }
            WaitError::NeverAdded => {
                unreachable!("we ensured the waiter was added, this is a bug!")
            }
            error => {
                #[cfg(debug_assertions)]
                todo!(
                    "james added a new WaitError variant that we don't \
                    know how to handle: {error:}"
                );

                #[cfg_attr(debug_assertions, allow(unreachable_code))]
                RequestError::Reset(message::Reset::BecauseISaidSo)
            }
        };

        // aquire a send permit first --- this way, we don't increment the
        // sequence number until we actually have a channel reservation.
        let permit = self.tx.reserve().await.map_err(|e| match e {
            SendError::Disconnected(()) => message::Reset::BecauseISaidSo,
            SendError::Error { error, .. } => error,
        })?;

        loop {
            let seq = self.state.seq.fetch_add(1, Ordering::Relaxed);
            // ensure waiter is enqueued before sending the request.
            let wait = self.state.dispatcher.wait(seq);
            pin_mut!(wait);
            match wait.as_mut().enqueue().await.map_err(handle_wait_error) {
                Ok(_) => {}
                Err(RequestError::Reset(reset)) => return Err(reset),
                Err(RequestError::SeqInUse) => {
                    // NOTE: yes, in theory, this loop *could* never terminate,
                    // if *all* sequence numbers have a currently-in-flight
                    // request. but, if you've somehow managed to spawn
                    // `usize::MAX` request tasks at the same time, and none of
                    // them have completed, you probably have worse problems...
                    tracing::trace!(seq, "sequence number in use, retrying...");
                    continue;
                }
            };

            // actually send the message...
            permit.send(Request::new(seq, req));

            return match wait.await.map_err(handle_wait_error) {
                Ok(rsp) => Ok(rsp),
                Err(RequestError::Reset(reset)) => Err(reset),
                Err(RequestError::SeqInUse) => unreachable!(
                    "we should have already enqueued the waiter, so its \
                    sequence number should be okay. this is a bug!"
                ),
            };
        }
    }

    /// Shut down the client dispatcher for this `Client`.
    ///
    /// This will fail any outstanding `Request` futures, and reset the
    /// connection.
    pub fn shutdown(&self) {
        tracing::debug!("shutting down client...");
        self.channel
            .close_with_error(message::Reset::BecauseISaidSo);
        self.dispatcher.close();
    }
}

impl<S, Req, Rsp> ReqRspService for S
where
    S: Service<ClientMsg = Request<Req>, ServerMsg = Response<Rsp>>,
    Req: 'static,
    Rsp: 'static,
{
    type Request = Req;
    type Response = Rsp;
}

impl<Rsp> SharedState<Rsp> {
    /// Run the client's dispatcher in the background until cancelled or the
    /// connection is reset.
    async fn dispatch(self: Arc<Self>, rx: Receiver<Response<Rsp>, message::Reset>) {
        loop {
            let rsp = match rx.recv().await {
                Ok(msg) => msg,
                Err(reset) => {
                    let reset = match reset {
                        RecvError::Error(e) => e,
                        _ => message::Reset::BecauseISaidSo,
                    };

                    tracing::debug!(%reset, "client connection reset, shutting down...");
                    rx.close_with_error(reset);
                    return;
                }
            };
            let seq = rsp.seq();
            let body = rsp.into_body();

            tracing::trace!(seq, "dispatching response...");

            match self.dispatcher.wake(&seq, body) {
                WakeOutcome::Woke => {
                    tracing::trace!(seq, "dispatched response");
                }
                WakeOutcome::Closed(_) => {
                    #[cfg(debug_assertions)]
                    unreachable!("the dispatcher should not be closed if it is still running...");
                }
                WakeOutcome::NoMatch(_) => {
                    tracing::debug!(seq, "client no longer interested in request");
                }
            };
        }
    }
}
