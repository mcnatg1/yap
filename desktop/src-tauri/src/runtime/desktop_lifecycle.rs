//! Owns application-wide background work that must not outlive the desktop process lifecycle.

use std::{
    future::Future,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
        Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

struct OwnedThread {
    name: &'static str,
    stop: Sender<()>,
    worker: JoinHandle<()>,
}

struct OwnedAsyncTask {
    worker: tauri::async_runtime::JoinHandle<()>,
}

pub(crate) struct DesktopLifecycle {
    shutting_down: AtomicBool,
    threads: Mutex<Vec<OwnedThread>>,
    async_tasks: Mutex<Vec<OwnedAsyncTask>>,
}

impl DesktopLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            shutting_down: AtomicBool::new(false),
            threads: Mutex::new(Vec::new()),
            async_tasks: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn spawn_periodic(
        &self,
        name: &'static str,
        interval: Duration,
        mut tick: impl FnMut() + Send + 'static,
    ) -> io::Result<()> {
        if interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} requires a non-zero interval"),
            ));
        }
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(io::Error::other(format!(
                "{name} cannot start during shutdown"
            )));
        }
        let (stop, stop_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stop_receiver.recv_timeout(interval) {
                    tick();
                }
            })
            .map_err(|error| {
                io::Error::new(error.kind(), format!("{name} failed to start: {error}"))
            })?;
        threads.push(OwnedThread { name, stop, worker });
        Ok(())
    }

    pub(crate) fn spawn_async_task(
        &self,
        name: &'static str,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> io::Result<()> {
        let mut tasks = self
            .async_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(io::Error::other(format!(
                "{name} cannot start during shutdown"
            )));
        }
        let worker = tauri::async_runtime::spawn(task);
        tasks.push(OwnedAsyncTask { worker });
        Ok(())
    }

    pub(crate) fn shutdown(&self) -> Vec<String> {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            return Vec::new();
        }

        let async_tasks = {
            let mut tasks = self
                .async_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *tasks)
        };
        for task in async_tasks {
            task.worker.abort();
        }

        let threads = {
            let mut threads = self
                .threads
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *threads)
        };
        for thread in &threads {
            let _ = thread.stop.send(());
        }
        threads
            .into_iter()
            .filter_map(|thread| {
                thread
                    .worker
                    .join()
                    .err()
                    .map(|_| format!("{} panicked during shutdown", thread.name))
            })
            .collect()
    }
}

impl Default for DesktopLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DesktopLifecycle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::{Duration, Instant},
    };

    use super::DesktopLifecycle;

    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn shutdown_wakes_and_joins_periodic_work() {
        let lifecycle = DesktopLifecycle::new();
        let ticks = Arc::new(AtomicUsize::new(0));
        let worker_ticks = Arc::clone(&ticks);
        let (first_tick, first_tick_receiver) = mpsc::channel();
        lifecycle
            .spawn_periodic("lifecycle-test", Duration::from_millis(2), move || {
                worker_ticks.fetch_add(1, Ordering::SeqCst);
                let _ = first_tick.send(());
            })
            .unwrap();
        first_tick_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let started = Instant::now();
        assert!(lifecycle.shutdown().is_empty());
        assert!(started.elapsed() < Duration::from_secs(1));
        let stopped_at = ticks.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(ticks.load(Ordering::SeqCst), stopped_at);
        assert!(lifecycle.shutdown().is_empty());
    }

    #[test]
    fn work_cannot_start_after_shutdown() {
        let lifecycle = DesktopLifecycle::new();
        assert!(lifecycle.shutdown().is_empty());
        assert!(lifecycle
            .spawn_periodic("late-work", Duration::from_secs(1), || {})
            .unwrap_err()
            .to_string()
            .contains("during shutdown"));
        assert!(lifecycle
            .spawn_async_task("late-async-work", async {})
            .unwrap_err()
            .to_string()
            .contains("during shutdown"));
    }

    #[test]
    fn shutdown_aborts_owned_async_work() {
        let lifecycle = DesktopLifecycle::new();
        let dropped = Arc::new(AtomicBool::new(false));
        let task_dropped = Arc::clone(&dropped);
        let (started, started_receiver) = mpsc::channel();
        lifecycle
            .spawn_async_task("async-lifecycle-test", async move {
                let _drop_signal = DropSignal(task_dropped);
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            })
            .unwrap();
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(lifecycle.shutdown().is_empty());
        let deadline = Instant::now() + Duration::from_secs(1);
        while !dropped.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(dropped.load(Ordering::SeqCst));
    }
}
