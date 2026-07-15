import json
import unittest
from copy import deepcopy
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from . import contract_http_values as http_contract
from . import contract_identity_values as identity_contract
from . import contract_schema_support as contract_schema


class ContractTests(unittest.TestCase):
    def test_openapi_exposes_the_phase_3_and_5_boundary(self) -> None:
        document = contract_schema.load_json(http_contract.OPENAPI_PATH)

        self.assertEqual(document["openapi"], "3.1.0")
        self.assertEqual(set(document["paths"]), {path for path, _ in http_contract.HTTP_OPERATIONS})
        for (path, method), operation_id in http_contract.HTTP_OPERATIONS.items():
            operation = document["paths"][path][method]
            self.assertEqual(operation["operationId"], operation_id)
            behavior, owner = http_contract.PHASE_BOUNDARY[(path, method)]
            self.assertEqual(operation["x-yap-phase-3-behavior"], behavior)
            self.assertEqual(operation["x-yap-later-owner"], owner)
            if (path, method) in http_contract.CURRENT_BEHAVIOR:
                self.assertEqual(
                    operation["x-yap-current-behavior"],
                    http_contract.CURRENT_BEHAVIOR[(path, method)],
                )

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
        self.assertEqual(schemas["RecordingJobStatus"]["enum"], identity_contract.RECORDING_JOB_STATUSES)
        self.assertNotIn("server_processing_cohere", json.dumps(document))
        self.assertEqual(
            [name for name in contract_schema.schema_property_names(document) if "_" in name], []
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
        phase5_metadata = job_request["properties"]["metadata"]["allOf"][1][
            "properties"
        ]
        self.assertEqual(phase5_metadata["mode"]["const"], "meeting")
        self.assertEqual(phase5_metadata["origin"]["const"], "imported_file")
        self.assertEqual(
            phase5_metadata["retentionExpiresAtUtc"]["$ref"],
            "#/components/schemas/UtcDateTime",
        )
        self.assertEqual(job_request["properties"]["tracks"]["maxItems"], 1)
        self.assertEqual(job_request["properties"]["chunks"]["maxItems"], 4096)
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
