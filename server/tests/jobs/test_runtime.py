from __future__ import annotations

import unittest
from concurrent.futures import Future
import os
from pathlib import Path
import tempfile
from unittest.mock import patch

import yap_server.__main__ as server_main
from yap_server.config import ServerSettings
from yap_server.jobs.runtime import (
    RoutedBatchProcessor,
    StorageRuntimeLease,
    ensure_development_batch_bind,
    resolve_phase5_worker_image,
)
from yap_server.pools.batch_asr import BatchAsrJob, WorkerContainmentError
from yap_server.workload_router import WorkloadRouter


class _Pool:
    def __init__(self) -> None:
        self.jobs: list[BatchAsrJob] = []
        self.future: Future[dict[str, object]] = Future()

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        self.jobs.append(job)
        return self.future

    def cancel(self, job_id: str) -> bool:
        return bool(self.jobs and self.jobs[-1].job_id == job_id)


class RoutedBatchProcessorTests(unittest.TestCase):
    def test_submit_routes_batch_work_before_entering_the_isolated_pool(self) -> None:
        pool = _Pool()
        router = WorkloadRouter(
            max_pending=3,
            max_pending_per_owner=3,
            max_consecutive_live=8,
        )
        processor = RoutedBatchProcessor(
            router=router,
            pool=pool,
            owner_key="development-loopback",
        )
        job = BatchAsrJob(
            job_id="job-phase5-runtime",
            input_path=Path("input.wav"),
            result_path=Path("result.json"),
            language="en",
            input_sha256="a" * 64,
        )

        returned = processor.submit(job)

        self.assertIs(returned, pool.future)
        self.assertEqual(pool.jobs, [job])
        self.assertEqual(router.pending_count, 0)

    def test_cancel_routes_to_the_same_isolated_pool_owner(self) -> None:
        pool = _Pool()
        processor = RoutedBatchProcessor(
            router=WorkloadRouter(
                max_pending=3,
                max_pending_per_owner=3,
                max_consecutive_live=8,
            ),
            pool=pool,
            owner_key="development-loopback",
        )
        job = BatchAsrJob(
            job_id="job-phase5-cancel",
            input_path=Path("input.wav"),
            result_path=Path("result.json"),
            language="en",
            input_sha256="a" * 64,
        )
        processor.submit(job)

        self.assertTrue(processor.cancel(job.job_id))

    @unittest.skipUnless(os.name == "posix", "POSIX storage lease")
    def test_storage_runtime_lease_excludes_a_second_server_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            storage = Path(temporary)
            first = StorageRuntimeLease(storage)
            try:
                with self.assertRaisesRegex(ValueError, "already owned"):
                    StorageRuntimeLease(storage)
            finally:
                first.close()

            replacement = StorageRuntimeLease(storage)
            replacement.close()

    def test_unauthenticated_batch_runtime_is_loopback_only(self) -> None:
        ensure_development_batch_bind("127.0.0.1")
        ensure_development_batch_bind("::1")
        for host in ("localhost", "0.0.0.0", "192.168.50.1", "yap.internal"):
            with self.subTest(host=host):
                with self.assertRaisesRegex(ValueError, "SSH tunnel"):
                    ensure_development_batch_bind(host)

    def test_phase5_runtime_uses_the_inspected_checked_head_worker_image(self) -> None:
        checked_head = "a" * 40
        image_id = "sha256:" + "b" * 64
        environ = {
            "YAP_PHASE5_WORKER_IMAGE": f"yap-phase5-asr:phase5-{checked_head}",
            "YAP_PHASE5_CHECKED_HEAD": checked_head,
        }

        with patch(
            "yap_server.jobs.runtime.inspect_worker_image",
            return_value={"id": image_id},
        ) as inspect:
            resolved = resolve_phase5_worker_image(
                environ,
                docker_binary="docker-test",
            )

        self.assertEqual(resolved, image_id)
        inspect.assert_called_once_with(
            environ["YAP_PHASE5_WORKER_IMAGE"],
            checked_head,
            docker_binary="docker-test",
        )

    def test_phase5_runtime_requires_image_and_checked_head(self) -> None:
        for environ in (
            {},
            {"YAP_PHASE5_WORKER_IMAGE": "yap-phase5-asr:test"},
            {
                "YAP_PHASE5_WORKER_IMAGE": "yap-phase5-asr:test",
                "YAP_PHASE5_CHECKED_HEAD": "not-a-commit",
            },
        ):
            with self.subTest(environ=environ):
                with self.assertRaises(ValueError):
                    resolve_phase5_worker_image(environ, docker_binary="docker")


class ServerMainTests(unittest.TestCase):
    def test_linux_termination_uses_the_graceful_runtime_cleanup_path(self) -> None:
        with (
            patch.object(server_main.signal, "signal") as install_signal,
            patch.object(
                server_main.ServerSettings,
                "from_env",
                return_value=ServerSettings(),
            ),
            patch.object(server_main, "build_batch_runtime", return_value=None),
            patch.object(server_main, "serve", side_effect=KeyboardInterrupt),
        ):
            server_main.main()

        install_signal.assert_any_call(
            server_main.signal.SIGTERM,
            server_main._raise_keyboard_interrupt,
        )

    def test_startup_storage_failure_does_not_expose_private_paths(self) -> None:
        private_path = "C:/private/recordings/patient-audio.wav"
        with (
            patch.object(server_main.signal, "signal"),
            patch.object(
                server_main.ServerSettings,
                "from_env",
                return_value=ServerSettings(),
            ),
            patch.object(
                server_main,
                "build_batch_runtime",
                side_effect=OSError(private_path),
            ),
        ):
            with self.assertRaises(SystemExit) as stopped:
                server_main.main()

        self.assertEqual(str(stopped.exception), "Yap private server startup failed.")
        self.assertNotIn(private_path, str(stopped.exception))
        self.assertIsNone(stopped.exception.__cause__)
        self.assertTrue(stopped.exception.__suppress_context__)

    def test_startup_containment_failure_uses_the_generic_boundary(self) -> None:
        with (
            patch.object(server_main.signal, "signal"),
            patch.object(
                server_main.ServerSettings,
                "from_env",
                return_value=ServerSettings(),
            ),
            patch.object(
                server_main,
                "build_batch_runtime",
                side_effect=WorkerContainmentError("private container detail"),
            ),
        ):
            with self.assertRaises(SystemExit) as stopped:
                server_main.main()

        self.assertEqual(str(stopped.exception), "Yap private server startup failed.")
        self.assertNotIn("private container detail", str(stopped.exception))
        self.assertIsNone(stopped.exception.__cause__)
        self.assertTrue(stopped.exception.__suppress_context__)

    def test_serving_storage_failure_does_not_expose_private_paths(self) -> None:
        private_path = "/srv/yap/private/patient-audio.wav"
        with (
            patch.object(server_main.signal, "signal"),
            patch.object(
                server_main.ServerSettings,
                "from_env",
                return_value=ServerSettings(),
            ),
            patch.object(server_main, "build_batch_runtime", return_value=None),
            patch.object(server_main, "serve", side_effect=OSError(private_path)),
        ):
            with self.assertRaises(SystemExit) as stopped:
                server_main.main()

        self.assertEqual(
            str(stopped.exception),
            "Yap private server runtime became unavailable.",
        )
        self.assertNotIn(private_path, str(stopped.exception))
        self.assertIsNone(stopped.exception.__cause__)
        self.assertTrue(stopped.exception.__suppress_context__)


if __name__ == "__main__":
    unittest.main()
