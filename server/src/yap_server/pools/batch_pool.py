from __future__ import annotations

from concurrent.futures import Future, ThreadPoolExecutor
import threading

from yap_server.pools.batch_contract import (
    BatchAsrJob,
    BatchWorker,
    DuplicatePoolJob,
    PoolBackpressure,
    PoolFenced,
    WorkerContainmentError,
)


class BatchAsrPool:
    """A bounded thread-backed pool for isolated batch-ASR workers."""

    def __init__(
        self,
        worker: BatchWorker,
        *,
        max_workers: int = 1,
        max_queued: int = 2,
    ) -> None:
        if max_workers < 1 or max_queued < 0:
            raise ValueError("pool limits are invalid")
        self._worker = worker
        self._slots = threading.BoundedSemaphore(max_workers + max_queued)
        self._lock = threading.Lock()
        self._outstanding: set[str] = set()
        self._cancellations: dict[str, threading.Event] = {}
        self._futures: dict[str, Future[dict[str, object]]] = {}
        self._fenced_reason: str | None = None
        self._executor = ThreadPoolExecutor(
            max_workers=max_workers,
            thread_name_prefix="yap-batch-asr",
        )

    @property
    def outstanding_count(self) -> int:
        with self._lock:
            return len(self._outstanding)

    @property
    def fenced(self) -> bool:
        with self._lock:
            return self._fenced_reason is not None

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        with self._lock:
            if self._fenced_reason is not None:
                raise PoolFenced(self._fenced_reason)
            if job.job_id in self._outstanding:
                raise DuplicatePoolJob(f"pool job {job.job_id!r} is already outstanding")
            if not self._slots.acquire(blocking=False):
                raise PoolBackpressure("batch ASR pool is at its bounded capacity")
            self._outstanding.add(job.job_id)
            cancellation = threading.Event()
            self._cancellations[job.job_id] = cancellation
        try:
            future = self._executor.submit(self._run_job, job, cancellation)
            with self._lock:
                self._futures[job.job_id] = future
        except BaseException:
            self._release(job.job_id)
            raise
        future.add_done_callback(lambda _future: self._release(job.job_id))
        return future

    def _run_job(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        try:
            return self._worker.run(job, cancellation)
        except WorkerContainmentError:
            with self._lock:
                self._fenced_reason = (
                    "batch ASR pool is fenced because container cleanup "
                    "could not be verified"
                )
            raise

    def cancel(self, job_id: str) -> bool:
        with self._lock:
            cancellation = self._cancellations.get(job_id)
            future = self._futures.get(job_id)
            if cancellation is None or future is None:
                return False
            cancellation.set()
        future.cancel()
        return True

    def _release(self, job_id: str) -> None:
        with self._lock:
            self._outstanding.discard(job_id)
            self._cancellations.pop(job_id, None)
            self._futures.pop(job_id, None)
            self._slots.release()

    def shutdown(self) -> None:
        close_worker = getattr(self._worker, "close", None)
        try:
            with self._lock:
                for cancellation in self._cancellations.values():
                    cancellation.set()
            if callable(close_worker):
                close_worker()
        finally:
            self._executor.shutdown(wait=True, cancel_futures=True)
