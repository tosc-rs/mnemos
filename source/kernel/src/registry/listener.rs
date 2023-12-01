//! A [`Listener`] is used by a [`RegisteredDriver`] to [accept incoming
//! connections](Handshake) from clients.
#![warn(missing_docs)]
use super::Service;
use calliope::{
    req_rsp::{Request, Response},
    tricky_pipe::{bidi::BiDi, mpsc, oneshot},
};
use futures::{select_biased, FutureExt};

/// A listener for incoming connection [`Handshake`]s to a [`RegisteredDriver`].
#[must_use = "a `Listener` does nothing if incoming connections are not accepted"]
pub struct Listener<S: Service + 'static> {
    rx: mpsc::Receiver<Handshake<S>>,
}

/// A registration for a [`RegisteredDriver`]. This type is provided to
/// [`Registry::register`] in order to add the driver to the registry.
///
/// [`Registry::register`]: crate::registry::Registry::register
#[must_use = "a `Registration` does nothing if not registered with a `Registry`"]
pub struct Registration<S: Service + 'static> {
    pub(super) tx: mpsc::Sender<Handshake<S>>,
}

/// A connection request received from a [`Listener`].
///
/// A `Handshake` contains a [`Hello`] message sent by the client, which can
/// be used to identify the requested connection. The service may choose to
/// [`accept`](Self::accept) or [`reject`](Self::reject) the connection,
/// potentially using the value of the [`Hello`] message to make this decision.
///
/// [`Hello`]: RegisteredDriver::Hello
#[must_use = "a `Handshake` does nothing if not `accept`ed or `reject`ed"]
#[non_exhaustive]
pub struct Handshake<S: Service> {
    /// The [`RegisteredDriver::Hello`] message sent by the client to identify
    /// the requested incoming connection.
    pub hello: S::Hello,

    /// [Accepts](Accept::accept) or [rejects](Accept::reject) the handshake.
    ///
    /// The [`Handshake::accept`] and [`Handshake::reject`] methods may be used
    /// to accept the handshake, but the [`Accept`] type provides these methods
    /// on a separate type, so that the [`Hello` message](#structfield.hello)
    /// can be moved out of the `Handshake` value while still allowing the
    /// connection to be accepted.
    pub accept: Accept<S>,
    // TODO(eliza): consider adding client metadata here?
}

/// Accepts or rejects an incoming connection [`Handshake`].
#[must_use = "an `Accept` does nothing if not `accept`ed or `reject`ed"]
pub struct Accept<S: Service> {
    pub(super) reply: oneshot::Sender<HandshakeResult<S>>,
}

// /// A stream of incoming requests from all clients.
// ///
// /// This type is used when a service wishes all clients to send requests to the
// /// same channel. It automatically accepts all incoming connections with the
// /// same request channel, and returns any received requests to the service.
// ///
// /// A [`Listener`] can be converted into a [`RequestStream`] using the
// /// [`Listener::into_request_stream`] method.
// ///
// /// Any [`Hello`] messages received from new connections are discarded by the
// /// [`RequestStream`], and connections are never [`reject`]ed with a
// /// [`ConnectError`].
// ///
// /// Note, however, that this type does *not* require that the
// /// [`RegisteredDriver`] type's [`RegisteredDriver::Hello`] type is [`()`], or
// /// that its [`RegisteredDriver::ConnectError`] type is
// /// [`core::convert::Infallible`]. This is because a [`RegisteredDriver`]
// /// *declaration* which includes a [`Hello`] and/or [`ConnectError`] type may be
// /// implemented by a server that does not care about [`Hello`]s or about
// /// [`reject`]ing connections on some platforms. Other platforms may
// /// implement the same [`RegisteredDriver`] declaration with a service that does
// /// consume [`Hello`]s or [`reject`] connections, but `RequestStream` is still
// /// usable with that `RegisteredDriver` in cases where the implementation does
// /// not need those features.
// ///
// /// [`reject`]: Handshake::reject
// /// [`Hello`]: RegisteredDriver::Hello
// /// [`ConnectError`]: RegisteredDriver::ConnectError
// #[must_use = "a `RequestStream` does nothing if `next_request` is not called"]
// pub struct RequestStream<S: Service + 'static> {
//     // chan: KConsumer<Message<S>>,
//     listener: Listener<S>,
// }

/// Errors returned by [`Handshake::accept`], [`Accept::accept`],
/// [`Handshake::reject`], and [`Accept::reject`].
#[derive(Debug, Eq, PartialEq)]
pub enum AcceptError {
    /// The client of the connection has cancelled the connection.
    Canceled,
}

pub type Channel<S> =
    BiDi<<S as Service>::ClientMsg, <S as Service>::ServerMsg, calliope::message::Reset>;

pub(super) type HandshakeResult<S> = Result<Channel<S>, <S as Service>::ConnectError>;

// === impl Listener ===

impl<S: Service> Listener<S> {
    /// Returns a new `Listener` and an associated [`Registration`].
    ///
    /// The `Listener`'s channel will have capacity for up to
    /// `incoming_capacity` un-accepted connections before clients have to wait
    /// to send a connection.
    pub async fn new(incoming_capacity: u8) -> (Self, Registration<S>) {
        let pipe = mpsc::TrickyPipe::new(incoming_capacity);
        let tx = pipe.sender();
        let rx = pipe.receiver().unwrap();
        let registration = Registration { tx };
        let listener = Self { rx };
        (listener, registration)
    }

