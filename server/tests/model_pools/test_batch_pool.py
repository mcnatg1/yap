from __future__ import annotations

import unittest
from pathlib import Path

from yap_server.pools.batch_asr import (
    BatchAsrJob,
    BatchAsrPool,
    DuplicatePoolJob,
    PoolBackpressure,
    PoolFenced,
    WorkerContainmentError,
    WorkerExecutionError,
)

from .batch_asr_fixtures import (
    AUDIO_SHA256,
    BlockingWorker,
    CancellationAwareWorker,
    ClosableWorker,
    ContainmentFailureWorker,
)


class BatchAsrPoolTests(unittest.TestCase):
    def test_batch_job_requires_an_explicit_iso_language(self) -> None:
        job = BatchAsrJob(
            "job-1",
            Path("one.wav"),
            Path("one.json"),
            language="en",
            input_sha256=AUDIO_SHA256,
        )

        self.assertEqual(job.language, "en")
        for invalid in ("", "auto", "EN", "eng", "../en"):
            with self.subTest(invalid=invalid):
                with self.assertRaises(ValueError):
                    BatchAsrJob(
                        "job-1",
                        Path("one.wav"),
                        Path("one.json"),
                        language=invalid,
                        input_sha256=AUDIO_SHA256,
                    )

    def test_pool_bounds_running_and_queued_work(self) -> None:
        worker = BlockingWorker()
        pool = BatchAsrPool(worker, max_workers=1, max_queued=1)
        try:
            first = pool.submit(
                BatchAsrJob(
                    "job-1",
                    Path("one.wav"),
                    Path("one.json"),
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )
            self.assertTrue(worker.started.wait(timeout=2))
            second = pool.submit(
                BatchAsrJob(
                    "job-2",
                    Path("two.wav"),
                    Path("two.json"),
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )

            with self.assertRaises(PoolBackpressure):
                pool.submit(
                    BatchAsrJob(
                        "job-3",
                        Path("three.wav"),
                        Path("three.json"),
                        language="en",
                        input_sha256=AUDIO_SHA256,
                    )
                )

            worker.release.set()
            self.assertEqual(first.result(timeout=2)["jobId"], "job-1")
            self.assertEqual(second.result(timeout=2)["jobId"], "job-2")
        finally:
            worker.release.set()
            pool.shutdown()

    def test_pool_rejects_duplicate_outstanding_job(self) -> None:
        worker = BlockingWorker()
        pool = BatchAsrPool(worker, max_workers=1, max_queued=1)
        try:
            job = BatchAsrJob(
                "job-1",
                Path("one.wav"),
                Path("one.json"),
                language="en",
                input_sha256=AUDIO_SHA256,
            )
            future = pool.submit(job)
            self.assertTrue(worker.started.wait(timeout=2))
            with self.assertRaises(DuplicatePoolJob):
                pool.submit(job)
            worker.release.set()
            future.result(timeout=2)
        finally:
            worker.release.set()
            pool.shutdown()

    def test_pool_shutdown_stops_the_worker_before_waiting_for_threads(self) -> None:
        worker = ClosableWorker()
        pool = BatchAsrPool(worker, max_workers=1, max_queued=0)
        pool.submit(
            BatchAsrJob(
                "job-1",
                Path("one.wav"),
                Path("one.json"),
                language="en",
                input_sha256=AUDIO_SHA256,
            )
        )
        self.assertTrue(worker.started.wait(timeout=2))

        pool.shutdown()

        self.assertTrue(worker.closed.is_set())

    def test_pool_cancels_one_running_job_without_stopping_the_worker(self) -> None:
        worker = CancellationAwareWorker()
        pool = BatchAsrPool(worker, max_workers=1, max_queued=0)
        try:
            future = pool.submit(
                BatchAsrJob(
                    "job-1",
                    Path("one.wav"),
                    Path("one.json"),
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )
            self.assertTrue(worker.started.wait(timeout=2))

            self.assertTrue(pool.cancel("job-1"))

            with self.assertRaisesRegex(WorkerExecutionError, "cancelled"):
                future.result(timeout=2)
            self.assertTrue(worker.stopped.is_set())
            self.assertEqual(pool.outstanding_count, 0)
            self.assertFalse(pool.cancel("job-1"))
        finally:
            pool.shutdown()

    def test_pool_cancels_queued_work_without_deadlocking_its_completion_callback(
        self,
    ) -> None:
        worker = BlockingWorker()
        pool = BatchAsrPool(worker, max_workers=1, max_queued=1)
        try:
            running = pool.submit(
                BatchAsrJob(
                    "job-1",
                    Path("one.wav"),
                    Path("one.json"),
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )
            self.assertTrue(worker.started.wait(timeout=2))
            queued = pool.submit(
                BatchAsrJob(
                    "job-2",
                    Path("two.wav"),
                    Path("two.json"),
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )

            self.assertTrue(pool.cancel("job-2"))

            self.assertTrue(queued.cancelled())
            self.assertEqual(pool.outstanding_count, 1)
            worker.release.set()
            running.result(timeout=2)
            self.assertEqual(pool.outstanding_count, 0)
        finally:
            worker.release.set()
            pool.shutdown()

    def test_pool_fences_new_work_after_unverified_container_cleanup(self) -> None:
        pool = BatchAsrPool(ContainmentFailureWorker(), max_workers=1, max_queued=0)
        job = BatchAsrJob(
            "job-1",
            Path("one.wav"),
            Path("one.json"),
            language="en",
            input_sha256=AUDIO_SHA256,
        )
        try:
            with self.assertRaises(WorkerContainmentError):
                pool.submit(job).result(timeout=2)

            with self.assertRaisesRegex(PoolFenced, "cleanup"):
                pool.submit(
                    BatchAsrJob(
                        "job-2",
                        Path("two.wav"),
                        Path("two.json"),
                        language="en",
                        input_sha256=AUDIO_SHA256,
                    )
                )
            self.assertTrue(pool.fenced)
            self.assertEqual(pool.outstanding_count, 0)
        finally:
            pool.shutdown()
