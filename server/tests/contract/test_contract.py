import json
import unittest
from copy import deepcopy
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


SERVER_ROOT = Path(__file__).resolve().parents[2]
OPENAPI_PATH = SERVER_ROOT / "openapi" / "openapi.json"
LIVE_EVENTS_PATH = SERVER_ROOT / "openapi" / "live-events.schema.json"
EXAMPLES_ROOT = SERVER_ROOT / "openapi" / "examples"

HTTP_OPERATIONS = {
    ("/v1/health", "get"): "getHealth",
    ("/v1/jobs", "post"): "createJob",
    ("/v1/jobs/{jobId}", "get"): "getJob",
    ("/v1/jobs/{jobId}", "delete"): "cancelJob",
    (
        "/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}",
        "put",
    ): "uploadJobChunk",
    ("/v1/jobs/{jobId}/commit", "post"): "commitJob",
    ("/v1/live", "get"): "connectLive",
}

PHASE_BOUNDARY = {
    ("/v1/health", "get"): ("Implemented", "Server process"),
    ("/v1/jobs", "post"): ("Contract only", "Phase 5 upload intake"),
    ("/v1/jobs/{jobId}", "get"): ("Contract only", "Phase 5 job status"),
    ("/v1/jobs/{jobId}", "delete"): ("Contract only", "Phase 5 cancellation"),
    (
        "/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}",
        "put",
    ): ("Contract only", "Phase 5 resumable upload"),
    ("/v1/jobs/{jobId}/commit", "post"): (
        "Contract only",
        "Phase 5 upload commit",
    ),
    ("/v1/live", "get"): ("Event schema only", "Phase 5 WSS streaming"),
}

CHUNK_PATH = "/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}"

HTTP_SCHEMA_CONTRACTS: list[dict[str, Any]] = [
    {
        "path": "/v1/health",
        "method": "get",
        "request": None,
        "success": {"200": "#/components/schemas/HealthView"},
        "errors": ["500"],
    },
    {
        "path": "/v1/jobs",
        "method": "post",
        "request": (
            "application/json",
            "#/components/schemas/CreateRecordingJobRequest",
        ),
        "success": {"202": "#/components/schemas/RecordingJob"},
        "errors": ["400", "429", "501"],
    },
    {
        "path": "/v1/jobs/{jobId}",
        "method": "get",
        "request": None,
        "success": {"200": "#/components/schemas/RecordingJob"},
        "errors": ["404", "501"],
    },
    {
        "path": "/v1/jobs/{jobId}",
        "method": "delete",
        "request": None,
        "success": {"202": "#/components/schemas/RecordingJob"},
        "errors": ["404", "409", "501"],
    },
    {
        "path": CHUNK_PATH,
        "method": "put",
        "request": (
            "application/octet-stream",
            {"type": "string", "format": "binary"},
        ),
        "success": {
            "200": "#/components/schemas/ChunkUploadReceipt",
            "201": "#/components/schemas/ChunkUploadReceipt",
        },
        "errors": ["400", "404", "409", "415", "501"],
    },
    {
        "path": "/v1/jobs/{jobId}/commit",
        "method": "post",
        "request": (
            "application/json",
            "#/components/schemas/CommitRecordingJobRequest",
        ),
        "success": {"202": "#/components/schemas/RecordingJob"},
        "errors": ["400", "404", "409", "501"],
    },
    {
        "path": "/v1/live",
        "method": "get",
        "request": None,
        "success": {"101": None},
        "errors": ["400", "501"],
    },
]

CREATE_JOB_IDENTITY_INVARIANTS = {
    "singleSessionIdentity": {
        "rule": "all_equal",
        "paths": [
            "metadata.sessionId",
            "captureManifest.sessionId",
            "chunks[*].replayKey.sessionId",
        ],
    },
    "chunkTrackMembership": {
        "rule": "member_of",
        "path": "chunks[*].replayKey.trackId",
        "setPath": "tracks[*].trackId",
    },
    "uniqueTrackIds": {"rule": "unique_by", "path": "tracks[*].trackId"},
    "uniqueReplayKeys": {
        "rule": "unique_by",
        "path": "chunks[*].replayKey",
    },
    "manifestImmutability": {
        "rule": "immutable_after_accept",
        "paths": ["captureManifest", "chunks"],
    },
}

RECORDING_JOB_IDENTITY_INVARIANTS = {
    "singleSessionIdentity": {
        "rule": "all_equal",
        "paths": ["sessionId", "captureManifest.sessionId"],
    },
    "manifestContinuity": {
        "rule": "exact_object_equality",
        "path": "captureManifest",
        "sourcePath": "CreateRecordingJobRequest.captureManifest",
    },
}

COMMIT_IDENTITY_INVARIANTS = {
    "singleSessionIdentity": {
        "rule": "all_equal",
        "paths": [
            "job.sessionId",
            "job.captureManifest.sessionId",
            "captureManifest.sessionId",
        ],
    },
    "exactManifestContinuity": {
        "rule": "exact_object_equality",
        "path": "captureManifest",
        "sourcePath": "CreateRecordingJobRequest.captureManifest",
    },
    "chunkCountContinuity": {
        "rule": "equals_unique_count",
        "path": "chunkCount",
        "sourcePath": "CreateRecordingJobRequest.chunks[*].replayKey",
    },
    "mismatchDisposition": "reject_before_processing",
}

