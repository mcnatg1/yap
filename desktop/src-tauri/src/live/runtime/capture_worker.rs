use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

use tauri::Manager;

use crate::audio::capture::{CapturePacket, CapturePorts};
use crate::audio::coordinator::{BoundedSink, Coordinator, CoordinatorPorts, SinkKind};
use crate::audio::frame::PreparedFrame;
use crate::audio::session::{SessionId, TrackId};
use crate::audio::timeline::RecordingInput;

use super::super::{events, state::LiveSessionState};
use super::level_channel::{publish_level, LatestLevelSender};
use super::session_identity::active_session_matches;
use super::{spawn_stream_crash_handler, LiveRuntime};

const CAPTURE_LOSS_FINAL_DRAIN_ATTEMPTS: usize = 64;
pub(super) const CAPTURE_WORKER_FAILURE: &str = "Live capture worker stopped unexpectedly.";

pub(super) struct CaptureWorkerContext {
    pub(super) runtime: LiveRuntime,
    pub(super) app: tauri::AppHandle,
    pub(super) session: u64,
    pub(super) recording_session_id: SessionId,
    pub(super) active_session: Arc<AtomicU64>,
    pub(super) recording: BoundedSink<RecordingInput>,
    pub(super) local_asr: BoundedSink<PreparedFrame>,
    pub(super) level_tx: LatestLevelSender,
}

pub(super) fn run_capture_worker(
    ports: CapturePorts,
    errors: mpsc::Receiver<cpal::StreamError>,
    context: CaptureWorkerContext,
) {
    let CaptureWorkerContext {
        runtime,
        app,
        session,
        recording_session_id,
        active_session,
        recording,
        local_asr,
        level_tx,
    } = context;
    let packet_runtime = runtime.clone();
    let packet_app = app.clone();
    let error_runtime = runtime.clone();
    let error_app = app.clone();
    let loss_runtime = runtime.clone();
    let loss_app = app.clone();
    let recording_guard = recording.clone();
    let coordinator = Arc::new(Mutex::new(capture_worker_coordinator(
        recording_session_id,
        recording,
        local_asr,
    )));
    let transcription_degraded = Arc::new(AtomicBool::new(false));
    let packet_coordinator = Arc::clone(&coordinator);
    let packet_degraded = Arc::clone(&transcription_degraded);
    let loss_coordinator = Arc::clone(&coordinator);
    run_guarded_capture_packet_worker(
        &recording_guard,
        || {
            run_capture_packet_loop(
                ports,
                errors,
                move |packet, losses| {
                    if !active_session_matches(active_session.load(Ordering::SeqCst), session) {
                        return false;
                    }
                    let mut coordinator = match packet_coordinator.lock() {
                        Ok(coordinator) => coordinator,
                        Err(_) => return true,
                    };
                    match coordinator.consume(packet, losses) {
                        Ok(level) => {
                            if coordinator
                                .outcome(SinkKind::LocalAsr)
                                .is_some_and(|outcome| outcome.dropped_frames > 0)
                                && mark_local_asr_degraded_once(&packet_degraded)
                            {
                                let state = packet_app.state::<LiveSessionState>();
                                let view = state.mark_transcription_backpressure();
                                events::emit_session(&packet_app, &view);
                            }
                            publish_level(&level_tx, level);
                            false
                        }
                        Err(message) => {
                            spawn_stream_crash_handler(
                                packet_app.clone(),
                                packet_runtime.clone(),
                                session,
                                message,
                            );
                            true
                        }
                    }
                },
                move |error| {
                    let message = format!("Microphone input stopped: {error}");
                    crate::stt::log_yap(&format!("live input stream error: {error}"));
                    spawn_stream_crash_handler(
                        error_app.clone(),
                        error_runtime.clone(),
                        session,
                        message,
                    );
                    true
                },
                move |loss| {
                    process_capture_loss(&loss_coordinator, loss, |message| {
                        spawn_stream_crash_handler(
                            loss_app.clone(),
                            loss_runtime.clone(),
                            session,
                            message,
                        );
                    })
                },
            );
        },
        move |message| spawn_stream_crash_handler(app, runtime, session, message),
    );
    if let Ok(mut coordinator) = coordinator.lock() {
        for outcome in coordinator.outcomes() {
            crate::stt::log_yap(&format!(
                "audio sink {:?} accepted={} dropped={} closed={} error={:?}",
                outcome.kind,
                outcome.accepted_frames,
                outcome.dropped_frames,
                outcome.closed,
                outcome.error
            ));
        }
        coordinator.close();
    };
}

