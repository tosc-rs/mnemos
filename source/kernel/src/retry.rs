use crate::tracing;
use core::{fmt, future::Future, num::NonZeroUsize};
use maitake::time::{self, Duration};

/// An exponential backoff.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExpBackoff {
    min: Duration,
    max: Duration,
    cur: Duration,
}

pub struct Retry<P = ()> {
    predicate: P,
    backoff: ExpBackoff,
    max: Option<NonZeroUsize>,
}

pub struct MaxRetries<P> {
    inner: P,
    max: usize,
    cur: usize,
}

pub trait ShouldRetry<E> {
    fn should_retry(&mut self, error: &E) -> bool;
    fn reset(&mut self) {
        // nop
    }
}

impl ExpBackoff {
    const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(2);

    #[must_use]
    pub const fn new(min: Duration) -> Self {
        Self {
            max: Self::DEFAULT_MAX_BACKOFF,
            min,
            cur: min,
        }
    }

    #[must_use]
    pub const fn with_max(self, max: Duration) -> Self {
        Self { max, ..self }
    }

    /// Wait until the current backoff period has elapsed, incrementing the
    /// backoff for the next call to `wait`.
    pub async fn wait(&mut self) {
        tracing::trace!("backing off for {:?}...", self.cur);

        let cur = self.cur;

        if self.cur < self.max {
            self.cur *= 2;
        }

        time::sleep(cur).await
    }

    /// Reset the backoff to the `min` value.
    pub fn reset(&mut self) {
        tracing::trace!("reset backoff to {:?}", self.min);
        self.cur = self.min;
    }

    pub fn current(&self) -> Duration {
        self.cur
    }
}

// === impl Retry ===

impl Retry {
    pub const fn new(backoff: ExpBackoff) -> Self {
        Self {
            predicate: (),
            backoff,
            max: None,
        }
    }
}

impl<P> Retry<P> {
    pub fn with_predicate<P2>(self, predicate: P2) -> Retry<P2> {
        Retry {
            predicate,
            backoff: self.backoff,
            max: self.max,
        }
    }

    pub fn with_max(self, max: impl Into<Option<NonZeroUsize>>) -> Self {
        Self {
            max: max.into(),
            ..self
        }
    }

    pub async fn retry<T, E, F>(&mut self, op: impl Fn() -> F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>>,
        P: ShouldRetry<E>,
        E: fmt::Display,
    {
        self.predicate.reset();
        self.backoff.reset();
        let mut retries: usize = 0;
        loop {
            match op().await {
                Ok(t) => {
                    if retries > 0 {
                        tracing::debug!(retries, "succeeded after retrying");
                    }
                    return Ok(t);
                }
                Err(error) if !self.predicate.should_retry(&error) => {
                    tracing::debug!(%error, "error is not retryable!");
                    return Err(error);
                }
                Err(error) => {
                    if let Some(max) = self.max {
                        if retries >= max.into() {
                            tracing::debug!(%error, max, "maximum retry limit reached!");
                            return Err(error);
                        }
                    }
                    self.backoff.wait().await;
                    tracing::debug!(%error, retries, "retrying after backoff...");
                    retries += 1;
                }
            }
        }
    }
}

impl<E> ShouldRetry<E> for () {
    fn should_retry(&mut self, _: &E) -> bool {
        true
    }
}

impl<E, F> ShouldRetry<E> for F
where
    F: FnMut(&E) -> bool,
{
    fn should_retry(&mut self, error: &E) -> bool {
        (self)(error)
    }
}

impl<E, P> ShouldRetry<E> for MaxRetries<P>
where
    P: ShouldRetry<E>,
{
    fn should_retry(&mut self, error: &E) -> bool {
        if self.inner.should_retry(error) && self.cur <= self.max {
            self.cur += 1;
            true
        } else {
            false
        }
    }

    fn reset(&mut self) {
        self.inner.reset();
        self.cur = 0;
    }
}