CHUNK_IDENTITY_INVARIANTS = {
    "jobManifestContinuity": {
        "rule": "resolved_from_job",
        "jobPath": "path.jobId",
        "sourcePath": "CreateRecordingJobRequest",
    },
    "singleSessionIdentity": {
        "rule": "all_equal",
        "paths": [
            "job.sessionId",
            "job.captureManifest.sessionId",
            "headers.Idempotency-Key.session",
            "manifestChunk.replayKey.sessionId",
        ],
    },
    "chunkTrackMembership": {
        "rule": "all_equal_and_declared",
        "paths": [
            "path.trackId",
            "headers.Idempotency-Key.track",
            "manifestChunk.replayKey.trackId",
        ],
        "setPath": "CreateRecordingJobRequest.tracks[*].trackId",
    },
    "sequenceIdentity": {
        "rule": "all_equal",
        "paths": [
            "path.sequenceStart-path.sequenceEnd",
            "headers.Idempotency-Key.sequenceStart-sequenceEnd",
            "manifestChunk.replayKey.sequenceStart-sequenceEnd",
        ],
    },
    "contentIdentity": {
        "rule": "all_equal",
        "paths": [
            "headers.X-Yap-Content-SHA256",
            "manifestChunk.contentIdentity.sha256",
        ],
    },
    "uniqueReplayKey": {
        "rule": "unique_by",
        "path": "CreateRecordingJobRequest.chunks[*].replayKey",
    },
}

LIVE_ENVELOPE_IDENTITY_INVARIANTS = {
    "authoritativeSessionPath": "sessionId",
    "nestedSessionIdentity": {
        "rule": "all_equal",
        "pathPattern": "**.sessionId",
        "authoritativePath": "sessionId",
    },
}

LIVE_START_IDENTITY_INVARIANTS = {
    "sessionIdentity": {
        "rule": "all_equal",
        "paths": ["sessionId", "metadata.sessionId"],
    },
    "declaredTrackIds": {"rule": "unique_by", "path": "tracks[*].trackId"},
}

LIVE_CHUNK_IDENTITY_INVARIANTS = {
    "sessionIdentity": {
        "rule": "all_equal",
        "paths": ["sessionId", "replayKey.sessionId"],
    },
    "trackMembership": {
        "rule": "member_of",
        "path": "replayKey.trackId",
        "setPath": "session.start.tracks[*].trackId",
    },
    "uniqueReplayKey": {
        "rule": "unique_by",
        "path": "replayKey",
        "scopePath": "sessionId",
    },
    "duplicateReplay": {
        "rule": "same_key_requires_same_content_identity",
        "contentPath": "contentIdentity",
    },
}

LIVE_GAP_IDENTITY_INVARIANTS = {
    "sessionIdentity": {
        "rule": "all_equal",
        "paths": ["sessionId", "gap.sessionId"],
    },
    "trackMembership": {
        "rule": "member_of",
        "path": "gap.trackId",
        "setPath": "session.start.tracks[*].trackId",
    },
}

LIVE_FINAL_IDENTITY_INVARIANTS = {
    "sessionIdentity": {
        "rule": "all_equal",
        "paths": ["sessionId", "result.sessionId"],
    }
}

BATCH_PROVENANCE_INVARIANTS = {
    "originTrackCoherence": {
        "rule": "origin_matches_all_track_source_kinds",
        "cases": {
            "imported_file": "imported",
            "live_capture": "captured",
        },
    },
    "importedPhysicalSourceHint": {
        "rule": "physical_source_requires_user_declared",
        "provenancePath": "tracks[*].source.provenance",
        "allowedObjectKind": "user_declared",
    },
}

LIVE_PROVENANCE_INVARIANTS = {
    "requiredOrigin": "live_capture",
    "originTrackCoherence": {
        "rule": "origin_matches_all_track_source_kinds",
        "cases": {"live_capture": "captured"},
    },
}

RECORDING_JOB_STATUSES = [
    "accepted",
    "preflighting",
    "blocked_setup_required",
    "blocked_server_unavailable",
    "blocked_sign_in_required",
    "queued_local_fallback",
    "queued_server",
    "preprocessing",
    "uploading",
    "server_processing",
    "local_transcribing",
    "saving",
    "diarization_queued",
    "diarization_running",
    "complete",
    "partial",
    "failed",
    "cancelled",
]

CLIENT_EVENT_TYPES = [
    "session.start",
    "audio.chunk",
    "audio.gap",
    "session.finish",
    "session.cancel",
    "ping",
]

SERVER_EVENT_TYPES = [
    "session.accepted",
    "transcript.partial",
    "transcript.final",
    "server.backpressure",
    "session.error",
    "session.finished",
    "pong",
]


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise AssertionError(f"{path} must contain a JSON object")
    return value


