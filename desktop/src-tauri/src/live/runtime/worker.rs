use std::thread::{self, JoinHandle};

pub(super) fn join_worker(handle: JoinHandle<()>) -> Result<(), String> {
    if handle.thread().id() == thread::current().id() {
        return Err("Worker attempted to join itself.".to_string());
    }
    handle
        .join()
        .map_err(|_| "Worker panicked during shutdown.".to_string())
}
