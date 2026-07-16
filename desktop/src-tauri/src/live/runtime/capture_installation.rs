use std::sync::{atomic::AtomicU64, Arc};

use crate::audio::capture::CaptureAdapter;
use crate::audio::recording::RecordingSinkHandle;

use super::asr_adapter::PendingAsrAdapter;
use super::level_channel::LatestLevelReceiver;

pub(super) struct CaptureInstallation {
    pub(super) capture: CaptureAdapter,
    pub(super) recording: RecordingSinkHandle,
    pub(super) pending_asr: PendingAsrAdapter,
    pub(super) app: tauri::AppHandle,
    pub(super) level: LatestLevelReceiver,
    pub(super) session: u64,
    pub(super) active_session: Arc<AtomicU64>,
}
