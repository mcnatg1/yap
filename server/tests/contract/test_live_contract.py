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
    def test_live_events_have_version_and_monotonic_sequence(self) -> None:
        schema = contract_schema.load_json(http_contract.LIVE_EVENTS_PATH)

        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertIn(urlparse(schema["$id"]).scheme, {"http", "https"})
        self.assertEqual(schema["x-yap-client-events"], identity_contract.CLIENT_EVENT_TYPES)
        self.assertEqual(schema["x-yap-server-events"], identity_contract.SERVER_EVENT_TYPES)
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
        all_event_types = identity_contract.CLIENT_EVENT_TYPES + identity_contract.SERVER_EVENT_TYPES
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

        document = contract_schema.load_json(http_contract.OPENAPI_PATH)
        live_operation = document["paths"]["/v1/live"]["get"]
        self.assertEqual(
            live_operation["x-yap-live-events-schema"], "./live-events.schema.json"
        )
        self.assertEqual(live_operation["x-yap-phase-3-behavior"], "Event schema only")

    def test_live_source_identity_invariants_are_normative(self) -> None:
        live_schema = contract_schema.load_json(http_contract.LIVE_EVENTS_PATH)
        openapi = contract_schema.load_json(http_contract.OPENAPI_PATH)
        definitions = live_schema["$defs"]
        cases = [
            (
                "EventEnvelope",
                definitions["EventEnvelope"].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.LIVE_ENVELOPE_IDENTITY_INVARIANTS,
            ),
            (
                "SessionStartEvent",
                definitions["SessionStartEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.LIVE_START_IDENTITY_INVARIANTS,
            ),
            (
                "AudioChunkEvent",
                definitions["AudioChunkEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.LIVE_CHUNK_IDENTITY_INVARIANTS,
            ),
            (
                "AudioGapEvent",
                definitions["AudioGapEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.LIVE_GAP_IDENTITY_INVARIANTS,
            ),
            (
                "TranscriptFinalEvent",
                definitions["TranscriptFinalEvent"]["allOf"][1].get(
                    "x-yap-source-identity-invariants"
                ),
                identity_contract.LIVE_FINAL_IDENTITY_INVARIANTS,
            ),
        ]
        for label, actual, expected in cases:
            with self.subTest(contract=label):
                self.assertEqual(actual, expected)

        documents = {
            "live-events.schema.json": live_schema,
            "openapi.json": openapi,
        }
        for reference in contract_schema.iter_references(live_schema):
            with self.subTest(schema_reference=reference):
                try:
                    resolved, _ = contract_schema.resolve_reference(
                        reference, "live-events.schema.json", documents
                    )
                except (AssertionError, KeyError, TypeError) as error:
                    self.fail(f"unresolved live schema reference {reference}: {error}")
                self.assertIsNotNone(resolved)
