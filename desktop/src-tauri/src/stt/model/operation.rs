use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tokio::sync::Notify;

#[derive(Debug)]
struct DownloadOperationInner {
    generation: u64,
    cancelled: AtomicBool,
    cancellation: Notify,
    cleanup_failure: Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct DownloadOperation {
    inner: Arc<DownloadOperationInner>,
}

impl DownloadOperation {
    pub fn new(generation: u64) -> Self {
        Self {
            inner: Arc::new(DownloadOperationInner {
                generation,
                cancelled: AtomicBool::new(false),
                cancellation: Notify::new(),
                cleanup_failure: Mutex::new(None),
            }),
        }
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            self.inner.cancellation.notify_one();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn take_cleanup_failure(&self) -> Option<String> {
        self.inner
            .cleanup_failure
            .lock()
            .expect("download cleanup state poisoned")
            .take()
    }

    pub(super) async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            self.inner.cancellation.notified().await;
        }
    }

    pub(super) fn record_cleanup_failure(&self, message: String) {
        let mut failure = self
            .inner
            .cleanup_failure
            .lock()
            .expect("download cleanup state poisoned");
        match failure.as_mut() {
            Some(existing) => {
                existing.push_str("; ");
                existing.push_str(&message);
            }
            None => *failure = Some(message),
        }
    }
}