    /// Awaits the next incoming [`Handshake`] to this listener.
    ///
    /// This method returns a [`Handshake`] when a new incoming connection
    /// request is received. If no incoming connection is available, this method
    /// will yield until one is ready.
    ///
    /// To return an incoming connection if one is available, *without* waiting,
    /// use the [`try_handshake`](Self::try_handshake) method.
    pub async fn handshake(&self) -> Handshake<S> {
        self.rx
            .recv()
            .await
            // The sender end of the incoming connection channel is owned by the
            // kernel, so this never closes.
            .expect("the kernel should never drop the sender end of a service's incoming channel!")
    }

    /// Attempts to return the next incoming [`Handshake`], without waiting.
    ///
    /// To asynchronously wait until the next incoming connection is available,
    /// use [`handshake`](Self::handshake) instead..
    ///
    /// # Returns
    ///
    /// - [`Some`]`(`[`Handshake`]`)` if a new incoming connection is
    ///   available without waiting.
    /// - [`None`] if no incoming connection is available without waiting.
    pub async fn try_handshake(&self) -> Option<Handshake<S>> {
        self.rx.recv().await.ok()
    }

    // /// Converts this `Listener` into a [`RequestStream`] --- a simple stream of
    // /// incoming requests, which [accepts](Handshake::accept) all connections
    // /// with the same [`KChannel`].
    // ///
    // /// The next request from any client may be awaited from the
    // /// [`RequestStream`] using the [`RequestStream::next_request`] method.
    // ///
    // /// This is useful when a service wishes to handle all requests with the
    // /// same channel, rather than spawning separate worker tasks for each
    // /// client, or routing requests based on a connection's [`Hello`] message.
    // ///
    // /// **Note**: Any [`Hello`] messages received from new connections are
    // /// discarded by the [`RequestStream`].
    // ///
    // /// [`Hello`]: RegisteredDriver::Hello
    // pub async fn into_request_stream(self, capacity: usize) -> RequestStream<S> {
    //     //     let chan = KChannel::new(capacity).into_consumer();
    //     //     RequestStream {
    //     //         chan,
    //     //         listener: self,
    //     //     }
    //     todo!()
    // }
}

// === impl Handshake ===

impl<S: Service> Handshake<S> {
    /// Accept the connection, returning the provided `channel` to the client.
    ///
    /// Any requests sent by the client once the connection has been accepted
    /// will now be received by the [`KConsumer`] corresponding to the provided
    /// [`KProducer`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`()`]`)` if the connection was successfully accepted.
    /// - [`Err`]`(`[`AcceptError`]`)` if the connection was canceled by the
    ///   client. In this case, the client is no longer interested in the
    ///   connection (and may or may not still exist), and the service may
    ///   ignore this connection request.
    pub fn accept(self, channel: Channel<S>) -> Result<(), AcceptError> {
        self.accept.accept(channel)
    }

    /// Reject the connection, returning the provided `error` to the client.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`()`]`)` if the connection was successfully rejected and the
    ///   error was received by the client.
    /// - [`Err`]`(`[`AcceptError`]`)` if the connection was canceled by the
    ///   client.
    pub fn reject(self, error: S::ConnectError) -> Result<(), AcceptError> {
        self.accept.reject(error)
    }
}

// === impl Accept ===

impl<S: Service> Accept<S> {
    /// Accept the connection, returning the provided `channel` to the client.
    ///
    /// Any requests sent by the client once the connection has been accepted
    /// will now be received by the [`KConsumer`] corresponding to the provided
    /// [`KProducer`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`()`]`)` if the connection was successfully accepted.
    /// - [`Err`]`(`[`AcceptError`]`)` if the connection was canceled by the
    ///   client. In this case, the client is no longer interested in the
    ///   connection (and may or may not still exist), and the service may
    ///   ignore this connection request.
    pub fn accept(self, channel: Channel<S>) -> Result<(), AcceptError> {
        self.reply
            .send(Ok(channel))
            .map_err(|_| AcceptError::Canceled)
    }

    /// Reject the connection, returning the provided `error` to the client.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`()`]`)` if the connection was successfully rejected and the
    ///   error was received by the client.
    /// - [`Err`]`(`[`AcceptError`]`)` if the connection was canceled by the
    ///   client.
    pub fn reject(self, error: S::ConnectError) -> Result<(), AcceptError> {
        self.reply
            .send(Err(error))
            .map_err(|_| AcceptError::Canceled)
    }
}

// === impl RequestStream ===

// impl<S: Service> RequestStream<S> {
//     /// Returns the next incoming message, accepting any new connections until a
//     /// message is received.
//     ///
//     /// If all request senders have been dropped, this method waits until a new
//     /// connection is available to accept, and then waits for a message from a
//     /// client.
//     ///
//     /// **Note**: Any [`Hello`] messages received from new connections are
//     /// discarded.
//     ///
//     /// [`Hello`]: RegisteredDriver::Hello
//     pub async fn next_request(&self) -> Message<S> {
//         loop {
//             let conn = select_biased! {
//                 msg = self.chan.dequeue_async().fuse() => {
//                     match msg {
//                         Ok(msg) => return msg,
//                         Err(_) => {
//                             // if the request stream is "closed", that just
//                             // means that all the senders are dropped. That
//                             // doesn't mean that it's time for the service to
//                             // die --- new receivers may be created by new
//                             // incoming connections. So, wait for the next
//                             // connection request.
//                             self.listener.handshake().await
//                         }
//                     }
//                 },
//                 conn = self.listener.handshake().fuse() => {
//                     conn
//                 }
//             };

//             tracing::trace!("accepting new connection...");
//             if conn.accept(self.chan.producer()).is_err() {
//                 tracing::debug!("incoming connection canceled");
//             }
//         }
//     }
// }
