from __future__ import annotations

from typing import Mapping

from .contract_values import (
    JOB_STATUSES,
    MAX_MODEL_PROVENANCE_CHARS,
    MAX_TRANSCRIPT_BYTES,
    PERSISTED_ERROR_CODES,
    exact_keys,
    identifier,
    language_tag,
    mapping,
    text,
    utc_timestamp,
)


def validate_persisted_projection(
    job_id: str,
    creation: Mapping[str, object],
    projection: Mapping[str, object],
) -> None:
    metadata = mapping(creation.get("metadata"), "metadata")
    status = text(projection.get("status"), "persisted projection.status")
    if status not in JOB_STATUSES:
        raise ValueError("persisted job status is invalid")

    keys = {
        "jobId",
        "sessionId",
        "displayName",
        "sessionMode",
        "sessionOrigin",
        "status",
        "route",
        "captureManifest",
        "createdAtUtc",
        "updatedAtUtc",
    }
    if status == "failed":
        keys.add("error")
    exact_keys(projection, keys, "persisted projection")

    if (
        projection.get("jobId") != job_id
        or projection.get("sessionId") != metadata.get("sessionId")
        or projection.get("displayName") != creation.get("displayName")
        or projection.get("sessionMode") != metadata.get("mode")
        or projection.get("sessionOrigin") != metadata.get("origin")
        or projection.get("route") != creation.get("route")
        or projection.get("captureManifest") != creation.get("captureManifest")
    ):
        raise ValueError("persisted job projection differs from creation")

    created_at = utc_timestamp(
        projection.get("createdAtUtc"),
        "persisted projection.createdAtUtc",
    )
    updated_at = utc_timestamp(
        projection.get("updatedAtUtc"),
        "persisted projection.updatedAtUtc",
    )
    if updated_at < created_at:
        raise ValueError("persisted job projection timestamps are invalid")

    if status != "failed":
        return
    error = mapping(projection.get("error"), "persisted projection.error")
    exact_keys(
        error,
        {"code", "message", "retryable", "requestId"},
        "persisted projection.error",
    )
    code = identifier(error.get("code"), 64, "persisted projection.error.code")
    message = text(error.get("message"), "persisted projection.error.message")
    if (
        code not in PERSISTED_ERROR_CODES
        or len(message) > 512
        or error.get("retryable") is not True
        or error.get("requestId") != f"job-{job_id}"
    ):
        raise ValueError("persisted job error is invalid")


def validate_result_revision(
    result: Mapping[str, object],
    projection: Mapping[str, object],
) -> None:
    exact_keys(
        result,
        {
            "sessionId",
            "revision",
            "authority",
            "createdAtUtc",
            "captureManifestSha256",
            "previousResultSha256",
            "status",
            "language",
            "transcript",
            "alignedWords",
            "modelProvenance",
        },
        "result revision",
    )
    capture_manifest = mapping(projection.get("captureManifest"), "captureManifest")
    transcript = text(result.get("transcript"), "result transcript")
    if (
        result.get("sessionId") != projection.get("sessionId")
        or result.get("revision") != 1
        or result.get("authority") != "server_authoritative"
        or result.get("captureManifestSha256") != capture_manifest.get("sha256")
        or result.get("previousResultSha256") is not None
        or result.get("status") not in {"complete", "partial"}
        or not transcript.strip()
        or len(transcript.encode("utf-8")) > MAX_TRANSCRIPT_BYTES
        or result.get("alignedWords") != []
    ):
        raise ValueError("result revision identity or content is invalid")
    utc_timestamp(result.get("createdAtUtc"), "result createdAtUtc")

    language = mapping(result.get("language"), "result language")
    exact_keys(language, {"languageBcp47", "confidence"}, "result language")
    language_tag(language.get("languageBcp47"), "result languageBcp47")
    confidence = language.get("confidence")
    if confidence is not None and (
        isinstance(confidence, bool)
        or not isinstance(confidence, (int, float))
        or not 0 <= confidence <= 1
    ):
        raise ValueError("result language confidence is invalid")

    provenance = result.get("modelProvenance")
    if not isinstance(provenance, list) or len(provenance) != 1:
        raise ValueError("result model provenance is invalid")
    model = mapping(provenance[0], "result model provenance")
    exact_keys(
        model,
        {"modelId", "revision", "calibrationRevision"},
        "result model provenance",
    )
    for field in ("modelId", "revision", "calibrationRevision"):
        if len(text(model.get(field), f"result {field}")) > MAX_MODEL_PROVENANCE_CHARS:
            raise ValueError("result model provenance is oversized")
