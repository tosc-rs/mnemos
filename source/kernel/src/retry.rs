use core::{fmt, future::Future};
use maitake::time::{self, Duration};
use tracing;

/// An exponential backoff.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExpBackoff {
    min: Duration,
    max: Duration,
    cur: Duration,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Retry<P = AlwaysRetry, B = ExpBackoff> {
    predicate: P,
    backoff: B,
}

/// A retry policy which will retry errors up to `max` times, and then fail.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WithMaxRetries<P> {
    predicate: P,
    max: usize,
    cur: usize,
}

/// A retry policy which always retries errors.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AlwaysRetry;

/// A backoff strategy for retries.
pub trait Backoff {
    fn backoff(&mut self) -> time::Duration;
    fn reset(&mut self);
}

/// A strategy for determining whether an error is retryable.
pub trait ShouldRetry<E> {
    /// Returns `true` if the provided error is retryable.
    fn should_retry(&mut self, error: &E) -> bool;
    fn reset(&mut self);
}

// === impl ExpBackoff ===

impl ExpBackoff {
    /// The default maximum retry backoff.
    pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(2);
    pub const DEFAULT_MIN_BACKOFF: Duration = Duration::from_millis(5);

    #[must_use]
    pub const fn new(min: Duration) -> Self {
        Self {
            max: Self::DEFAULT_MAX_BACKOFF,
            min,
            cur: min,
        }
    }

    /// Sets the maximum duration to back off for.
    ///
    /// Once the backoff duration reaches the maximum, it will no longer
    /// increase until the backoff is [`reset`](Self::reset).
    #[must_use]
    pub const fn with_max_backoff(self, max: Duration) -> Self {
        Self { max, ..self }
    }

    /// Returns the current backoff, incrementing the backoff returned by the
    /// next call to `backoff`.
    #[must_use]
    pub fn backoff(&mut self) -> Duration {
        tracing::trace!("backing off for {:?}...", self.cur);

        let cur = self.cur;

        if self.cur < self.max {
            self.cur *= 2;
        }

        cur
    }

    pub async fn wait(&mut self) {
        time::sleep(self.backoff()).await
    }

    /// Reset the backoff to the `min` value.
    pub fn reset(&mut self) {
        tracing::trace!("reset backoff to {:?}", self.min);
        self.cur = self.min;
    }

    #[inline]
    #[must_use]
    pub fn current(&self) -> Duration {
        self.cur
    }
}

impl Default for ExpBackoff {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MIN_BACKOFF).with_max_backoff(Self::DEFAULT_MAX_BACKOFF)
    }
}

impl Backoff for ExpBackoff {
    fn backoff(&mut self) -> Duration {
        ExpBackoff::backoff(self)
    }

    fn reset(&mut self) {
        ExpBackoff::reset(self)
    }
}

// === impl Backoff for Duration ===

impl Backoff for Duration {
    fn backoff(&mut self) -> time::Duration {
        *self
    }

    fn reset(&mut self) {}
}

// === impl ShouldRetry ===

impl<F, E> ShouldRetry<E> for F
where
    F: Fn(&E) -> bool,
{
    fn should_retry(&mut self, error: &E) -> bool {
        self(error)
    }

    fn reset(&mut self) {}
}

impl<P, E> ShouldRetry<E> for WithMaxRetries<P>
where
    P: ShouldRetry<E>,
{
    fn should_retry(&mut self, error: &E) -> bool {
        if self.cur > self.max {
            tracing::debug!(max = self.max, "maximum retry limit reached!");
            return false;
        }
        if self.predicate.should_retry(error) {
            self.cur += 1;
            tracing::trace!(remaining = self.max - self.cur, "retrying...");
            true
        } else {
            false
        }
    }

    fn reset(&mut self) {
        self.cur = 0;
    }
}

impl<E> ShouldRetry<E> for AlwaysRetry {
    fn should_retry(&mut self, _: &E) -> bool {
        true
    }

    fn reset(&mut self) {}
}

// === impl Retry ===

impl Default for Retry {
    fn default() -> Self {
        Self::new(AlwaysRetry, ExpBackoff::default())
    }
}

impl<P, B> Retry<P, B>
where
    B: Backoff,
{
    #[must_use]
    pub const fn new(predicate: P, backoff: B) -> Self {
        Self { predicate, backoff }
    }

    /// Sets the [predicate](ShouldRetry) used to determine if an error is
    /// retryable.
    ///
    /// If [`predicate.should_retry()`](ShouldRetry::should_retry) returns
    /// `true` for a given error, the error is retried.
    /// Otherwise, the error is not retried.
    #[must_use]
    pub fn with_predicate<P2>(self, predicate: P2) -> Retry<P2, B> {
        Retry {
            predicate,
            backoff: self.backoff,
        }
    }

    /// Sets the [backoff policy](Backoff) used to determine how long to back
    /// off for between retries.
    #[must_use]
    pub fn with_backoff<B2: Backoff>(self, backoff: B2) -> Retry<P, B2> {
        Retry {
            predicate: self.predicate,
            backoff,
        }
    }

    /// Sets a maximum retry limit of `max` retries. If an operation would be
    /// retried more than `max` times, it will fail, regardless of whether the
    /// `P` indicates it is retryable.
    pub fn with_max_retries(self, max: usize) -> Retry<WithMaxRetries<P>, B> {
        Retry {
            predicate: WithMaxRetries {
                predicate: self.predicate,
                max,
                cur: 0,
            },
            backoff: self.backoff,
        }
    }

    pub async fn retry<'op, T, E, F>(&mut self, mut op: impl FnMut() -> F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>> + 'op,
        P: ShouldRetry<E>,
        E: fmt::Display,
    {
        self.backoff.reset();
        self.predicate.reset();

        loop {
            match op().await {
                Ok(t) => return Ok(t),
                Err(error) if !self.predicate.should_retry(&error) => {
                    tracing::debug!(%error, "error is not retryable!");
                    return Err(error);
                }
                Err(error) => {
                    let backoff = self.backoff.backoff();
                    tracing::trace!("backing off for {backoff:?}...");
                    time::sleep(backoff).await;

                    tracing::debug!(%error, "retrying after backoff...");
                }
            }
        }
    }

    /// Retry the asynchronous operation returned by `F`.
    pub async fn retry_with_input<I, T, E, F>(
        &mut self,
        input: I,
        mut op: impl FnMut(I) -> F,
    ) -> Result<T, E>
    where
        F: Future<Output = (I, Result<T, E>)>,
        P: ShouldRetry<E>,
        E: fmt::Display,
    {
        self.backoff.reset();
        self.predicate.reset();

        let mut input = Some(input);
        loop {
            let i = input.take().unwrap();
            match op(i).await {
                (_, Ok(t)) => return Ok(t),
                (_, Err(error)) if !self.predicate.should_retry(&error) => {
                    tracing::debug!(%error, "error is not retryable!");
                    return Err(error);
                }
                (i, Err(error)) => {
                    let backoff = self.backoff.backoff();
                    tracing::trace!("backing off for {backoff:?}...");
                    time::sleep(backoff).await;

                    tracing::debug!(%error, "retrying after backoff...");
                    input = Some(i);
                }
            }
        }
    }
}
