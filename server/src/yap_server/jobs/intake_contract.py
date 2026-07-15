from __future__ import annotations

from typing import Mapping

from .chunk_contract import validated_single_track_chunks
from .contract_values import (
    COUNTRY,
    MAX_CHUNK_BYTES,
    MAX_CHUNKS,
    MAX_JOB_PCM_BYTES,
    MAX_PRIVATE_RETENTION,
    MAX_TRACKS,
    exact_keys,
    identifier,
    integer_between,
    language_tag,
    mapping,
    text,
    utc_timestamp,
    valid_sha256,
)


def validate_create_request(
    request: Mapping[str, object],
    supported_languages: frozenset[str],
) -> None:
    exact_keys(
        request,
        {"displayName", "metadata", "tracks", "route", "captureManifest", "chunks"},
        "job",
    )
    display_name = text(request.get("displayName"), "displayName")
    if len(display_name) > 256 or request.get("route") != "server_batch":
        raise ValueError("invalid display name or route")

    metadata = mapping(request.get("metadata"), "metadata")
    exact_keys(
        metadata,
        {
            "sessionId",
            "mode",
            "origin",
            "triggerMode",
            "startedAtUtc",
            "utcOffsetMinutesAtStart",
            "localeHintBcp47",
            "countryCodeHint",
            "preferredLanguagesBcp47",
            "appVersion",
            "platform",
            "privacyPolicyVersion",
            "retentionExpiresAtUtc",
        },
        "metadata",
    )
    session_id = identifier(metadata.get("sessionId"), 128, "sessionId")
    mode = metadata.get("mode")
    if mode != "meeting":
        raise ValueError("the Phase 5 batch slice accepts meeting sessions only")
    origin = metadata.get("origin")
    if origin != "imported_file":
        raise ValueError("the Phase 5 batch slice accepts imported recordings only")
    if metadata.get("triggerMode") not in {"push_to_talk", "toggle"}:
        raise ValueError("invalid trigger mode")
    started_at = utc_timestamp(metadata.get("startedAtUtc"), "startedAtUtc")
    utc_offset = metadata.get("utcOffsetMinutesAtStart")
    if utc_offset is not None and not integer_between(utc_offset, -840, 840):
        raise ValueError("invalid UTC offset")
    locale = metadata.get("localeHintBcp47")
    if locale is not None:
        language_tag(locale, "localeHintBcp47")
    country = metadata.get("countryCodeHint")
    if country is not None and (
        not isinstance(country, str) or COUNTRY.fullmatch(country) is None
    ):
        raise ValueError("invalid country code")
    languages = metadata.get("preferredLanguagesBcp47")
    if not isinstance(languages, list) or not 1 <= len(languages) <= 8:
        raise ValueError("an explicit preferred language is required")
    for index, language in enumerate(languages):
        language_tag(language, f"preferredLanguagesBcp47[{index}]")
    if languages[0].split("-", 1)[0].lower() not in supported_languages:
        raise ValueError("preferred language is unsupported")
    for field, maximum in (
        ("appVersion", 64),
        ("platform", 64),
        ("privacyPolicyVersion", 128),
    ):
        value = text(metadata.get(field), field)
        if len(value) > maximum:
            raise ValueError(f"{field} is too long")
    retention = metadata.get("retentionExpiresAtUtc")
    if retention is None:
        raise ValueError("server batch metadata requires a retention expiry")
    retention_at = utc_timestamp(retention, "retentionExpiresAtUtc")
    if retention_at <= started_at:
        raise ValueError("retention expiry must be after session start")
    if retention_at - started_at > MAX_PRIVATE_RETENTION:
        raise ValueError("retention expiry exceeds the private-storage maximum")

    tracks = request.get("tracks")
    if not isinstance(tracks, list) or not 1 <= len(tracks) <= MAX_TRACKS:
        raise ValueError("invalid track count")
    track_ids: set[str] = set()
    for value in tracks:
        track = mapping(value, "tracks[]")
        exact_keys(
            track,
            {
                "trackId",
                "source",
                "deviceId",
                "originalSampleRateHz",
                "originalChannels",
            },
            "track",
        )
        track_id = identifier(track.get("trackId"), 64, "trackId")
        if track_id in track_ids:
            raise ValueError("track IDs must be unique")
        track_ids.add(track_id)
        _validate_track_source(track.get("source"), origin)
        device_id = track.get("deviceId")
        if device_id is not None and (
            not isinstance(device_id, str) or not 1 <= len(device_id) <= 128
        ):
            raise ValueError("invalid device ID")
        if not integer_between(track.get("originalSampleRateHz"), 1, 2**31 - 1):
            raise ValueError("invalid original sample rate")
        if not integer_between(track.get("originalChannels"), 1, 64):
            raise ValueError("invalid original channel count")

    capture_manifest = mapping(request.get("captureManifest"), "captureManifest")
    exact_keys(
        capture_manifest,
        {"schemaVersion", "sessionId", "sha256", "byteLength"},
        "captureManifest",
    )
    if (
        not integer_between(capture_manifest.get("schemaVersion"), 1, 2**31 - 1)
        or capture_manifest.get("sessionId") != session_id
        or not valid_sha256(capture_manifest.get("sha256"))
        or not integer_between(capture_manifest.get("byteLength"), 1, 2**63 - 1)
    ):
        raise ValueError("invalid capture manifest")

    chunks = request.get("chunks")
    if not isinstance(chunks, list) or not 1 <= len(chunks) <= MAX_CHUNKS:
        raise ValueError("invalid chunk count")
    replay_keys: set[tuple[object, ...]] = set()
    total_bytes = 0
    for value in chunks:
        chunk = mapping(value, "chunks[]")
        exact_keys(
            chunk,
            {
                "replayKey",
                "contentIdentity",
                "audioCodec",
                "sampleRateHz",
                "channels",
                "startMs",
                "durationMs",
            },
            "chunk",
        )
        replay = mapping(chunk.get("replayKey"), "replayKey")
        exact_keys(
            replay,
            {"schemaVersion", "sessionId", "trackId", "sequenceStart", "sequenceEnd"},
            "replayKey",
        )
        track_id = identifier(replay.get("trackId"), 64, "replayKey.trackId")
        sequence_start = replay.get("sequenceStart")
        sequence_end = replay.get("sequenceEnd")
        if (
            not integer_between(replay.get("schemaVersion"), 1, 2**31 - 1)
            or replay.get("sessionId") != session_id
            or track_id not in track_ids
            or not integer_between(sequence_start, 0, 2**63 - 1)
            or not integer_between(sequence_end, sequence_start, 2**63 - 1)
        ):
            raise ValueError("invalid replay identity")
        replay_identity = (
            replay["schemaVersion"],
            replay["sessionId"],
            track_id,
            sequence_start,
            sequence_end,
        )
        if replay_identity in replay_keys:
            raise ValueError("replay keys must be unique")
        replay_keys.add(replay_identity)
        content = mapping(chunk.get("contentIdentity"), "contentIdentity")
        exact_keys(content, {"sha256", "byteLength"}, "contentIdentity")
        byte_length = content.get("byteLength")
        if (
            not valid_sha256(content.get("sha256"))
            or not integer_between(byte_length, 2, MAX_CHUNK_BYTES)
            or byte_length % 2 != 0
            or chunk.get("audioCodec") != "pcm_s16le"
            or chunk.get("sampleRateHz") != 16000
            or chunk.get("channels") != 1
            or not integer_between(chunk.get("startMs"), 0, 2**63 - 1)
            or not integer_between(chunk.get("durationMs"), 1, 2**31 - 1)
        ):
            raise ValueError("invalid chunk declaration")
        if byte_length * 1000 != chunk["durationMs"] * 16000 * 2:
            raise ValueError("chunk duration and byte length differ")
        if sequence_end - sequence_start + 1 != byte_length // 2:
            raise ValueError("chunk sequence range and PCM frame count differ")
        total_bytes += byte_length
        if total_bytes > MAX_JOB_PCM_BYTES:
            raise ValueError("job audio exceeds the WAV size limit")

    if len(track_ids) != 1:
        raise ValueError("the current batch slice accepts exactly one track")
    validated_single_track_chunks(chunks)


