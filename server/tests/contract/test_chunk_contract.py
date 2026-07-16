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
    def test_chunk_contract_separates_replay_key_from_content_hash(self) -> None:
        document = contract_schema.load_json(http_contract.OPENAPI_PATH)
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
