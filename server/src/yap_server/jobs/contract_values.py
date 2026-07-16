from __future__ import annotations

from datetime import datetime, timedelta, timezone
import re
from typing import Mapping


MAX_CHUNKS = 4096
MAX_TRACKS = 8
MAX_CHUNK_BYTES = 1024 * 1024
MAX_JOB_PCM_BYTES = 16000 * 2 * 4 * 60 * 60
MAX_TRANSCRIPT_BYTES = 1024 * 1024
MAX_MODEL_PROVENANCE_CHARS = 256
MAX_PRIVATE_RETENTION = timedelta(days=30)
MAX_CLIENT_CLOCK_SKEW = timedelta(minutes=5)
TERMINAL_STATUSES = frozenset({"complete", "partial", "failed", "cancelled"})
JOB_STATUSES = frozenset(
    {
        "accepted",
        "uploading",
        "server_processing",
        *TERMINAL_STATUSES,
    }
)
PERSISTED_ERROR_CODES = frozenset(
    {
        "ASR_RESULT_INVALID",
        "ASR_RESULT_PUBLISH_FAILED",
        "ASR_CLEANUP_UNVERIFIED",
        "ASR_WORKER_FAILED",
        "SERVER_RESTARTED",
        "SERVER_STORAGE_ERROR",
    }
)

_OPAQUE_ID = re.compile(r"^[A-Za-z0-9_-]+$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_BCP47 = re.compile(r"^[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*$")
COUNTRY = re.compile(r"^[A-Z]{2}$")


def mapping(value: object, field: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise ValueError(f"{field} must be an object")
    return value


def text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


def exact_keys(value: Mapping[str, object], expected: set[str], field: str) -> None:
    if set(value) != expected:
        raise ValueError(f"{field} fields differ from the contract")


def identifier(value: object, maximum: int, field: str) -> str:
    parsed = text(value, field)
    if len(parsed) > maximum or _OPAQUE_ID.fullmatch(parsed) is None:
        raise ValueError(f"{field} is invalid")
    return parsed


def integer_between(value: object, minimum: int, maximum: int) -> bool:
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and minimum <= value <= maximum
    )


def valid_sha256(value: object) -> bool:
    return isinstance(value, str) and _SHA256.fullmatch(value) is not None


def language_tag(value: object, field: str) -> str:
    parsed = text(value, field)
    if len(parsed) > 35 or _BCP47.fullmatch(parsed) is None:
        raise ValueError(f"{field} is invalid")
    return parsed


def utc_timestamp(value: object, field: str) -> datetime:
    parsed_text = text(value, field)
    if not parsed_text.endswith("Z"):
        raise ValueError(f"{field} must be UTC")
    try:
        parsed = datetime.fromisoformat(parsed_text[:-1] + "+00:00")
    except ValueError as error:
        raise ValueError(f"{field} is invalid") from error
    if parsed.tzinfo != timezone.utc:
        raise ValueError(f"{field} must be UTC")
    return parsed
