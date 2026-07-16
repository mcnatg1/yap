from __future__ import annotations

from concurrent.futures import Future
import hashlib
import threading

from yap_server.pools.batch_asr import (
    BatchAsrJob,
    PoolBackpressure,
    WorkerContainmentError,
    WorkerExecutionError,
)


class _Processor:
    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        raise AssertionError(f"job {job.job_id} must not dispatch before commit")


class _ControlledProcessor:
    def __init__(self) -> None:
        self.jobs: list[BatchAsrJob] = []
        self.future: Future[dict[str, object]] = Future()

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        self.jobs.append(job)
        return self.future


class _BusyProcessor:
    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        raise PoolBackpressure(f"capacity unavailable for {job.job_id}")


class _UnstoppableProcessor:
    def __init__(self) -> None:
        self.future: Future[dict[str, object]] = Future()

    def submit(self, _job: BatchAsrJob) -> Future[dict[str, object]]:
        self.future.set_running_or_notify_cancel()
        return self.future

    def cancel(self, _job_id: str) -> bool:
        return False


class _ActiveCancellationWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.stopped = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        if not cancellation.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} was not cancelled")
        self.stopped.set()
        raise WorkerExecutionError("isolated ASR worker was cancelled")


class _UnverifiedCleanupWorker:
    def __init__(self) -> None:
        self.started = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        if not cancellation.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} was not cancelled")
        raise WorkerContainmentError("owned container cleanup could not be verified")


class _DelayedCancellationWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.cancellation_received = threading.Event()
        self.release_cleanup = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        if not cancellation.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} was not cancelled")
        self.cancellation_received.set()
        if not self.release_cleanup.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} cleanup was not released")
        raise WorkerExecutionError("isolated ASR worker was cancelled")


def _create_request(
    *,
    session_id: str = "s-phase5-create",
    retention_expires_at_utc: str | None = "2026-08-13T21:00:00Z",
) -> dict[str, object]:
    track_id = "track-1"
    chunk = bytes(320)
    return {
        "displayName": "Phase 5 vertical slice",
        "metadata": {
            "sessionId": session_id,
            "mode": "meeting",
            "origin": "imported_file",
            "triggerMode": "toggle",
            "startedAtUtc": "2026-07-14T21:00:00Z",
            "utcOffsetMinutesAtStart": -300,
            "localeHintBcp47": "en-US",
            "countryCodeHint": "US",
            "preferredLanguagesBcp47": ["en-US"],
            "appVersion": "0.1.0",
            "platform": "windows",
            "privacyPolicyVersion": "development-only",
            "retentionExpiresAtUtc": retention_expires_at_utc,
        },
        "tracks": [
            {
                "trackId": track_id,
                "source": {"kind": "imported", "provenance": "unknown"},
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
                    "sha256": hashlib.sha256(chunk).hexdigest(),
                    "byteLength": len(chunk),
                },
                "audioCodec": "pcm_s16le",
                "sampleRateHz": 16000,
                "channels": 1,
                "startMs": 0,
                "durationMs": 10,
            }
        ],
    }


def _published_result(job: dict[str, object]) -> dict[str, object]:
    return {
        "sessionId": job["sessionId"],
        "revision": 1,
        "authority": "server_authoritative",
        "createdAtUtc": "2026-07-14T21:20:00Z",
        "captureManifestSha256": job["captureManifest"]["sha256"],
        "previousResultSha256": None,
        "status": "complete",
        "language": {"languageBcp47": "en", "confidence": None},
        "transcript": "Crash-safe private transcript.",
        "alignedWords": [],
        "modelProvenance": [
            {
                "modelId": "private-asr",
                "revision": "revision-1",
                "calibrationRevision": "asr-not-applicable",
            }
        ],
    }
