use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::server_connector::batch::{
    ApiError, CreateRecordingJobRequest, RecordingJob, TranscriptResultRevision,
};

pub(super) fn validate_job_projection(
    projection: &RecordingJob,
    request: &CreateRecordingJobRequest,
    expected_job_id: Option<&str>,
    allowed_statuses: &[&str],
) -> Result<(), String> {
    let manifest = &projection.capture_manifest;
    let error_is_valid = match (projection.status.as_str(), projection.error.as_ref()) {
        ("failed", Some(error)) => valid_server_job_error(error),
        ("failed", None) => false,
        (_, None) => true,
        (_, Some(_)) => false,
    };
    if expected_job_id.is_some_and(|expected| projection.job_id != expected)
        || projection.job_id.is_empty()
        || projection.session_id != request.metadata.session_id.as_str()
        || projection.display_name != request.display_name
        || projection.session_mode != "meeting"
        || projection.session_origin != "imported_file"
        || projection.route.as_deref() != Some("server_batch")
        || manifest.schema_version != request.capture_manifest.schema_version
        || manifest.session_id != request.capture_manifest.session_id
        || manifest.sha256 != request.capture_manifest.sha256
        || manifest.byte_length != request.capture_manifest.byte_length
        || !allowed_statuses.contains(&projection.status.as_str())
        || !error_is_valid
        || projection.created_at_utc.is_empty()
        || projection.updated_at_utc.is_empty()
    {
        return Err("server job projection conflicts with the prepared recording".into());
    }
    Ok(())
}

fn valid_server_job_error(error: &ApiError) -> bool {
    error.is_valid()
}

pub(super) fn validate_result_revision(
    result: &TranscriptResultRevision,
    request: &CreateRecordingJobRequest,
) -> Result<(), String> {
    let expected_language = request
        .metadata
        .preferred_languages_bcp47
        .first()
        .ok_or_else(|| "prepared recording has no preferred result language".to_string())?;
    let language = result
        .language
        .as_ref()
        .ok_or_else(|| "server result omitted its language decision".to_string())?;
    let timestamp_valid = result.created_at_utc.ends_with('Z')
        && result.created_at_utc.len() <= 64
        && OffsetDateTime::parse(&result.created_at_utc, &Rfc3339).is_ok();
    let language_valid = language.language_bcp47 == *expected_language
        && language.language_bcp47.len() <= 35
        && language
            .language_bcp47
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && language
            .confidence
            .is_none_or(|confidence| (0.0..=1.0).contains(&confidence));
    let provenance_valid = !result.model_provenance.is_empty()
        && result.model_provenance.len() <= 8
        && result.model_provenance.iter().all(|model| {
            [
                model.model_id.as_str(),
                model.revision.as_str(),
                model.calibration_revision.as_str(),
            ]
            .iter()
            .all(|value| !value.is_empty() && value.len() <= 256)
        });
    if result.session_id != request.metadata.session_id.as_str()
        || result.revision != 1
        || result.authority != "server_authoritative"
        || !timestamp_valid
        || result.capture_manifest_sha256 != request.capture_manifest.sha256
        || result.previous_result_sha256.is_some()
        || result.status != "complete"
        || !language_valid
        || result.transcript.trim().is_empty()
        || result.transcript.len() > 2 * 1024 * 1024 - 1
        || !result.aligned_words.is_empty()
        || !provenance_valid
    {
        return Err("server result revision conflicts with the prepared recording".into());
    }
    Ok(())
}

pub(super) fn result_retention_expiry_ms(
    request: &CreateRecordingJobRequest,
) -> Result<u64, String> {
    let encoded = request
        .metadata
        .retention_expires_at_utc
        .as_deref()
        .filter(|value| value.ends_with('Z'))
        .ok_or_else(|| "prepared meeting job has no UTC result retention expiry".to_string())?;
    let parsed = OffsetDateTime::parse(encoded, &Rfc3339)
        .map_err(|_| "prepared meeting job has an invalid result retention expiry".to_string())?;
    let milliseconds = parsed.unix_timestamp_nanos().div_euclid(1_000_000);
    u64::try_from(milliseconds)
        .map_err(|_| "prepared meeting result retention expiry is out of range".to_string())
}