def make_job_request(origin: str, track_source: dict[str, Any]) -> dict[str, Any]:
    session_id = "s-provenance-test"
    track_id = "track-1"
    return {
        "displayName": "Provenance contract test",
        "metadata": {
            "sessionId": session_id,
            "mode": "dictation",
            "origin": origin,
            "triggerMode": "toggle",
            "startedAtUtc": "2026-07-12T16:00:00Z",
            "utcOffsetMinutesAtStart": None,
            "localeHintBcp47": None,
            "countryCodeHint": None,
            "preferredLanguagesBcp47": [],
            "appVersion": "0.1.0",
            "platform": "windows",
            "privacyPolicyVersion": "unconfigured",
            "retentionExpiresAtUtc": None,
        },
        "tracks": [
            {
                "trackId": track_id,
                "source": deepcopy(track_source),
                "deviceId": None,
                "originalSampleRateHz": 48000,
                "originalChannels": 1,
            }
        ],
        "route": "server_batch",
        "captureManifest": {
            "schemaVersion": 1,
            "sessionId": session_id,
            "sha256": "a" * 64,
            "byteLength": 4096,
        },
        "chunks": [
            {
                "replayKey": {
                    "schemaVersion": 1,
                    "sessionId": session_id,
                    "trackId": track_id,
                    "sequenceStart": 0,
                    "sequenceEnd": 159,
                },
                "contentIdentity": {
                    "sha256": "b" * 64,
                    "byteLength": 320,
                },
                "audioCodec": "pcm_s16le",
                "sampleRateHz": 16000,
                "channels": 1,
                "startMs": 0,
                "durationMs": 10,
            }
        ],
    }


def make_live_start(origin: str, track_source: dict[str, Any]) -> dict[str, Any]:
    job_request = make_job_request(origin, track_source)
    return {
        "schemaVersion": 1,
        "sessionId": job_request["metadata"]["sessionId"],
        "eventSequence": 0,
        "eventType": "session.start",
        "metadata": job_request["metadata"],
        "tracks": job_request["tracks"],
        "route": "server_live",
    }


def resolve_pointer(document: dict[str, Any], pointer: str) -> Any:
    if not pointer.startswith("#/"):
        raise AssertionError(f"unsupported JSON pointer: {pointer}")
    value: Any = document
    for token in pointer[2:].split("/"):
        decoded = token.replace("~1", "/").replace("~0", "~")
        value = value[decoded]
    return value


def resolve_reference(
    reference: str,
    document_name: str,
    documents: dict[str, dict[str, Any]],
) -> tuple[Any, str]:
    if reference.startswith("#/"):
        target_name = document_name
        pointer = reference
    else:
        target_name, separator, fragment = reference.partition("#")
        if not separator or target_name not in documents:
            raise AssertionError(f"unsupported schema reference: {reference}")
        pointer = f"#{fragment}"
    return resolve_pointer(documents[target_name], pointer), target_name


def iter_references(value: Any) -> list[str]:
    references: list[str] = []
    if isinstance(value, dict):
        reference = value.get("$ref")
        if isinstance(reference, str):
            references.append(reference)
        for child in value.values():
            references.extend(iter_references(child))
    elif isinstance(value, list):
        for child in value:
            references.extend(iter_references(child))
    return references


def json_type_matches(value: Any, expected: str) -> bool:
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "null":
        return value is None
    raise AssertionError(f"unsupported schema type in subset checker: {expected}")


def evaluated_property_names(
    schema: dict[str, Any],
    *,
    document_name: str,
    documents: dict[str, dict[str, Any]],
    seen: set[tuple[str, str]] | None = None,
) -> set[str]:
    seen = set() if seen is None else seen
    names = set(schema.get("properties", {}))
    reference = schema.get("$ref")
    if isinstance(reference, str):
        key = (document_name, reference)
        if key not in seen:
            seen.add(key)
            target, target_name = resolve_reference(reference, document_name, documents)
            if isinstance(target, dict):
                names.update(
                    evaluated_property_names(
                        target,
                        document_name=target_name,
                        documents=documents,
                        seen=seen,
                    )
                )
    for subschema in schema.get("allOf", []):
        names.update(
            evaluated_property_names(
                subschema,
                document_name=document_name,
                documents=documents,
                seen=seen,
            )
        )
    return names


