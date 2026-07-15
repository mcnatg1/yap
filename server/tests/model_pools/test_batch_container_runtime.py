from __future__ import annotations

import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from yap_server.pools.batch_asr import (
    _force_remove_container,
    _run_bounded_process,
    reconcile_owned_containers,
    BatchAsrJob,
    ContainerBatchAsrWorker,
    WorkerExecutionError,
)

from .batch_asr_fixtures import (
    AUDIO_SHA256,
    CHECKED_HEAD,
    IMAGE_ID,
    STORAGE_NAMESPACE,
    test_lock as _test_lock,
)


class ContainerBatchAsrRuntimeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.lock = _test_lock()

    def test_default_process_runner_stops_at_the_output_limit(self) -> None:
        with self.assertRaisesRegex(WorkerExecutionError, "exceeded"):
            _run_bounded_process(
                [sys.executable, "-c", "print('x' * 4096)"],
                timeout_seconds=5,
                output_limit_bytes=1024,
            )

    def test_default_process_runner_honors_shutdown_cancellation(self) -> None:
        cancelled = threading.Event()
        trigger = threading.Timer(0.1, cancelled.set)
        trigger.start()
        try:
            with self.assertRaisesRegex(WorkerExecutionError, "cancelled"):
                _run_bounded_process(
                    [sys.executable, "-c", "import time; time.sleep(30)"],
                    timeout_seconds=5,
                    output_limit_bytes=1024,
                    cancellation=cancelled,
                )
        finally:
            trigger.join(timeout=1)

    def test_default_runner_force_removes_the_named_container_after_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_dir = root / "model"
            model_dir.mkdir()
            input_path = root / "speech.wav"
            input_path.write_bytes(b"wav")
            worker = ContainerBatchAsrWorker(
                image=IMAGE_ID,
                model_dir=model_dir,
                lock=self.lock,
                run_as_uid=1000,
                run_as_gid=1000,
                checked_head=CHECKED_HEAD,
                storage_namespace=STORAGE_NAMESPACE,
            )
            job = BatchAsrJob(
                "job-1",
                input_path,
                root / "result.json",
                language="en",
                input_sha256=AUDIO_SHA256,
            )

            with (
                patch(
                    "yap_server.pools.batch_asr._run_bounded_process",
                    side_effect=WorkerExecutionError("isolated ASR worker timed out"),
                ),
                patch("yap_server.pools.batch_asr._force_remove_container") as remove,
            ):
                with self.assertRaisesRegex(WorkerExecutionError, "timed out"):
                    worker.run(job)

            remove.assert_called_once()
            docker_binary, container_name = remove.call_args.args
            self.assertEqual(docker_binary, "docker")
            self.assertRegex(container_name, r"^yap-phase4-asr-[0-9a-f]{32}$")

    def test_container_cleanup_requires_removal_or_verified_absence(self) -> None:
        def missing_runner(
            *args: object,
            **kwargs: object,
        ) -> subprocess.CompletedProcess[str]:
            del args, kwargs
            return subprocess.CompletedProcess(
                args=["docker"],
                returncode=1,
                stdout="",
                stderr="Error response from daemon: No such container: worker",
            )

        _force_remove_container(
            "docker",
            "yap-phase4-asr-" + "a" * 32,
            runner=missing_runner,
        )

        def denied_runner(
            *args: object,
            **kwargs: object,
        ) -> subprocess.CompletedProcess[str]:
            del args, kwargs
            return subprocess.CompletedProcess(
                args=["docker"],
                returncode=1,
                stdout="",
                stderr="permission denied",
            )

        with self.assertRaisesRegex(WorkerExecutionError, "could not remove"):
            _force_remove_container(
                "docker",
                "yap-phase4-asr-" + "a" * 32,
                runner=denied_runner,
            )

    def test_startup_reconciles_only_owned_containers_in_the_storage_namespace(
        self,
    ) -> None:
        calls: list[list[str]] = []

        def runner(
            command: list[str],
            **_kwargs: object,
        ) -> subprocess.CompletedProcess[str]:
            calls.append(command)
            if command[1:3] == ["container", "ls"]:
                return subprocess.CompletedProcess(
                    args=command,
                    returncode=0,
                    stdout="a" * 64 + "\n" + "b" * 64 + "\n",
                    stderr="",
                )
            return subprocess.CompletedProcess(
                args=command,
                returncode=0,
                stdout="",
                stderr="",
            )

        removed = reconcile_owned_containers(
            "docker-test",
            storage_namespace="storage-a1b2c3",
            runner=runner,
        )

        self.assertEqual(removed, 2)
        self.assertEqual(
            calls[0],
            [
                "docker-test",
                "container",
                "ls",
                "--all",
                "--quiet",
                "--filter",
                "label=com.mcnatg1.yap.owner=batch-asr",
                "--filter",
                "label=com.mcnatg1.yap.storage=storage-a1b2c3",
            ],
        )
        self.assertEqual(
            calls[1:],
            [
                ["docker-test", "container", "rm", "--force", "a" * 64],
                ["docker-test", "container", "rm", "--force", "b" * 64],
            ],
        )
