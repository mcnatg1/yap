import json
from copy import deepcopy
from pathlib import Path
from typing import Any

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
            "mode": "meeting",
            "origin": origin,
            "triggerMode": "toggle",
            "startedAtUtc": "2026-07-12T16:00:00Z",
            "utcOffsetMinutesAtStart": None,
            "localeHintBcp47": None,
            "countryCodeHint": None,
            "preferredLanguagesBcp47": ["en-US"],
            "appVersion": "0.1.0",
            "platform": "windows",
            "privacyPolicyVersion": "unconfigured",
            "retentionExpiresAtUtc": "2026-08-11T16:00:00Z",
        },
        "tracks": [
            {
                "trackId": track_id,
                "source": deepcopy(track_source),
                "deviceId": None,
                "originalSampleRateHz": 16000,
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
