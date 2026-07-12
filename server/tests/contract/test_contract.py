import json
import unittest
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


def assert_required_fields(
    test: unittest.TestCase,
    schema: dict[str, Any],
    value: dict[str, Any],
) -> None:
    test.assertTrue(set(schema["required"]).issubset(value))


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

        self.assertEqual(
            document["paths"]["/v1/health"]["get"]["responses"]["200"][
                "content"
            ]["application/json"]["schema"]["$ref"],
            "#/components/schemas/HealthView",
        )
        self.assertEqual(
            document["paths"]["/v1/jobs"]["post"]["requestBody"]["content"][
                "application/json"
            ]["schema"]["$ref"],
            "#/components/schemas/CreateRecordingJobRequest",
        )
        for path, method, status in [
            ("/v1/jobs", "post", "202"),
            ("/v1/jobs/{jobId}", "get", "200"),
            ("/v1/jobs/{jobId}", "delete", "202"),
            ("/v1/jobs/{jobId}/commit", "post", "202"),
        ]:
            response = document["paths"][path][method]["responses"][status]
            self.assertEqual(
                response["content"]["application/json"]["schema"]["$ref"],
                "#/components/schemas/RecordingJob",
            )

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

    def test_examples_conform_to_required_contract_fields(self) -> None:
        document = load_json(OPENAPI_PATH)
        live_schema = load_json(LIVE_EVENTS_PATH)
        health_example = load_json(EXAMPLES_ROOT / "health.ok.json")
        job_example = load_json(EXAMPLES_ROOT / "job.accepted.json")
        partial_example = load_json(EXAMPLES_ROOT / "live.partial.json")

        schemas = document["components"]["schemas"]
        assert_required_fields(self, schemas["HealthView"], health_example)
        assert_required_fields(self, schemas["ServerCapabilities"], health_example["capabilities"])
        self.assertEqual(health_example["status"], "ok")
        self.assertEqual(health_example["auth"], "not_configured")

        assert_required_fields(self, schemas["RecordingJob"], job_example)
        self.assertEqual(job_example["status"], "accepted")
        self.assertEqual(job_example["sessionOrigin"], "imported_file")
        self.assertEqual(job_example["route"], "server_batch")

        envelope = live_schema["$defs"]["EventEnvelope"]
        partial = live_schema["$defs"]["TranscriptPartialEvent"]["allOf"][1]
        assert_required_fields(self, envelope, partial_example)
        assert_required_fields(self, partial, partial_example)
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
