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
    "pcmFrameCount": {
        "rule": "inclusive_range_length_equals_pcm16_frame_count",
        "rangePath": "manifestChunk.replayKey.sequenceStart-sequenceEnd",
        "byteLengthPath": "manifestChunk.contentIdentity.byteLength",
        "formula": "sequenceEnd - sequenceStart + 1 == byteLength / 2",
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