def assert_schema_subset(
    value: Any,
    schema: dict[str, Any],
    *,
    document_name: str,
    documents: dict[str, dict[str, Any]],
    path: str = "$",
) -> None:
    reference = schema.get("$ref")
    if isinstance(reference, str):
        target, target_name = resolve_reference(reference, document_name, documents)
        if not isinstance(target, dict):
            raise AssertionError(f"{path}: $ref must resolve to a schema object")
        assert_schema_subset(
            value,
            target,
            document_name=target_name,
            documents=documents,
            path=path,
        )

    for subschema in schema.get("allOf", []):
        assert_schema_subset(
            value,
            subschema,
            document_name=document_name,
            documents=documents,
            path=path,
        )

    condition = schema.get("if")
    if isinstance(condition, dict):
        condition_matches = True
        try:
            assert_schema_subset(
                value,
                condition,
                document_name=document_name,
                documents=documents,
                path=path,
            )
        except AssertionError:
            condition_matches = False
        branch = schema.get("then" if condition_matches else "else")
        if isinstance(branch, dict):
            assert_schema_subset(
                value,
                branch,
                document_name=document_name,
                documents=documents,
                path=path,
            )

    one_of = schema.get("oneOf")
    if isinstance(one_of, list):
        matches = 0
        for candidate in one_of:
            try:
                assert_schema_subset(
                    value,
                    candidate,
                    document_name=document_name,
                    documents=documents,
                    path=path,
                )
            except AssertionError:
                continue
            matches += 1
        if matches != 1:
            raise AssertionError(f"{path}: expected exactly one oneOf match, got {matches}")

    if "const" in schema and value != schema["const"]:
        raise AssertionError(f"{path}: expected const {schema['const']!r}, got {value!r}")
    if "enum" in schema and value not in schema["enum"]:
        raise AssertionError(f"{path}: {value!r} is not in {schema['enum']!r}")

    expected_types = schema.get("type")
    if expected_types is not None:
        if isinstance(expected_types, str):
            expected_types = [expected_types]
        if not any(json_type_matches(value, expected) for expected in expected_types):
            raise AssertionError(
                f"{path}: expected type {expected_types!r}, got {type(value).__name__}"
            )

    if isinstance(value, dict):
        required = schema.get("required", [])
        missing = [name for name in required if name not in value]
        if missing:
            raise AssertionError(f"{path}: missing required fields {missing!r}")
        properties = schema.get("properties", {})
        for name, child in value.items():
            if name in properties:
                assert_schema_subset(
                    child,
                    properties[name],
                    document_name=document_name,
                    documents=documents,
                    path=f"{path}.{name}",
                )
                continue
            additional = schema.get("additionalProperties", True)
            if additional is False:
                raise AssertionError(f"{path}: unexpected field {name!r}")
            if isinstance(additional, dict):
                assert_schema_subset(
                    child,
                    additional,
                    document_name=document_name,
                    documents=documents,
                    path=f"{path}.{name}",
                )

        if schema.get("unevaluatedProperties") is False:
            allowed = evaluated_property_names(
                schema,
                document_name=document_name,
                documents=documents,
            )
            extras = sorted(set(value) - allowed)
            if extras:
                raise AssertionError(f"{path}: unexpected fields {extras!r}")

    if isinstance(value, list) and "items" in schema:
        for index, child in enumerate(value):
            assert_schema_subset(
                child,
                schema["items"],
                document_name=document_name,
                documents=documents,
                path=f"{path}[{index}]",
            )


def schema_property_names(value: Any) -> list[str]:
    names: list[str] = []
    if isinstance(value, dict):
        properties = value.get("properties")
        if isinstance(properties, dict):
            names.extend(properties)
        for child in value.values():
            names.extend(schema_property_names(child))
    elif isinstance(value, list):
        for child in value:
            names.extend(schema_property_names(child))
    return names


