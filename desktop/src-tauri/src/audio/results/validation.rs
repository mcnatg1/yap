use crate::audio::evidence::{AlignedWord, SpeakerTurn};

use super::ResultAuthority;

pub(super) fn validate_root_revision(
    revision: u64,
    capture_sidecar_sha256: &str,
    previous_result_sha256: Option<&str>,
) -> Result<(), ResultRevisionError> {
    validate_sha256(
        capture_sidecar_sha256,
        ResultRevisionError::InvalidCaptureHash,
    )?;
    if revision != 1 {
        return Err(ResultRevisionError::NonMonotonicRevision);
    }
    if previous_result_sha256.is_some() {
        return Err(ResultRevisionError::UnexpectedPreviousResultHash);
    }
    Ok(())
}

pub(super) fn validate_next_revision(
    previous_revision: u64,
    revision: u64,
    previous_capture_sidecar_sha256: &str,
    capture_sidecar_sha256: &str,
    previous_result_sha256: &str,
) -> Result<(), ResultRevisionError> {
    if revision
        != previous_revision
            .checked_add(1)
            .ok_or(ResultRevisionError::RevisionOverflow)?
    {
        return Err(ResultRevisionError::NonMonotonicRevision);
    }
    if capture_sidecar_sha256 != previous_capture_sidecar_sha256 {
        return Err(ResultRevisionError::CaptureHashChanged);
    }
    validate_sha256(
        capture_sidecar_sha256,
        ResultRevisionError::InvalidCaptureHash,
    )?;
    validate_sha256(
        previous_result_sha256,
        ResultRevisionError::InvalidPreviousResultHash,
    )
}

pub(super) fn validate_named_attribution_authority(
    authority: ResultAuthority,
    speaker_turns: &[SpeakerTurn],
    aligned_words: &[AlignedWord],
) -> Result<(), ResultRevisionError> {
    let contains_named = speaker_turns.iter().any(SpeakerTurn::has_named_attribution)
        || aligned_words.iter().any(AlignedWord::has_named_attribution);
    if contains_named && authority != ResultAuthority::ServerAuthoritative {
        return Err(ResultRevisionError::NamedAttributionRequiresServerAuthority);
    }
    Ok(())
}

pub(super) fn validate_wire_revision(
    revision: u64,
    capture_sidecar_sha256: &str,
    previous_result_sha256: Option<&str>,
) -> Result<(), ResultRevisionError> {
    validate_sha256(
        capture_sidecar_sha256,
        ResultRevisionError::InvalidCaptureHash,
    )?;
    match (revision, previous_result_sha256) {
        (1, None) => Ok(()),
        (1, Some(_)) => Err(ResultRevisionError::UnexpectedPreviousResultHash),
        (0, _) => Err(ResultRevisionError::NonMonotonicRevision),
        (_, Some(hash)) => validate_sha256(hash, ResultRevisionError::InvalidPreviousResultHash),
        (_, None) => Err(ResultRevisionError::MissingPreviousResultHash),
    }
}

fn validate_sha256(value: &str, error: ResultRevisionError) -> Result<(), ResultRevisionError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultRevisionError {
    InvalidCaptureHash,
    MissingPreviousResultHash,
    InvalidPreviousResultHash,
    UnexpectedPreviousResultHash,
    NonMonotonicRevision,
    RevisionOverflow,
    CaptureHashChanged,
    NamedAttributionRequiresServerAuthority,
}

impl std::fmt::Display for ResultRevisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ResultRevisionError {}
