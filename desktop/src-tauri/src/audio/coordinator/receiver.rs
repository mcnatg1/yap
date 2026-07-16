use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[cfg(test)]
use std::sync::Arc;

use super::sink_types::BoundedReceiver;

impl<T> BoundedReceiver<T> {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.claim_published_frame() {
                match self
                    .receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(item) => {
                        #[cfg(test)]
                        self.run_after_receive_hook_for_test();
                        return Ok(item);
                    }
                    Err(error) => {
                        self.restore_claimed_frame();
                        return Err(error);
                    }
                }
            }
            if self.state.closed.load(Ordering::Acquire) {
                return Err(std::sync::mpsc::RecvTimeoutError::Disconnected);
            }
            if Instant::now() >= deadline {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(test)]
    pub(super) fn set_after_receive_hook_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.state.after_receive_hook.lock().unwrap() = Some(hook);
    }

    fn claim_published_frame(&self) -> bool {
        let mut published = self.state.published_frames.load(Ordering::Acquire);
        loop {
            if published == 0 {
                return false;
            }
            match self.state.published_frames.compare_exchange_weak(
                published,
                published - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let result = self.state.queued_frames.fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |queued| queued.checked_sub(1),
                    );
                    debug_assert!(result.is_ok(), "a published frame must reserve queue depth");
                    return true;
                }
                Err(observed) => published = observed,
            }
        }
    }

    fn restore_claimed_frame(&self) {
        self.state.queued_frames.fetch_add(1, Ordering::AcqRel);
        self.state.published_frames.fetch_add(1, Ordering::Release);
    }

    #[cfg(test)]
    fn run_after_receive_hook_for_test(&self) {
        if let Some(hook) = self.state.after_receive_hook.lock().unwrap().as_ref() {
            hook();
        }
    }
}

impl<T> Drop for BoundedReceiver<T> {
    fn drop(&mut self) {
        self.state.queued_frames.store(0, Ordering::Release);
        self.state.published_frames.store(0, Ordering::Release);
        self.state.closed.store(true, Ordering::Release);
    }
}
