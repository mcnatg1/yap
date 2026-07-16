use std::sync::{atomic::AtomicU64, Arc};
use std::time::{Duration, Instant};

use crate::audio::capture::CaptureAdapter;
use crate::audio::recording::RecordingSinkHandle;

use super::super::stream::LiveStreamEngine;
use super::asr_adapter::{
    AdapterDrainStatus, PendingAsrAdapter, SessionAsrAdapter, ASR_ADAPTER_DRAIN_TIMEOUT,
};
use super::capture_installation::CaptureInstallation;
use super::level_channel::LevelWorker;
use super::stream_session::{SessionStream, StreamFinishStatus, StreamFinisher};
use super::warmup::SharedWarmup;

pub(super) struct LiveRuntimeResources {
    session: u64,
    capture: Option<CaptureAdapter>,
    stream: Option<SessionStream>,
    retiring_stream: Option<SessionStream>,
    pending_asr: Option<PendingAsrAdapter>,
    asr_adapter: Option<SessionAsrAdapter>,
    recording: Option<RecordingSinkHandle>,
    level: LevelWorker,
    last_used: Instant,
    #[cfg(test)]
    has_capture_for_test: bool,
    #[cfg(test)]
    has_stream_for_test: bool,
}

impl LiveRuntimeResources {
    pub(super) fn new() -> Self {
        Self {
            session: 0,
            capture: None,
            stream: None,
            retiring_stream: None,
            pending_asr: None,
            asr_adapter: None,
            recording: None,
            level: LevelWorker::new(),
            last_used: Instant::now(),
            #[cfg(test)]
            has_capture_for_test: false,
            #[cfg(test)]
            has_stream_for_test: false,
        }
    }

    pub(super) fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    pub(super) fn begin_capture_session(&mut self) -> Option<u64> {
        if self.capture.is_some() {
            return None;
        }
        self.session = self.session.saturating_add(1);
        self.mark_used();
        Some(self.session)
    }

    pub(super) fn capture_session_is_current(&self, session: u64, active_session: u64) -> bool {
        session != 0
            && self.session == session
            && self.capture.is_some()
            && active_session == session
    }

    pub(super) fn can_install_capture(&self, session: u64, active_session: u64) -> bool {
        capture_install_is_current(
            session,
            self.session,
            active_session,
            self.capture.is_some(),
        )
    }

    pub(super) fn install_capture(&mut self, installation: CaptureInstallation) {
        let CaptureInstallation {
            capture,
            recording,
            pending_asr,
            app,
            level,
            session,
            active_session,
        } = installation;
        self.capture = Some(capture);
        self.recording = Some(recording);
        self.pending_asr = Some(pending_asr);
        self.level.start(app, level, session, active_session);
    }

    pub(super) fn has_running_stream(&self) -> bool {
        self.stream.as_ref().is_some_and(SessionStream::is_running)
    }

    pub(super) fn reuse_stream(&mut self, session: u64) -> Result<bool, String> {
        self.reap_retiring_stream()?;
        if let Some(stream) = self.stream.as_ref().filter(|stream| stream.is_running()) {
            stream.retarget(session);
            return Ok(true);
        }
        self.retire_stream();
        Ok(false)
    }

    pub(super) fn install_stream(
        &mut self,
        app: tauri::AppHandle,
        session: u64,
        engine: LiveStreamEngine,
        model_warmup: Arc<SharedWarmup<LiveStreamEngine>>,
        active_session: Arc<AtomicU64>,
    ) {
        self.stream = Some(SessionStream::start(
            engine,
            session,
            active_session,
            app,
            model_warmup,
        ));
    }

    pub(super) fn start_pending_asr_adapter(&mut self, session: u64) -> Result<(), String> {
        self.cancel_asr_adapter()?;
        let samples_tx = self
            .stream
            .as_ref()
            .map(SessionStream::sender)
            .ok_or_else(|| "Live stream is unavailable.".to_string())?;
        let pending = self
            .pending_asr
            .take()
            .ok_or_else(|| "Live pre-roll is unavailable.".to_string())?;
        self.asr_adapter = Some(pending.start(samples_tx, session));
        Ok(())
    }

    pub(super) fn stop_capture(&mut self) -> (Vec<String>, Option<StreamFinishStatus>) {
        let mut errors = Vec::new();
        let mut adapter_status = None;
        if let Some(capture) = self.capture.take() {
            if let Err(error) = capture.shutdown() {
                errors.push(error);
            }
        }
        self.pending_asr.take();
        if let Some(mut adapter) = self.asr_adapter.take() {
            match adapter.drain_after_capture(ASR_ADAPTER_DRAIN_TIMEOUT) {
                Ok(AdapterDrainStatus::Drained) => {}
                Ok(AdapterDrainStatus::TimedOut | AdapterDrainStatus::TimedOutRetained) => {
                    adapter_status = Some(StreamFinishStatus::TimedOut);
                    if let Some(error) = adapter.take_cleanup_error() {
                        errors.push(error);
                    }
                    if adapter.retains_cleanup_ownership() {
                        self.asr_adapter = Some(adapter);
                    }
                }
                Err(error) => {
                    errors.push(error);
                    adapter_status = Some(StreamFinishStatus::Disconnected);
                    if adapter.retains_cleanup_ownership() {
                        self.asr_adapter = Some(adapter);
                    }
                }
            }
        }
        if let Err(error) = self.level.shutdown() {
            errors.push(error);
        }
        #[cfg(test)]
        {
            self.has_capture_for_test = false;
        }
        (errors, adapter_status)
    }

