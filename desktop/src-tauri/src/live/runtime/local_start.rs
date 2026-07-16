//! Two-stage local start: establish durable capture before waiting for ASR warmup.

use std::sync::{atomic::Ordering, Arc};

use tauri::Manager;

use crate::audio::capture::CaptureAdapter;
use crate::audio::coordinator::{bounded_sink, SinkKind, RECORDING_QUEUE_CAPACITY};
use crate::audio::recording::RecordingSinkHandle;
use crate::audio::session::{SessionMetadata, SessionMode, SessionOrigin, TriggerMode};

use super::super::{
    devices, events, recordings,
    state::{LiveCaptureMode, LiveSessionState},
};
use super::asr_adapter::PendingAsrAdapter;
use super::capture_installation::CaptureInstallation;
use super::capture_worker::{run_capture_worker, CaptureWorkerContext};
use super::level_channel::level_channel;
use super::{LiveRuntime, LiveStartFailure, StartIntent};

pub(crate) struct LocalCaptureStart {
    pub(super) session: u64,
}

impl LiveRuntime {
    pub(crate) fn start_local_capture(
        &self,
        app: tauri::AppHandle,
        selected_device_id: Option<String>,
        capture_mode: LiveCaptureMode,
        intent: StartIntent,
    ) -> Result<Option<LocalCaptureStart>, LiveStartFailure> {
        let session = {
            let inner = self.inner.lock().expect("live runtime poisoned");
            if inner.is_capturing() {
                return Ok(None);
            }
            drop(inner);
            self.ensure_recording_ready_to_start()
                .map_err(|message| LiveStartFailure::new(0, message))?;
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let Some(session) = inner.begin_capture_session() else {
                return Ok(None);
            };
            self.active_session.store(session, Ordering::SeqCst);
            session
        };

        if !self.start_intent_is_current(intent) {
            return Ok(None);
        }

        let resolved = match devices::resolve_capture_device(selected_device_id.as_deref()) {
            Ok(resolved) => resolved,
            Err(error) => return Err(LiveStartFailure::new(session, error)),
        };
        let stream_config = resolved.config.config();
        let sample_format = resolved.config.sample_format();
        let (level_tx, level) = level_channel();
        let pending_asr = PendingAsrAdapter::new();
        let local_asr = pending_asr.sink();
        let capture_runtime = self.clone();
        let capture_app = app.clone();
        let capture_active_session = Arc::clone(&self.active_session);
        let (recording_sink, recording_rx) =
            bounded_sink(SinkKind::Recording, RECORDING_QUEUE_CAPACITY);
        let recording_directory = recordings::recordings_dir();
        let recording_reservation =
            crate::audio::recording::allocate_recording_session(&recording_directory)
                .map_err(|message| LiveStartFailure::new(session, message))?;
        let recording_session_id = recording_reservation.session_id().clone();
        let trigger_mode = match capture_mode {
            LiveCaptureMode::PushToTalk => TriggerMode::PushToTalk,
            LiveCaptureMode::Toggle => TriggerMode::Toggle,
        };
        let session_metadata = SessionMetadata::new(
            recording_session_id.clone(),
            SessionMode::Dictation,
            SessionOrigin::LiveCapture,
            trigger_mode,
            std::time::SystemTime::now(),
            None,
            None,
            None,
            Vec::new(),
            None,
        )
        .map_err(|message| LiveStartFailure::new(session, message))?;
        let recording_reservation = recording_reservation
            .with_session_metadata(session_metadata)
            .map_err(|message| LiveStartFailure::new(session, message))?;
        let recording_handle = RecordingSinkHandle::spawn_reserved(
            recording_reservation,
            recording_sink,
            recording_rx,
        );
        let recording_for_capture = recording_handle.sink();
        if !self.start_intent_is_current(intent)
            || self.active_session.load(Ordering::Acquire) != session
        {
            let _ = recording_handle.abort("live start cancelled before capture opened");
            return Ok(None);
        }
        let capture = match CaptureAdapter::open(
            resolved.device,
            stream_config,
            sample_format,
            move |ports, errors| {
                run_capture_worker(
                    ports,
                    errors,
                    CaptureWorkerContext {
                        runtime: capture_runtime,
                        app: capture_app,
                        session,
                        recording_session_id,
                        active_session: capture_active_session,
                        recording: recording_for_capture,
                        local_asr,
                        level_tx,
                    },
                );
            },
        ) {
            Ok(capture) => capture,
            Err(error) => {
                if let Err(finalize_error) =
                    recording_handle.abort(format!("capture adapter failed to open: {error}"))
                {
                    crate::stt::log_yap(&format!(
                        "live recording abort after capture-open failure failed: {finalize_error}"
                    ));
                }
                return Err(LiveStartFailure::new(session, error));
            }
        };
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if !inner.can_install_capture(session, self.active_session.load(Ordering::SeqCst)) {
            inner.mark_used();
            drop(inner);
            if let Err(error) = capture.shutdown() {
                crate::stt::log_yap(&format!("live capture shutdown failed: {error}"));
            }
            let _ = recording_handle.finalize();
            drop(level);
            return Ok(None);
        }
        inner.install_capture(CaptureInstallation {
            capture,
            recording: recording_handle,
            pending_asr,
            app: app.clone(),
            level,
            session,
            active_session: Arc::clone(&self.active_session),
        });
        drop(inner);

        let state = app.state::<LiveSessionState>();
        let Some(view) = state.try_begin_listening_from_armed() else {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let (shutdown_errors, _) = inner.stop_capture();
            drop(inner);
            super::log_worker_shutdown_errors(shutdown_errors);
            let _ = self.finalize_recording();
            return Ok(None);
        };
        events::emit_session(&app, &view);
        Ok(Some(LocalCaptureStart { session }))
    }

    pub(crate) fn complete_local_start(
        &self,
        app: tauri::AppHandle,
        start: LocalCaptureStart,
        intent: StartIntent,
    ) -> Result<bool, LiveStartFailure> {
        let session = start.session;
        let reused = self.run_start_lifecycle(intent, || {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if !inner
                .capture_session_is_current(session, self.active_session.load(Ordering::Acquire))
            {
                return Ok(false);
            }
            if inner.reuse_stream(session)? {
                inner.start_pending_asr_adapter(session)?;
                return Ok(true);
            }
            Ok(false)
        });
        match reused {
            None => return Ok(false),
            Some(Ok(true)) => return Ok(true),
            Some(Ok(false)) => {}
            Some(Err(message)) => return Err(LiveStartFailure::new(session, message)),
        }

        self.request_model_warmup()
            .map_err(|message| LiveStartFailure::new(session, message))?;
        let Some(model) = self
            .model_warmup
            .wait_cancellable(|| !self.start_intent_is_current(intent))
            .map_err(|message| LiveStartFailure::new(session, message))?
        else {
            return Ok(false);
        };
        let model_warmup = Arc::clone(&self.model_warmup);
        self.run_start_lifecycle(intent, move || {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if !inner
                .capture_session_is_current(session, self.active_session.load(Ordering::Acquire))
            {
                return Ok(false);
            }
            if !inner.reuse_stream(session)? {
                inner.install_stream(
                    app,
                    session,
                    model.commit(),
                    model_warmup,
                    Arc::clone(&self.active_session),
                );
            }
            inner.start_pending_asr_adapter(session)?;
            Ok(true)
        })
        .unwrap_or(Ok(false))
        .map_err(|message| LiveStartFailure::new(session, message))
    }
}
