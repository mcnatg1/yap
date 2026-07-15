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
    def test_http_source_identity_and_manifest_invariants_are_normative(self) -> None:
        document = contract_schema.load_json(http_contract.OPENAPI_PATH)
        schemas = document["components"]["schemas"]
        cases = [
            (
                "CreateRecordingJobRequest",
                schemas["CreateRecordingJobRequest"].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.CREATE_JOB_IDENTITY_INVARIANTS,
            ),
            (
                "RecordingJob",
                schemas["RecordingJob"].get("x-yap-source-identity-invariants"),
                identity_contract.RECORDING_JOB_IDENTITY_INVARIANTS,
            ),
            (
                "CommitRecordingJobRequest",
                schemas["CommitRecordingJobRequest"].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.COMMIT_IDENTITY_INVARIANTS,
            ),
            (
                "uploadJobChunk",
                document["paths"][http_contract.CHUNK_PATH]["put"].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.CHUNK_IDENTITY_INVARIANTS,
            ),
        ]
        for label, actual, expected in cases:
            with self.subTest(contract=label):
                self.assertEqual(actual, expected)

        self.assertIs(
            schemas["CaptureManifestReference"].get("x-yap-immutable"), True
        )

    def test_session_origin_and_track_provenance_are_coherent(self) -> None:
        openapi = contract_schema.load_json(http_contract.OPENAPI_PATH)
        live_schema = contract_schema.load_json(http_contract.LIVE_EVENTS_PATH)
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
                identity_contract.BATCH_PROVENANCE_INVARIANTS,
            )
        with self.subTest(contract="live provenance metadata"):
            self.assertEqual(
                live_start_schema["allOf"][1].get(
                    "x-yap-source-provenance-invariants"
                ),
                identity_contract.LIVE_PROVENANCE_INVARIANTS,
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
                contract_schema.make_job_request("imported_file", imported),
                create_schema,
                "openapi.json",
            ),
            (
                "batch imported user-declared microphone",
                contract_schema.make_job_request("imported_file", user_declared),
                create_schema,
                "openapi.json",
            ),
            (
                "live start captured microphone",
                contract_schema.make_live_start("live_capture", captured),
                live_start_schema,
                "live-events.schema.json",
            ),
        ]
        for label, value, schema, document_name in positive_cases:
            with self.subTest(valid=label):
                contract_schema.assert_schema_subset(
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
                contract_schema.make_job_request("imported_file", captured),
                create_schema,
                "openapi.json",
            ),
            (
                "batch live origin cannot claim imported track",
                contract_schema.make_job_request("live_capture", imported),
                create_schema,
                "openapi.json",
            ),
            (
                "batch live origin remains outside the Phase 5 profile",
                contract_schema.make_job_request("live_capture", captured),
                create_schema,
                "openapi.json",
            ),
            (
                "imported physical hint must be user-declared",
                contract_schema.make_job_request("imported_file", invalid_hint),
                create_schema,
                "openapi.json",
            ),
            (
                "live start cannot claim imported track",
                contract_schema.make_live_start("live_capture", imported),
                live_start_schema,
                "live-events.schema.json",
            ),
            (
                "live start cannot use imported origin",
                contract_schema.make_live_start("imported_file", imported),
                live_start_schema,
                "live-events.schema.json",
            ),
        ]
        for label, value, schema, document_name in negative_cases:
            with self.subTest(invalid=label):
                with self.assertRaises(AssertionError):
                    contract_schema.assert_schema_subset(
                        value,
                        schema,
                        document_name=document_name,
                        documents=documents,
                    )

    def test_http_operation_schema_links_are_frozen(self) -> None:
        document = contract_schema.load_json(http_contract.OPENAPI_PATH)
        documents = {"openapi.json": document}

        for contract in http_contract.HTTP_SCHEMA_CONTRACTS:
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
                        contract_schema.resolve_reference(expected_schema, "openapi.json", documents)
                    else:
                        self.assertEqual(actual_schema, expected_schema)

            for status, expected_schema in contract["success"].items():
                with self.subTest(path=path, method=method, success=status):
                    response = operation["responses"][status]
                    if "$ref" in response:
                        response, _ = contract_schema.resolve_reference(
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
                        contract_schema.resolve_reference(expected_schema, "openapi.json", documents)

            for status in contract["errors"]:
                with self.subTest(path=path, method=method, error=status):
                    response = operation["responses"][status]
                    if "$ref" in response:
                        response, _ = contract_schema.resolve_reference(
                            response["$ref"], "openapi.json", documents
                        )
                    self.assertEqual(set(response["content"]), {"application/json"})
                    error_schema = response["content"]["application/json"]["schema"]
                    self.assertEqual(
                        error_schema, {"$ref": "#/components/schemas/ApiError"}
                    )

        for reference in contract_schema.iter_references(document):
            with self.subTest(component_reference=reference):
                try:
                    resolved, _ = contract_schema.resolve_reference(
                        reference, "openapi.json", documents
                    )
                except (AssertionError, KeyError, TypeError) as error:
                    self.fail(f"unresolved OpenAPI component reference {reference}: {error}")
                self.assertIsNotNone(resolved)