class ContractTests(unittest.TestCase):
    def test_openapi_exposes_the_phase_3_and_5_boundary(self) -> None:
        document = load_json(OPENAPI_PATH)

        self.assertEqual(document["openapi"], "3.1.0")
        self.assertEqual(set(document["paths"]), {path for path, _ in HTTP_OPERATIONS})
        for (path, method), operation_id in HTTP_OPERATIONS.items():
            operation = document["paths"][path][method]
            self.assertEqual(operation["operationId"], operation_id)
            behavior, owner = PHASE_BOUNDARY[(path, method)]
            self.assertEqual(operation["x-yap-phase-3-behavior"], behavior)
            self.assertEqual(operation["x-yap-later-owner"], owner)

        schemas = document["components"]["schemas"]
        expected_components = {
            "RecordingJobStatus",
            "SessionMode",
            "SessionOrigin",
            "AudioRoute",
            "SessionMetadata",
            "CaptureTrackDescriptor",
            "ChunkReplayKey",
            "ContentIdentity",
            "AudioGap",
            "CaptureManifestReference",
            "ResultAuthority",
            "ResultStatus",
            "TranscriptResultRevision",
            "SpeakerResultRevision",
            "SpeakerTurn",
            "AlignedWord",
            "ServerCapabilities",
            "HealthView",
            "RecordingJob",
            "ApiError",
        }
        self.assertTrue(expected_components.issubset(schemas))
        self.assertEqual(schemas["RecordingJobStatus"]["enum"], RECORDING_JOB_STATUSES)
        self.assertNotIn("server_processing_cohere", json.dumps(document))
        self.assertEqual(
            [name for name in schema_property_names(document) if "_" in name], []
        )

        origin_projection = {
            "live_capture": "liveCapture",
            "imported_file": "importedFile",
        }
        route_projection = {
            "local_fallback": "localFallback",
            "server_batch": "serverBatch",
            "server_live": "serverLive",
        }
        self.assertEqual(schemas["SessionOrigin"]["enum"], list(origin_projection))
        self.assertEqual(
            schemas["SessionOrigin"]["x-yap-recording-job-view-projection"],
            origin_projection,
        )
        self.assertEqual(
            {react: wire for wire, react in origin_projection.items()},
            {"liveCapture": "live_capture", "importedFile": "imported_file"},
        )
        self.assertEqual(schemas["AudioRoute"]["enum"], list(route_projection))
        self.assertEqual(
            schemas["AudioRoute"]["x-yap-recording-job-view-projection"],
            route_projection,
        )
        self.assertEqual(
            {react: wire for wire, react in route_projection.items()},
            {
                "localFallback": "local_fallback",
                "serverBatch": "server_batch",
                "serverLive": "server_live",
            },
        )

        metadata = schemas["SessionMetadata"]
        self.assertEqual(metadata["properties"]["startedAtUtc"]["format"], "date-time")
        self.assertEqual(metadata["properties"]["localeHintBcp47"]["maxLength"], 35)
        self.assertEqual(
            metadata["properties"]["preferredLanguagesBcp47"]["maxItems"], 8
        )
        self.assertFalse(
            metadata["properties"]["preferredLanguagesBcp47"].get(
                "uniqueItems", False
            )
        )
        self.assertEqual(
            metadata["properties"]["countryCodeHint"]["pattern"], "^[A-Z]{2}$"
        )
        self.assertIn(
            "unconfigured",
            metadata["properties"]["privacyPolicyVersion"]["description"],
        )
        self.assertIn(
            "opaque",
            schemas["CaptureTrackDescriptor"]["properties"]["deviceId"][
                "description"
            ].lower(),
        )

        job_request = schemas["CreateRecordingJobRequest"]
        replay_key = schemas["ChunkReplayKey"]
        forbidden_ownership_fields = {
            "tenantId",
            "tenant_id",
            "ownerSubjectId",
            "owner_subject_id",
            "ownerNamespace",
            "owner_namespace",
        }
        self.assertTrue(forbidden_ownership_fields.isdisjoint(job_request["properties"]))
        self.assertTrue(forbidden_ownership_fields.isdisjoint(replay_key["properties"]))

        api_error = schemas["ApiError"]
        self.assertEqual(
            set(api_error["required"]),
            {"code", "message", "retryable", "requestId"},
        )
        self.assertEqual(
            api_error["example"],
            {
                "code": "SERVER_BUSY",
                "message": "Server capacity is temporarily unavailable.",
                "retryable": True,
                "requestId": "req-01J...",
            },
        )

    def test_http_source_identity_and_manifest_invariants_are_normative(self) -> None:
        document = load_json(OPENAPI_PATH)
        schemas = document["components"]["schemas"]
        cases = [
            (
                "CreateRecordingJobRequest",
                schemas["CreateRecordingJobRequest"].get(
                    "x-yap-source-identity-invariants"
                ),
                CREATE_JOB_IDENTITY_INVARIANTS,
            ),
            (
                "RecordingJob",
                schemas["RecordingJob"].get("x-yap-source-identity-invariants"),
                RECORDING_JOB_IDENTITY_INVARIANTS,
            ),
            (
                "CommitRecordingJobRequest",
                schemas["CommitRecordingJobRequest"].get(
                    "x-yap-source-identity-invariants"
                ),
                COMMIT_IDENTITY_INVARIANTS,
            ),
            (
                "uploadJobChunk",
                document["paths"][CHUNK_PATH]["put"].get(
                    "x-yap-source-identity-invariants"
                ),
                CHUNK_IDENTITY_INVARIANTS,
            ),
        ]
        for label, actual, expected in cases:
            with self.subTest(contract=label):
                self.assertEqual(actual, expected)

        self.assertIs(
            schemas["CaptureManifestReference"].get("x-yap-immutable"), True
        )

    def test_session_origin_and_track_provenance_are_coherent(self) -> None:
        openapi = load_json(OPENAPI_PATH)
        live_schema = load_json(LIVE_EVENTS_PATH)
        documents = {
            "openapi.json": openapi,
            "live-events.schema.json": live_schema,
        }
        create_schema = openapi["components"]["schemas"][
            "CreateRecordingJobRequest"
        ]
        live_start_schema = live_schema["$defs"]["SessionStartEvent"]

        with self.subTest(contract="batch provenance metadata"):
            self.assertEqual(
                create_schema.get("x-yap-source-provenance-invariants"),
                BATCH_PROVENANCE_INVARIANTS,
            )
        with self.subTest(contract="live provenance metadata"):
            self.assertEqual(
                live_start_schema["allOf"][1].get(
                    "x-yap-source-provenance-invariants"
                ),
                LIVE_PROVENANCE_INVARIANTS,
            )

        captured = {"kind": "captured", "source": "microphone"}
        imported = {"kind": "imported", "provenance": "unknown"}
        user_declared = {
            "kind": "imported",
            "provenance": {
                "kind": "user_declared",
                "source": "microphone",
            },
        }

        positive_cases = [
            (
                "batch imported unknown",
                make_job_request("imported_file", imported),
                create_schema,
                "openapi.json",
            ),
            (
                "batch imported user-declared microphone",
                make_job_request("imported_file", user_declared),
                create_schema,
                "openapi.json",
            ),
            (
                "batch live capture",
                make_job_request("live_capture", captured),
                create_schema,
                "openapi.json",
            ),
            (
                "live start captured microphone",
                make_live_start("live_capture", captured),
                live_start_schema,
                "live-events.schema.json",
            ),
        ]
        for label, value, schema, document_name in positive_cases:
            with self.subTest(valid=label):
                assert_schema_subset(
                    value,
                    schema,
                    document_name=document_name,
                    documents=documents,
                )

        invalid_hint = {
            "kind": "imported",
            "provenance": {
                "kind": "inferred",
                "source": "microphone",
            },
        }
        negative_cases = [
            (
                "batch imported origin cannot claim captured track",
                make_job_request("imported_file", captured),
                create_schema,
                "openapi.json",
            ),
            (
                "batch live origin cannot claim imported track",
                make_job_request("live_capture", imported),
                create_schema,
                "openapi.json",
            ),
            (
                "imported physical hint must be user-declared",
                make_job_request("imported_file", invalid_hint),
                create_schema,
                "openapi.json",
            ),
            (
                "live start cannot claim imported track",
                make_live_start("live_capture", imported),
                live_start_schema,
                "live-events.schema.json",
            ),
            (
                "live start cannot use imported origin",
                make_live_start("imported_file", imported),
                live_start_schema,
                "live-events.schema.json",
            ),
        ]
        for label, value, schema, document_name in negative_cases:
            with self.subTest(invalid=label):
                with self.assertRaises(AssertionError):
                    assert_schema_subset(
                        value,
                        schema,
                        document_name=document_name,
                        documents=documents,
                    )

    def test_http_operation_schema_links_are_frozen(self) -> None:
        document = load_json(OPENAPI_PATH)
        documents = {"openapi.json": document}

        for contract in HTTP_SCHEMA_CONTRACTS:
            path = contract["path"]
            method = contract["method"]
            operation = document["paths"][path][method]
            with self.subTest(path=path, method=method, link="responses"):
                self.assertEqual(
                    set(operation["responses"]),
                    set(contract["success"]) | set(contract["errors"]),
                )

            expected_request = contract["request"]
            with self.subTest(path=path, method=method, link="request"):
                if expected_request is None:
                    self.assertNotIn("requestBody", operation)
                else:
                    media_type, expected_schema = expected_request
                    request_body = operation["requestBody"]
                    self.assertTrue(request_body["required"])
                    self.assertEqual(set(request_body["content"]), {media_type})
                    actual_schema = request_body["content"][media_type]["schema"]
                    if isinstance(expected_schema, str):
                        self.assertEqual(actual_schema, {"$ref": expected_schema})
                        resolve_reference(expected_schema, "openapi.json", documents)
                    else:
                        self.assertEqual(actual_schema, expected_schema)

            for status, expected_schema in contract["success"].items():
                with self.subTest(path=path, method=method, success=status):
                    response = operation["responses"][status]
                    if "$ref" in response:
                        response, _ = resolve_reference(
                            response["$ref"], "openapi.json", documents
                        )
                    if expected_schema is None:
                        self.assertNotIn("content", response)
                    else:
                        self.assertEqual(set(response["content"]), {"application/json"})
                        actual_schema = response["content"]["application/json"][
                            "schema"
                        ]
                        self.assertEqual(actual_schema, {"$ref": expected_schema})
                        resolve_reference(expected_schema, "openapi.json", documents)

            for status in contract["errors"]:
                with self.subTest(path=path, method=method, error=status):
                    response = operation["responses"][status]
                    if "$ref" in response:
                        response, _ = resolve_reference(
                            response["$ref"], "openapi.json", documents
                        )
                    self.assertEqual(set(response["content"]), {"application/json"})
                    error_schema = response["content"]["application/json"]["schema"]
                    self.assertEqual(
                        error_schema, {"$ref": "#/components/schemas/ApiError"}
                    )

        for reference in iter_references(document):
            with self.subTest(component_reference=reference):
                try:
                    resolved, _ = resolve_reference(
                        reference, "openapi.json", documents
                    )
                except (AssertionError, KeyError, TypeError) as error:
                    self.fail(f"unresolved OpenAPI component reference {reference}: {error}")
                self.assertIsNotNone(resolved)

    def test_chunk_contract_separates_replay_key_from_content_hash(self) -> None:
        document = load_json(OPENAPI_PATH)
        schemas = document["components"]["schemas"]
        replay_key = schemas["ChunkReplayKey"]
        content_identity = schemas["ContentIdentity"]

        self.assertEqual(
            set(replay_key["required"]),
            {"schemaVersion", "sessionId", "trackId", "sequenceStart", "sequenceEnd"},
        )
        self.assertEqual(
            set(content_identity["required"]), {"sha256", "byteLength"}
        )
        self.assertNotIn("sha256", replay_key["properties"])
        self.assertNotIn("sequenceStart", content_identity["properties"])
        self.assertEqual(
            content_identity["properties"]["sha256"]["pattern"], "^[0-9a-f]{64}$"
        )

        operation = document["paths"][
            "/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}"
        ]["put"]
        parameters = {parameter["name"]: parameter for parameter in operation["parameters"]}
        required_headers = {
            "Idempotency-Key",
            "X-Yap-Content-SHA256",
            "X-Yap-Audio-Codec",
            "X-Yap-Sample-Rate-Hz",
            "X-Yap-Channels",
        }
        self.assertTrue(required_headers.issubset(parameters))
        for header in required_headers:
            self.assertTrue(parameters[header]["required"])
            self.assertEqual(parameters[header]["in"], "header")
        self.assertEqual(
            parameters["Idempotency-Key"]["schema"]["pattern"],
            "^[0-9]+/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+/[0-9]+/[0-9]+$",
        )
        self.assertEqual(
            parameters["X-Yap-Content-SHA256"]["schema"]["pattern"],
            "^[0-9a-f]{64}$",
        )
        self.assertEqual(
            parameters["X-Yap-Audio-Codec"]["schema"]["const"], "pcm_s16le"
        )
        self.assertEqual(
            parameters["X-Yap-Sample-Rate-Hz"]["schema"]["const"], 16000
        )
        self.assertEqual(parameters["X-Yap-Channels"]["schema"]["const"], 1)
        request_content = operation["requestBody"]["content"]
        self.assertEqual(set(request_content), {"application/octet-stream"})
        self.assertEqual(
            request_content["application/octet-stream"]["schema"],
            {"type": "string", "format": "binary"},
        )

        self.assertEqual(
            operation["x-yap-replay-semantics"],
            {
                "sameKeySameHash": "replay_success",
                "sameKeyDifferentHash": {
                    "status": 409,
                    "code": "CONTENT_IDENTITY_CONFLICT",
                },
                "differentKeySameHash": "allowed",
                "headerManifestMismatch": "reject_before_accept",
            },
        )
        conflict = operation["responses"]["409"]
        self.assertEqual(
            conflict["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ApiError",
        )
        self.assertEqual(
            conflict["content"]["application/json"]["example"]["code"],
            "CONTENT_IDENTITY_CONFLICT",
        )

    def test_live_events_have_version_and_monotonic_sequence(self) -> None:
        schema = load_json(LIVE_EVENTS_PATH)

        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertIn(urlparse(schema["$id"]).scheme, {"http", "https"})
        self.assertEqual(schema["x-yap-client-events"], CLIENT_EVENT_TYPES)
        self.assertEqual(schema["x-yap-server-events"], SERVER_EVENT_TYPES)
        self.assertEqual(
            schema["x-yap-ordering"],
            {
                "scope": "per_session_per_direction",
                "eventSequence": "strictly_increasing",
                "staleEventSequence": "ignore",
                "duplicateFinalEvents": "idempotent",
            },
        )

        mapping = schema["discriminator"]["mapping"]
        all_event_types = CLIENT_EVENT_TYPES + SERVER_EVENT_TYPES
        self.assertEqual(set(mapping), set(all_event_types))
        self.assertEqual(
            {entry["$ref"] for entry in schema["oneOf"]}, set(mapping.values())
        )

        envelope = schema["$defs"]["EventEnvelope"]
        self.assertEqual(
            set(envelope["required"]),
            {"schemaVersion", "sessionId", "eventSequence", "eventType"},
        )
        self.assertEqual(envelope["properties"]["schemaVersion"]["const"], 1)
        self.assertEqual(envelope["properties"]["eventSequence"]["minimum"], 0)

        for event_type, reference in mapping.items():
            definition_name = reference.removeprefix("#/$defs/")
            event_schema = schema["$defs"][definition_name]
            self.assertEqual(
                event_schema["allOf"][1]["properties"]["eventType"]["const"],
                event_type,
            )

        audio_chunk = schema["$defs"]["AudioChunkEvent"]["allOf"][1]
        self.assertTrue(
            {"replayKey", "contentIdentity", "binaryFollows"}.issubset(
                audio_chunk["required"]
            )
        )
        self.assertTrue(audio_chunk["properties"]["binaryFollows"]["const"])
        self.assertIn(
            "immediately following WebSocket binary message",
            audio_chunk["description"],
        )

        document = load_json(OPENAPI_PATH)
        live_operation = document["paths"]["/v1/live"]["get"]
        self.assertEqual(
            live_operation["x-yap-live-events-schema"], "./live-events.schema.json"
        )
        self.assertEqual(live_operation["x-yap-phase-3-behavior"], "Event schema only")

    def test_live_source_identity_invariants_are_normative(self) -> None:
        live_schema = load_json(LIVE_EVENTS_PATH)
        openapi = load_json(OPENAPI_PATH)
        definitions = live_schema["$defs"]
        cases = [
            (
                "EventEnvelope",
                definitions["EventEnvelope"].get(
                    "x-yap-source-identity-invariants"
                ),
                LIVE_ENVELOPE_IDENTITY_INVARIANTS,
            ),
            (
                "SessionStartEvent",
                definitions["SessionStartEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                LIVE_START_IDENTITY_INVARIANTS,
            ),
            (
                "AudioChunkEvent",
                definitions["AudioChunkEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                LIVE_CHUNK_IDENTITY_INVARIANTS,
            ),
            (
                "AudioGapEvent",
                definitions["AudioGapEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                LIVE_GAP_IDENTITY_INVARIANTS,
            ),
            (
                "TranscriptFinalEvent",
                definitions["TranscriptFinalEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                LIVE_FINAL_IDENTITY_INVARIANTS,
            ),
        ]
        for label, actual, expected in cases:
            with self.subTest(contract=label):
                self.assertEqual(actual, expected)

        documents = {
            "live-events.schema.json": live_schema,
            "openapi.json": openapi,
        }
        for reference in iter_references(live_schema):
            with self.subTest(schema_reference=reference):
                try:
                    resolved, _ = resolve_reference(
                        reference, "live-events.schema.json", documents
                    )
                except (AssertionError, KeyError, TypeError) as error:
                    self.fail(f"unresolved live schema reference {reference}: {error}")
                self.assertIsNotNone(resolved)

    def test_examples_conform_to_required_contract_fields(self) -> None:
        document = load_json(OPENAPI_PATH)
        live_schema = load_json(LIVE_EVENTS_PATH)
        documents = {
            "openapi.json": document,
            "live-events.schema.json": live_schema,
        }
        health_example = load_json(EXAMPLES_ROOT / "health.ok.json")
        job_example = load_json(EXAMPLES_ROOT / "job.accepted.json")
        partial_example = load_json(EXAMPLES_ROOT / "live.partial.json")

        schemas = document["components"]["schemas"]
        assert_schema_subset(
            health_example,
            schemas["HealthView"],
            document_name="openapi.json",
            documents=documents,
        )
        assert_schema_subset(
            job_example,
            schemas["RecordingJob"],
            document_name="openapi.json",
            documents=documents,
        )
        assert_schema_subset(
            partial_example,
            live_schema["$defs"]["TranscriptPartialEvent"],
            document_name="live-events.schema.json",
            documents=documents,
        )
        assert_schema_subset(
            ["en-US", "es-MX"],
            schemas["SessionMetadata"]["properties"]["preferredLanguagesBcp47"],
            document_name="openapi.json",
            documents=documents,
        )

        invalid_health = deepcopy(health_example)
        del invalid_health["capabilities"]["jobStatus"]
        with self.assertRaisesRegex(AssertionError, "missing required fields"):
            assert_schema_subset(
                invalid_health,
                schemas["HealthView"],
                document_name="openapi.json",
                documents=documents,
            )

        invalid_job = deepcopy(job_example)
        invalid_job["captureManifest"]["byteLength"] = "4096"
        with self.assertRaisesRegex(AssertionError, "expected type"):
            assert_schema_subset(
                invalid_job,
                schemas["RecordingJob"],
                document_name="openapi.json",
                documents=documents,
            )

        invalid_job = deepcopy(job_example)
        invalid_job["unexpected"] = True
        with self.assertRaisesRegex(AssertionError, "unexpected field"):
            assert_schema_subset(
                invalid_job,
                schemas["RecordingJob"],
                document_name="openapi.json",
                documents=documents,
            )

        invalid_partial = deepcopy(partial_example)
        invalid_partial["unexpected"] = True
        with self.assertRaisesRegex(AssertionError, "unexpected field"):
            assert_schema_subset(
                invalid_partial,
                live_schema["$defs"]["TranscriptPartialEvent"],
                document_name="live-events.schema.json",
                documents=documents,
            )

        self.assertEqual(health_example["status"], "ok")
        self.assertEqual(health_example["auth"], "not_configured")
        self.assertEqual(job_example["status"], "accepted")
        self.assertEqual(job_example["sessionOrigin"], "imported_file")
        self.assertEqual(job_example["route"], "server_batch")
        self.assertEqual(partial_example["eventType"], "transcript.partial")
        self.assertEqual(partial_example["schemaVersion"], 1)
        self.assertGreaterEqual(partial_example["eventSequence"], 0)

    def test_python_health_shapes_use_explicit_wire_names(self) -> None:
        from yap_server.schemas import HealthView, ServerCapabilities

        capabilities = ServerCapabilities(
            batch_jobs=False,
            live_streaming=False,
            job_status=False,
        )
        view = HealthView(
            service="yap-server",
            status="ok",
            api_version="1",
            auth="not_configured",
            capabilities=capabilities,
        )

        self.assertEqual(
            capabilities.to_wire(),
            {"batchJobs": False, "liveStreaming": False, "jobStatus": False},
        )
        self.assertEqual(view.to_wire(), load_json(EXAMPLES_ROOT / "health.ok.json"))
        self.assertFalse(hasattr(view, "__dict__"))
        with self.assertRaises((AttributeError, TypeError)):
            view.status = "broken"  # type: ignore[misc]


if __name__ == "__main__":
    unittest.main()
