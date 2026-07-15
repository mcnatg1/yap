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
    def test_examples_conform_to_required_contract_fields(self) -> None:
        document = contract_schema.load_json(http_contract.OPENAPI_PATH)
        live_schema = contract_schema.load_json(http_contract.LIVE_EVENTS_PATH)
        documents = {
            "openapi.json": document,
            "live-events.schema.json": live_schema,
        }
        health_example = contract_schema.load_json(http_contract.EXAMPLES_ROOT / "health.ok.json")
        job_example = contract_schema.load_json(http_contract.EXAMPLES_ROOT / "job.accepted.json")
        partial_example = contract_schema.load_json(http_contract.EXAMPLES_ROOT / "live.partial.json")

        schemas = document["components"]["schemas"]
        contract_schema.assert_schema_subset(
            health_example,
            schemas["HealthView"],
            document_name="openapi.json",
            documents=documents,
        )
        contract_schema.assert_schema_subset(
            job_example,
            schemas["RecordingJob"],
            document_name="openapi.json",
            documents=documents,
        )
        contract_schema.assert_schema_subset(
            partial_example,
            live_schema["$defs"]["TranscriptPartialEvent"],
            document_name="live-events.schema.json",
            documents=documents,
        )
        contract_schema.assert_schema_subset(
            ["en-US", "es-MX"],
            schemas["SessionMetadata"]["properties"]["preferredLanguagesBcp47"],
            document_name="openapi.json",
            documents=documents,
        )

        invalid_health = deepcopy(health_example)
        del invalid_health["capabilities"]["jobStatus"]
        with self.assertRaisesRegex(AssertionError, "missing required fields"):
            contract_schema.assert_schema_subset(
                invalid_health,
                schemas["HealthView"],
                document_name="openapi.json",
                documents=documents,
            )

        invalid_job = deepcopy(job_example)
        invalid_job["captureManifest"]["byteLength"] = "4096"
        with self.assertRaisesRegex(AssertionError, "expected type"):
            contract_schema.assert_schema_subset(
                invalid_job,
                schemas["RecordingJob"],
                document_name="openapi.json",
                documents=documents,
            )

        invalid_job = deepcopy(job_example)
        invalid_job["unexpected"] = True
        with self.assertRaisesRegex(AssertionError, "unexpected field"):
            contract_schema.assert_schema_subset(
                invalid_job,
                schemas["RecordingJob"],
                document_name="openapi.json",
                documents=documents,
            )

        invalid_partial = deepcopy(partial_example)
        invalid_partial["unexpected"] = True
        with self.assertRaisesRegex(AssertionError, "unexpected field"):
            contract_schema.assert_schema_subset(
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
        self.assertEqual(view.to_wire(), contract_schema.load_json(http_contract.EXAMPLES_ROOT / "health.ok.json"))
        self.assertFalse(hasattr(view, "__dict__"))
        with self.assertRaises((AttributeError, TypeError)):
            view.status = "broken"  # type: ignore[misc]


if __name__ == "__main__":
    unittest.main()
