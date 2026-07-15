from pathlib import Path
from typing import Any

SERVER_ROOT = Path(__file__).resolve().parents[2]
OPENAPI_PATH = SERVER_ROOT / "openapi" / "openapi.json"
LIVE_EVENTS_PATH = SERVER_ROOT / "openapi" / "live-events.schema.json"
EXAMPLES_ROOT = SERVER_ROOT / "openapi" / "examples"

HTTP_OPERATIONS = {
    ("/v1/health", "get"): "getHealth",
    ("/v1/jobs", "post"): "createJob",
    ("/v1/jobs/{jobId}", "get"): "getJob",
    ("/v1/jobs/{jobId}/result", "get"): "getJobResult",
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
    ("/v1/jobs/{jobId}/result", "get"): (
        "Contract only",
        "Phase 5 result retrieval",
    ),
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

CURRENT_BEHAVIOR = {
    ("/v1/jobs", "post"): "Implemented in the Phase 5 loopback batch runtime",
    ("/v1/jobs/{jobId}", "get"): "Implemented in the Phase 5 loopback batch runtime",
    ("/v1/jobs/{jobId}", "delete"): "Implemented in the Phase 5 loopback batch runtime",
    ("/v1/jobs/{jobId}/result", "get"): "Implemented in the Phase 5 loopback batch runtime",
    (
        "/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}",
        "put",
    ): "Implemented in the Phase 5 loopback batch runtime",
    ("/v1/jobs/{jobId}/commit", "post"): "Implemented in the Phase 5 loopback batch runtime",
    ("/v1/live", "get"): "Contract only; capability remains false",
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
        "errors": ["404", "501"],
    },
    {
        "path": "/v1/jobs/{jobId}/result",
        "method": "get",
        "request": None,
        "success": {"200": "#/components/schemas/TranscriptResultRevision"},
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
