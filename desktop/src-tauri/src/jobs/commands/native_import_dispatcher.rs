use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
};

use tauri::Manager;

use super::{
    command_error, emit_native_import_error, import_native_paths, JobCommandError,
    MAX_RECORDING_JOBS,
};

const NATIVE_IMPORT_BACKLOG: usize = 1;
type NativeImportSender = mpsc::SyncSender<Vec<PathBuf>>;
type NativeImportReceiver = mpsc::Receiver<Vec<PathBuf>>;

pub(crate) struct NativeImportDispatcher {
    batches: NativeImportSender,
    selection: NativeImportSelectionGate,
}

#[derive(Clone, Default)]
pub(super) struct NativeImportSelectionGate {
    active: Arc<AtomicBool>,
}

#[derive(Debug)]
pub(super) struct NativeImportSelectionLease {
    active: Arc<AtomicBool>,
}

pub(crate) fn install_native_import_dispatcher(app: &tauri::App) -> std::io::Result<()> {
    let dispatcher = NativeImportDispatcher::spawn(app.handle().clone())?;
    if app.manage(dispatcher) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "native import dispatcher is already installed",
        ))
    }
}

pub(crate) fn enqueue_native_import(app: &tauri::AppHandle, paths: Vec<PathBuf>) {
    let dispatcher = app.state::<NativeImportDispatcher>();
    if let Err(error) = dispatcher.enqueue(paths) {
        emit_native_import_error(app, &error);
    }
}

pub(super) fn begin_native_import_selection(
    app: &tauri::AppHandle,
) -> Result<NativeImportSelectionLease, JobCommandError> {
    app.state::<NativeImportDispatcher>().selection.try_begin()
}

impl NativeImportDispatcher {
    fn spawn(app: tauri::AppHandle) -> std::io::Result<Self> {
        let (batches, receiver) = native_import_channel();
        let _worker = thread::Builder::new()
            .name("native-recording-imports".into())
            .spawn(move || run_native_imports(app, receiver))?;
        Ok(Self {
            batches,
            selection: NativeImportSelectionGate::default(),
        })
    }

    fn enqueue(&self, paths: Vec<PathBuf>) -> Result<(), JobCommandError> {
        queue_native_import_batch(&self.batches, paths)
    }
}

impl NativeImportSelectionGate {
    pub(super) fn try_begin(&self) -> Result<NativeImportSelectionLease, JobCommandError> {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                command_error("IMPORT_BUSY", "Another recording picker is already active.")
            })?;
        Ok(NativeImportSelectionLease {
            active: Arc::clone(&self.active),
        })
    }
}

impl Drop for NativeImportSelectionLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn run_native_imports(app: tauri::AppHandle, receiver: NativeImportReceiver) {
    while let Ok(paths) = receiver.recv() {
        if let Err(error) = import_native_paths(&app, paths) {
            emit_native_import_error(&app, &error);
        }
    }
}

pub(super) fn native_import_channel() -> (NativeImportSender, NativeImportReceiver) {
    mpsc::sync_channel(NATIVE_IMPORT_BACKLOG)
}

pub(super) fn queue_native_import_batch(
    batches: &NativeImportSender,
    paths: Vec<PathBuf>,
) -> Result<(), JobCommandError> {
    if paths.len() > MAX_RECORDING_JOBS {
        return Err(command_error(
            "JOB_LIMIT_EXCEEDED",
            format!("Yap accepts at most {MAX_RECORDING_JOBS} recording jobs."),
        ));
    }
    match batches.try_send(paths) {
        Ok(()) => Ok(()),
        Err(mpsc::TrySendError::Full(_)) => Err(command_error(
            "IMPORT_BUSY",
            "Another recording import is already queued. Try again after it finishes.",
        )),
        Err(mpsc::TrySendError::Disconnected(_)) => Err(command_error(
            "IMPORT_UNAVAILABLE",
            "Recording import is temporarily unavailable.",
        )),
    }
}
