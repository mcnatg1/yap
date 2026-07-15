from __future__ import annotations

from typing import Mapping

from .contract_values import exact_keys, identifier


def persisted_state_metadata(state: Mapping[str, object]) -> tuple[str | None, bool]:
    schema_version = state.get("schemaVersion")
    if schema_version == 1:
        exact_keys(
            state,
            {"schemaVersion", "creation", "projection", "receipts"},
            "persisted job state",
        )
        return None, False
    if schema_version == 2:
        exact_keys(
            state,
            {
                "schemaVersion",
                "createIdempotencyKey",
                "creation",
                "projection",
                "receipts",
            },
            "persisted job state",
        )
        cancellation_requested = False
    elif schema_version == 3:
        exact_keys(
            state,
            {
                "schemaVersion",
                "createIdempotencyKey",
                "cancellationRequested",
                "creation",
                "projection",
                "receipts",
            },
            "persisted job state",
        )
        cancellation_requested = state.get("cancellationRequested")
        if not isinstance(cancellation_requested, bool):
            raise ValueError("persisted cancellation request is invalid")
    else:
        raise ValueError("persisted job state has an unsupported schema")
    raw_create_key = state.get("createIdempotencyKey")
    create_idempotency_key = (
        None
        if raw_create_key is None
        else identifier(raw_create_key, 128, "create idempotency key")
    )
    return create_idempotency_key, cancellation_requested
