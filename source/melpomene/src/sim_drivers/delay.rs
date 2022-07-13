use std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Poll, Waker},
    time::Duration,
};
use tokio::{task, time};

struct DelayInner {
    done: bool,
    waker: Option<Waker>,
}

pub struct Delay {
    inner: Arc<Mutex<DelayInner>>,
}

impl Drop for Delay {
    fn drop(&mut self) {
        // Take the waker on drop, ensuring the sleep thread won't wake a dead future
        let _ = self.inner.lock().unwrap().waker.take();
    }
}

impl Delay {
    pub fn new(dur: Duration) -> Self {
        let data1 = Arc::new(Mutex::new(DelayInner {
            done: false,
            waker: None,
        }));
        let data2 = data1.clone();
        let _ = task::spawn(async move {
            time::sleep(dur).await;
            let mut guard = data2.lock().unwrap();
            guard.done = true;
            if let Some(waker) = guard.waker.take() {
                waker.wake();
            }
        });

        Self { inner: data1 }
    }
}

impl Future for Delay {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let wake = cx.waker().clone();
        let mut guard = self.inner.lock().unwrap();
        if guard.done {
            Poll::Ready(())
        } else {
            guard.waker = Some(wake);
            Poll::Pending
        }
    }
}