pub(super) fn capture_worker_coordinator(
    recording_session_id: SessionId,
    recording: BoundedSink<RecordingInput>,
    local_asr: BoundedSink<PreparedFrame>,
) -> Coordinator {
    Coordinator::new(
        recording_session_id,
        TrackId::new("live-microphone").expect("static live track ID is valid"),
        CoordinatorPorts {
            recording,
            local_asr: Some(local_asr),
            speaker_evidence: None,
            server_transport: None,
        },
    )
}

pub(super) fn run_guarded_capture_packet_worker<R, C>(
    recording: &BoundedSink<RecordingInput>,
    run: R,
    process_crash: C,
) where
    R: FnOnce(),
    C: FnOnce(String),
{
    if catch_unwind(AssertUnwindSafe(run)).is_err() {
        recording.degrade(CAPTURE_WORKER_FAILURE);
        process_crash(CAPTURE_WORKER_FAILURE.to_string());
    }
}

pub(super) fn run_capture_packet_loop<P, E, L>(
    ports: CapturePorts,
    errors: mpsc::Receiver<cpal::StreamError>,
    process_packet: P,
    process_error: E,
    process_loss: L,
) where
    P: FnMut(&CapturePacket, &crate::audio::timeline::LossAccumulator) -> bool,
    E: FnMut(cpal::StreamError) -> bool,
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    run_capture_packet_loop_with_timeout(
        ports,
        errors,
        Duration::from_millis(50),
        process_packet,
        process_error,
        process_loss,
    );
}

pub(super) fn run_capture_packet_loop_with_timeout<P, E, L>(
    ports: CapturePorts,
    errors: mpsc::Receiver<cpal::StreamError>,
    receive_timeout: Duration,
    mut process_packet: P,
    mut process_error: E,
    mut process_loss: L,
) where
    P: FnMut(&CapturePacket, &crate::audio::timeline::LossAccumulator) -> bool,
    E: FnMut(cpal::StreamError) -> bool,
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    let CapturePorts {
        packets,
        returned_buffers,
        losses,
    } = ports;
    loop {
        if drain_capture_losses(&losses, &mut process_loss) == CaptureLossDrainStep::Stop {
            break;
        }
        let mut should_exit = false;
        while let Ok(error) = errors.try_recv() {
            if process_error(error) {
                should_exit = true;
                break;
            }
        }
        if should_exit {
            break;
        }
        match packets.recv_timeout(receive_timeout) {
            Ok(mut packet) => {
                let should_exit = process_packet(&packet, &losses);
                packet.samples.clear();
                let _ = returned_buffers.try_send(packet.samples);
                if should_exit {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drain_capture_losses_on_shutdown(&losses, &mut process_loss);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureLossDrainStep {
    Drained,
    Pending,
    Progressed,
    Stop,
}

fn drain_capture_losses<L>(
    losses: &crate::audio::timeline::LossAccumulator,
    process_loss: &mut L,
) -> CaptureLossDrainStep
where
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    match losses.try_drain() {
        Ok(crate::audio::timeline::TryDrain::Snapshot(snapshot)) => {
            if process_loss(Ok(snapshot)) {
                CaptureLossDrainStep::Stop
            } else {
                CaptureLossDrainStep::Progressed
            }
        }
        Ok(crate::audio::timeline::TryDrain::Pending) => CaptureLossDrainStep::Pending,
        Ok(crate::audio::timeline::TryDrain::Empty) => CaptureLossDrainStep::Drained,
        Err(error) => {
            if process_loss(Err(error)) {
                CaptureLossDrainStep::Stop
            } else {
                CaptureLossDrainStep::Progressed
            }
        }
    }
}

fn drain_capture_losses_on_shutdown<L>(
    losses: &crate::audio::timeline::LossAccumulator,
    process_loss: &mut L,
) where
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    for _ in 0..CAPTURE_LOSS_FINAL_DRAIN_ATTEMPTS {
        match drain_capture_losses(losses, process_loss) {
            CaptureLossDrainStep::Drained | CaptureLossDrainStep::Stop => return,
            CaptureLossDrainStep::Progressed => {}
            CaptureLossDrainStep::Pending => std::thread::yield_now(),
        }
    }
    let _ = process_loss(Err(crate::audio::timeline::TimelineError::DrainIncomplete));
}

pub(super) fn process_capture_loss<F>(
    coordinator: &Arc<Mutex<Coordinator>>,
    loss: Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    mut process_failure: F,
) -> bool
where
    F: FnMut(String),
{
    match loss {
        Ok(snapshot) => match coordinator.lock() {
            Ok(mut coordinator) => coordinator.consume_loss(snapshot).is_err(),
            Err(_) => true,
        },
        Err(error) => {
            let recording_error = format!("Capture loss timing failed: {error}");
            coordinator
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .degrade_recording(&recording_error);
            process_failure("Microphone capture timing became invalid.".to_string());
            true
        }
    }
}

pub(super) fn mark_local_asr_degraded_once(reported: &AtomicBool) -> bool {
    reported
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}
