use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::recording;
use crate::audio::session::{SessionMode, SessionOrigin};

pub(super) fn committed_meeting_is_expired(
    manifest: &recording::CaptureCommitManifest,
    now: OffsetDateTime,
) -> bool {
    let Some(metadata) = &manifest.session_metadata else {
        return false;
    };
    if metadata.session_id != manifest.session_id
        || metadata.mode != SessionMode::Meeting
        || metadata.origin != SessionOrigin::LiveCapture
    {
        return false;
    }
    let Some(expiry) = metadata.retention_expires_at_utc.as_deref() else {
        return false;
    };
    OffsetDateTime::parse(expiry, &Rfc3339)
        .map(|expiry| expiry <= now)
        .unwrap_or(false)
}