def _validate_track_source(value: object, origin: object) -> None:
    source = mapping(value, "track.source")
    kind = source.get("kind")
    if origin == "live_capture":
        exact_keys(source, {"kind", "source"}, "captured source")
        if kind != "captured" or source.get("source") not in {
            "microphone",
            "system_loopback",
        }:
            raise ValueError("live capture requires a captured source")
        return
    exact_keys(source, {"kind", "provenance"}, "imported source")
    if kind != "imported":
        raise ValueError("imported sessions require imported sources")
    provenance = source.get("provenance")
    if isinstance(provenance, str) and provenance in {"unknown", "mixed"}:
        return
    declared = mapping(provenance, "imported provenance")
    exact_keys(declared, {"kind", "source"}, "imported provenance")
    if declared.get("kind") != "user_declared" or declared.get("source") not in {
        "microphone",
        "system_loopback",
    }:
        raise ValueError("invalid imported source provenance")


def selected_language(
    request: Mapping[str, object],
    supported_languages: frozenset[str],
) -> str:
    metadata = mapping(request.get("metadata"), "metadata")
    languages = metadata.get("preferredLanguagesBcp47")
    if not isinstance(languages, list) or not languages:
        raise ValueError("server batch processing requires an explicit language")
    selected = text(languages[0], "preferredLanguagesBcp47[0]")
    if selected.split("-", 1)[0].lower() not in supported_languages:
        raise ValueError("selected language is not supported by the locked model")
    return selected
