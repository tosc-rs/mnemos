use super::{Message, RegisteredDriver};
use crate::comms::{
    kchannel::{KChannel, KConsumer, KProducer},
    oneshot,
};

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

    pub async fn next(&mut self) -> Connection<D> {
        self.rx
            .dequeue_async()
            .await
            .expect("the kernel should never drop the sender end of a service's incoming channel!")
    }

    pub async fn try_next(&mut self) -> Option<Connection<D>> {
        self.rx.dequeue_sync()
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