    pub(super) fn retire_stream(&mut self) {
        if let Err(error) = self.cancel_asr_adapter() {
            crate::stt::log_yap(&format!("live ASR adapter shutdown failed: {error}"));
        }
        if let Some(stream) = self.stream.take() {
            if let Err(error) = stream.shutdown(true) {
                crate::stt::log_yap(&format!("live stream worker shutdown failed: {error}"));
            }
        }
        let retiring_finished = self
            .retiring_stream
            .as_ref()
            .is_some_and(SessionStream::is_finished);
        if retiring_finished {
            let stream = self
                .retiring_stream
                .take()
                .expect("finished retiring stream was present");
            if let Err(error) = stream.shutdown(true) {
                crate::stt::log_yap(&format!("retiring live stream shutdown failed: {error}"));
            }
        }
        #[cfg(test)]
        {
            self.has_stream_for_test = false;
        }
    }

    pub(super) fn retire_stream_detached_reader(&mut self) {
        if let Err(error) = self.cancel_asr_adapter() {
            crate::stt::log_yap(&format!("live ASR adapter shutdown failed: {error}"));
        }
        if let Some(stream) = self.stream.take() {
            stream.cancel_reader();
            if self.retiring_stream.is_none() {
                self.retiring_stream = Some(stream);
            } else if let Err(error) = stream.shutdown(true) {
                crate::stt::log_yap(&format!("extra retiring stream shutdown failed: {error}"));
            }
        }
        #[cfg(test)]
        {
            self.has_stream_for_test = false;
        }
    }

    pub(super) fn stream_finisher(&self) -> Option<StreamFinisher> {
        self.stream.as_ref().map(SessionStream::finisher)
    }

    pub(super) fn recording_is_present(&self) -> bool {
        self.recording.is_some()
    }

    pub(super) fn take_recording(&mut self) -> Option<RecordingSinkHandle> {
        self.recording.take()
    }

    pub(super) fn is_idle_for(&self, threshold: Duration) -> bool {
        self.capture.is_none() && self.last_used.elapsed() >= threshold
    }

    pub(super) fn mark_used(&mut self) {
        self.last_used = Instant::now();
    }

    fn reap_retiring_stream(&mut self) -> Result<(), String> {
        let Some(stream) = self.retiring_stream.as_ref() else {
            return Ok(());
        };
        if !stream.is_finished() {
            return Err("Previous live transcription is still stopping.".into());
        }
        let stream = self
            .retiring_stream
            .take()
            .expect("retiring stream was present");
        stream.shutdown(true)
    }

    fn cancel_asr_adapter(&mut self) -> Result<(), String> {
        if let Some(mut adapter) = self.asr_adapter.take() {
            if let Err(error) = adapter.cancel_and_join() {
                self.asr_adapter = Some(adapter);
                return Err(error);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn for_test() -> Self {
        Self::new()
    }

    #[cfg(test)]
    pub(super) fn mark_resources_present_for_test(&mut self) {
        self.has_capture_for_test = true;
        self.has_stream_for_test = true;
    }

    #[cfg(test)]
    pub(super) fn resources_present_for_test(&self) -> (bool, bool) {
        (self.has_capture_for_test, self.has_stream_for_test)
    }

    #[cfg(test)]
    pub(super) fn mark_stream_crashed_for_test(&mut self) {
        self.has_capture_for_test = false;
        self.has_stream_for_test = false;
    }

    #[cfg(test)]
    pub(super) fn set_session_for_test(&mut self, session: u64) {
        self.session = session;
    }

    #[cfg(test)]
    pub(super) fn set_recording_for_test(&mut self, recording: RecordingSinkHandle) {
        self.recording = Some(recording);
    }

    #[cfg(test)]
    pub(super) fn set_stream_for_test(&mut self, stream: SessionStream) {
        self.stream = Some(stream);
    }

    #[cfg(test)]
    pub(super) fn set_retiring_stream_for_test(&mut self, stream: SessionStream) {
        self.retiring_stream = Some(stream);
    }

    #[cfg(test)]
    pub(super) fn has_retiring_stream_for_test(&self) -> bool {
        self.retiring_stream.is_some()
    }

    #[cfg(test)]
    pub(super) fn retiring_stream_is_finished_for_test(&self) -> bool {
        self.retiring_stream
            .as_ref()
            .is_none_or(SessionStream::is_finished)
    }

    #[cfg(test)]
    pub(super) fn reap_retiring_stream_for_test(&mut self) -> Result<(), String> {
        self.reap_retiring_stream()
    }
}

pub(super) fn capture_install_is_current(
    requested_session: u64,
    resource_session: u64,
    active_session: u64,
    has_capture: bool,
) -> bool {
    requested_session != 0
        && requested_session == resource_session
        && requested_session == active_session
        && !has_capture
}
