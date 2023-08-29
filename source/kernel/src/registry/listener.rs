use super::{Message, RegisteredDriver};
use crate::comms::{
    kchannel::{KChannel, KConsumer, KProducer},
    oneshot,
};
use futures::{select_biased, FutureExt};

/// A listener for incoming [`Connection`]s to a [`RegisteredDriver`].
pub struct Listener<D: RegisteredDriver> {
    rx: KConsumer<Connection<D>>,
}

/// A registration for a [`RegisteredDriver`]. This type is provided to
/// [`Registry::register`] in order to add the driver to the registry.
pub struct Registration<D: RegisteredDriver> {
    pub(super) tx: KProducer<Connection<D>>,
}

/// A connection request received from a [`Listener`].
#[non_exhaustive]
pub struct Connection<D: RegisteredDriver> {
    pub hello: D::Hello,
    pub accept: Accept<D>,
    // TODO(eliza): consider adding client metadata here?
}

/// Accepts or rejects an incoming [`Connection`].
pub struct Accept<D: RegisteredDriver> {
    pub(super) reply: oneshot::Sender<Result<Channel<D>, D::ConnectError>>,
}

/// A stream of incoming requests from all clients.
///
/// This type is used when a service wishes all clients to send requests to the
/// same channel. It automatically accepts all incoming connections with the
/// same request channel, and returns any received requests to the service.
///
/// A [`Listener`] can be converted into a [`RequestStream`] using the
/// [`Listener::into_request_stream`] method.
///
/// Any [`Hello`] messages received from new connections are discarded by the
/// [`RequestStream`], and connections are never [`reject`]ed with a
/// [`ConnectError`].
///
/// Note, however, that this type does *not* require that the
/// [`RegisteredDriver`] type's [`RegisteredDriver::Hello`] type is [`()`], or
/// that its [`RegisteredDriver::ConnectError`] type is
/// [`core::convert::Infallible`]. This is because a [`RegisteredDriver`]
/// *declaration* which includes a [`Hello`] and/or [`ConnectError`] type may be
/// implemented by a server that does not care about [`Hello`]s or about
/// [`reject`]ing connections on some platforms. Other platforms may
/// implement the same [`RegisteredDriver`] declaration with a service that does
/// consume [`Hello`]s or [`reject`] connections, but `RequestStream` is still
/// usable with that `RegisteredDriver` in cases where the implementation does
/// not need those features.
///
/// [`reject`]: Connection::reject
/// [`Hello`]: RegisteredDriver::Hello
/// [`ConnectError`]: RegisteredDriver::ConnectError
pub struct RequestStream<D: RegisteredDriver> {
    chan: KConsumer<Message<D>>,
    listener: Listener<D>,
}

pub enum AcceptError {
    /// The client of the connection has cancelled the connection.
    Canceled,
}

type Channel<D> = KProducer<Message<D>>;

// === impl Listener ===

impl<D: RegisteredDriver> Listener<D> {
    pub async fn new(incoming_capacity: usize) -> (Self, Registration<D>) {
        let (tx, rx) = KChannel::new(incoming_capacity).split();
        let registration = Registration { tx };
        let listener = Self { rx };
        (listener, registration)
    }

    pub async fn next(&self) -> Connection<D> {
        self.rx
            .dequeue_async()
            .await
            .expect("the kernel should never drop the sender end of a service's incoming channel!")
    }

    pub async fn try_next(&self) -> Option<Connection<D>> {
        self.rx.dequeue_sync()
    }

    /// Converts this `Listener` into a [`RequestStream`] --- a simple stream of
    /// incoming requests, which [accepts](Self::accept) all connections with
    /// the same channel.
    ///
    /// The next request from any client may be awaited from the
    /// [`RequestStream`] using the [`RequestStream::next_request`] method.
    ///
    /// This is useful when a service wishes to handle all requests with the
    /// same channel, rather than spawning separate worker tasks for each
    /// client, or routing requests based on a connection's [`Hello`] message.
    ///
    /// **Note**: Any [`Hello`] messages received from new connections are
    /// discarded by the [`RequestStream`].
    ///
    /// [`Hello`]: RegisteredDriver::Hello
    pub async fn into_request_stream(self, capacity: usize) -> RequestStream<D> {
        let chan = KChannel::new(capacity).into_consumer();
        RequestStream {
            chan,
            listener: self,
        }
    }
}

// === impl Connection ===

impl<D: RegisteredDriver> Connection<D> {
    /// Accept the connection, returning the provided `channel` to the client.
    pub fn accept(self, channel: Channel<D>) -> Result<(), AcceptError> {
        self.accept.accept(channel)
    }

    /// Reject the connection, returning the provided `error` to the client.
    pub fn reject(self, error: D::ConnectError) -> Result<(), AcceptError> {
        self.accept.reject(error)
    }

    pub fn split(self) -> (D::Hello, Accept<D>) {
        (self.hello, self.accept)
    }
}

// === impl Accept ===

impl<D: RegisteredDriver> Accept<D> {
    /// Accept the connection, returning the provided `channel` to the client.
    pub fn accept(self, channel: Channel<D>) -> Result<(), AcceptError> {
        match self.reply.send(Ok(channel)) {
            Ok(()) => Ok(()),
            Err(oneshot::ReusableError::ChannelClosed) => Err(AcceptError::Canceled),
            Err(error) => unreachable!(
                "we are the sender, so we should only ever see `ChannelClosed` errors: {error:?}"
            ),
        }
    }

    /// Reject the connection, returning the provided `error` to the client.
    pub fn reject(self, error: D::ConnectError) -> Result<(), AcceptError> {
        match self.reply.send(Err(error)) {
            Ok(()) => Ok(()),
            Err(oneshot::ReusableError::ChannelClosed) => Err(AcceptError::Canceled),
            Err(error) => unreachable!(
                "we are the sender, so we should only ever see `ChannelClosed` errors: {error:?}"
            ),
        }
    }
}

// === impl RequestStream ===

impl<D: RegisteredDriver> RequestStream<D> {
    /// Returns the next incoming message, accepting any new connections until a
    /// message is received.
    ///
    /// If all request senders have been dropped, this method waits until a new
    /// connection is available to accept, and then waits for a message from a
    /// client.
    ///
    /// **Note**: Any [`Hello`] messages received from new connections are
    /// discarded.
    ///
    /// [`Hello`]: RegisteredDriver::Hello
    pub async fn next_request(&self) -> Message<D> {
        loop {
            let conn = select_biased! {
                msg = self.chan.dequeue_async().fuse() => {
                    match msg {
                        Ok(msg) => return msg,
                        Err(_) => {
                            // if the request stream is "closed", that just
                            // means that all the senders are dropped. That
                            // doesn't mean that it's time for the service to
                            // die --- new receivers may be created by new
                            // incoming connections. So, wait for the next
                            // connection request.
                            self.listener.next().await
                        }
                    }
                },
                conn = self.listener.next().fuse() => {
                    conn
                }
            };

            tracing::trace!("accepting new connection...");
            if conn.accept(self.chan.producer()).is_err() {
                tracing::debug!("incoming connection canceled");
            }
        }
    }
}
