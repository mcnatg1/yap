import json
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from yap_server.pools.batch_asr import (
    _MAX_WORKER_OUTPUT_BYTES,
    _force_remove_container,
    _run_bounded_process,
    reconcile_owned_containers,
    BatchAsrJob,
    BatchAsrPool,
    ContainerBatchAsrWorker,
    DuplicatePoolJob,
    PoolBackpressure,
    PoolFenced,
    WorkerContainmentError,
    WorkerExecutionError,
)
from yap_server.pools.model_lock import LockedFixture, ModelPoolLock


REPO_ROOT = Path(__file__).resolve().parents[3]
IMAGE_ID = "sha256:" + "e" * 64
AUDIO_SHA256 = "f" * 64
CHECKED_HEAD = "a" * 40
STORAGE_NAMESPACE = "storage-test"


def _test_lock() -> ModelPoolLock:
    return ModelPoolLock(
        schema_version=1,
        runtime_image="registry.example/asr",
        runtime_source="https://example.invalid/runtime",
        runtime_license="Example runtime license",
        runtime_platform="linux/arm64",
        runtime_digest="sha256:" + "a" * 64,
        runtime_source_tag="26.06-py3",
        runtime_python_version="3.12",
        runtime_torch_version="2.13.0a0+example",
        runtime_cuda_version="13.3.0",
        runtime_torch_cuda_version="13.3",
        runtime_overlay_packages=(("transformers", "5.13.1"),),
        pool_id="cohere-batch",
        model_id="CohereLabs/cohere-transcribe-03-2026",
        model_revision="b" * 40,
        model_license="Apache-2.0",
        model_source="https://example.invalid/upstream",
        model_distribution_id="example/cohere-distribution",
        model_distribution_revision="c" * 40,
        model_distribution_source="https://example.invalid/distribution",
        model_distribution_provenance="verified test distribution",
        supported_languages=("en",),
        artifacts=(),
        fixture=LockedFixture(
            path="fixture.wav",
            source="https://example.invalid/fixture.wav",
            license="CC-BY-4.0",
            sha256="d" * 64,
            golden_transcript="fixture",
        ),
    )


def _valid_worker_result(lock: ModelPoolLock) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "jobId": "job-1",
        "model": {
            "poolId": lock.pool_id,
            "id": lock.model_id,
            "revision": lock.model_revision,
        },
        "audio": {
            "sha256": AUDIO_SHA256,
            "durationMs": 100,
            "sampleRateHz": 16000,
        },
        "transcript": {
            "text": "hello",
            "language": "en",
            "punctuation": True,
        },
        "runtime": {
            "device": "cuda",
            "pythonVersion": "3.12.9",
            "torchVersion": lock.runtime_torch_version,
            "torchCudaVersion": lock.runtime_torch_cuda_version,
            "overlayPackages": dict(lock.runtime_overlay_packages),
        },
    }


class _BlockingWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.release = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        _cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        self.release.wait(timeout=5)
        return {"schemaVersion": 1, "jobId": job.job_id}


class _ClosableWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.closed = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        _cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        self.closed.wait(timeout=0.25)
        return {"schemaVersion": 1, "jobId": job.job_id}

    def close(self) -> None:
        self.closed.set()


class _CancellationAwareWorker:
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
            raise AssertionError(f"job {job.job_id} was not cancelled")
        self.stopped.set()
        raise WorkerExecutionError("isolated ASR worker was cancelled")


class _ContainmentFailureWorker:
    def run(
        self,
        job: BatchAsrJob,
        _cancellation: threading.Event,
    ) -> dict[str, object]:
        raise WorkerContainmentError(
            f"container cleanup could not be verified for {job.job_id}"
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
        worker = _BlockingWorker()
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
        worker = _BlockingWorker()
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
        worker = _ClosableWorker()
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
        worker = _CancellationAwareWorker()
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
        worker = _BlockingWorker()
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
        pool = BatchAsrPool(_ContainmentFailureWorker(), max_workers=1, max_queued=0)
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


class ContainerBatchAsrWorkerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.lock = _test_lock()

    def test_runs_as_the_explicit_non_root_service_identity(self) -> None:
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
                run_as_gid=1001,
                checked_head=CHECKED_HEAD,
                storage_namespace=STORAGE_NAMESPACE,
            )

            command = worker.build_command(
                BatchAsrJob(
                    "job-1",
                    input_path,
                    root / "result.json",
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )

            self.assertIn("--user 1000:1001", " ".join(command))

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

    def test_rejects_a_root_or_non_numeric_service_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model_dir = Path(directory)
            for uid, gid in ((0, 1000), (1000, 0), (True, 1000)):
                with self.subTest(uid=uid, gid=gid):
                    with self.assertRaises(ValueError):
                        ContainerBatchAsrWorker(
                            image=IMAGE_ID,
                            model_dir=model_dir,
                            lock=self.lock,
                            run_as_uid=uid,
                            run_as_gid=gid,
                            checked_head=CHECKED_HEAD,
                            storage_namespace=STORAGE_NAMESPACE,
                        )

    def test_command_is_offline_read_only_non_root_and_capability_dropped(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_dir = root / "model"
            model_dir.mkdir()
            input_path = root / "speech.wav"
            input_path.write_bytes(b"wav")
            result_path = root / "result.json"
            worker = ContainerBatchAsrWorker(
                image=IMAGE_ID,
                model_dir=model_dir,
                lock=self.lock,
                run_as_uid=1000,
                run_as_gid=1000,
                checked_head=CHECKED_HEAD,
                storage_namespace=STORAGE_NAMESPACE,
            )

            command = worker.build_command(
                BatchAsrJob(
                    "job-1",
                    input_path,
                    result_path,
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )
            rendered = " ".join(command)

            self.assertRegex(rendered, r"--name yap-phase4-asr-[0-9a-f]{32}")
            self.assertIn("--network none", rendered)
            self.assertIn("--read-only", command)
            self.assertIn("--cap-drop ALL", rendered)
            self.assertIn("no-new-privileges", rendered)
            self.assertIn("--user 1000:1000", rendered)
            self.assertIn("--pull never", rendered)
            self.assertIn("--memory 96g", rendered)
            self.assertIn("--memory-swap 96g", rendered)
            self.assertIn("--cpus 16", rendered)
            self.assertIn("nvidia.com/gpu=all", rendered)
            self.assertIn("HF_HUB_OFFLINE=1", rendered)
            self.assertIn("TRANSFORMERS_OFFLINE=1", rendered)
            self.assertIn(
                "--tmpfs /tmp:rw,nosuid,nodev,noexec,size=1g",
                rendered,
            )
            self.assertIn(
                "--tmpfs /triton-cache:rw,nosuid,nodev,exec,size=256m,"
                "mode=0700,uid=1000,gid=1000",
                rendered,
            )
            self.assertIn("TRITON_CACHE_DIR=/triton-cache", rendered)
            self.assertIn("--language en", rendered)
            self.assertNotIn(str(result_path), rendered)

    def test_container_command_labels_checked_head_runtime_storage_and_job_owner(
        self,
    ) -> None:
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
                checked_head="a" * 40,
                storage_namespace="storage-a1b2c3",
                runtime_instance_id="c" * 32,
            )

            rendered = " ".join(
                worker.build_command(
                    BatchAsrJob(
                        "job-1",
                        input_path,
                        root / "result.json",
                        language="en",
                        input_sha256=AUDIO_SHA256,
                    )
                )
            )

            for label in (
                "com.mcnatg1.yap.owner=batch-asr",
                "com.mcnatg1.yap.storage=storage-a1b2c3",
                "com.mcnatg1.yap.runtime=" + "c" * 32,
                "com.mcnatg1.yap.job=job-1",
                "org.opencontainers.image.revision=" + "a" * 40,
            ):
                self.assertIn(f"--label {label}", rendered)

    def test_captures_validated_json_and_publishes_atomically(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_dir = root / "model"
            model_dir.mkdir()
            input_path = root / "speech.wav"
            input_path.write_bytes(b"wav")
            result_path = root / "result.json"

            def runner(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
                del args, kwargs
                return subprocess.CompletedProcess(
                    args=["docker"],
                    returncode=0,
                    stdout=json.dumps(_valid_worker_result(self.lock)) + "\n",
                    stderr="",
                )

            worker = ContainerBatchAsrWorker(
                image=IMAGE_ID,
                model_dir=model_dir,
                lock=self.lock,
                run_as_uid=1000,
                run_as_gid=1000,
                checked_head=CHECKED_HEAD,
                storage_namespace=STORAGE_NAMESPACE,
                runner=runner,
            )
            result = worker.run(
                BatchAsrJob(
                    "job-1",
                    input_path,
                    result_path,
                    language="en",
                    input_sha256=AUDIO_SHA256,
                )
            )

            self.assertEqual(result["jobId"], "job-1")
            self.assertEqual(json.loads(result_path.read_text(encoding="utf-8")), result)
            self.assertEqual(list(root.glob(".result.json.*.tmp")), [])

    def test_rejects_unlocked_overlay_package_versions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_dir = root / "model"
            model_dir.mkdir()
            input_path = root / "speech.wav"
            input_path.write_bytes(b"wav")
            result_path = root / "result.json"
            payload = _valid_worker_result(self.lock)
            runtime = dict(payload["runtime"])  # type: ignore[arg-type]
            runtime["overlayPackages"] = {"transformers": "5.12.0"}
            payload["runtime"] = runtime

            def runner(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
                del args, kwargs
                return subprocess.CompletedProcess(
                    args=["docker"],
                    returncode=0,
                    stdout=json.dumps(payload) + "\n",
                    stderr="",
                )

            worker = ContainerBatchAsrWorker(
                image=IMAGE_ID,
                model_dir=model_dir,
                lock=self.lock,
                run_as_uid=1000,
                run_as_gid=1000,
                checked_head=CHECKED_HEAD,
                storage_namespace=STORAGE_NAMESPACE,
                runner=runner,
            )

            with self.assertRaises(WorkerExecutionError):
                worker.run(
                    BatchAsrJob(
                        "job-1",
                        input_path,
                        result_path,
                        language="en",
                        input_sha256=AUDIO_SHA256,
                    )
                )
            self.assertFalse(result_path.exists())

    def test_rejects_worker_output_past_the_parent_memory_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_dir = root / "model"
            model_dir.mkdir()
            input_path = root / "speech.wav"
            input_path.write_bytes(b"wav")

            def runner(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
                del args, kwargs
                return subprocess.CompletedProcess(
                    args=["docker"],
                    returncode=0,
                    stdout="x" * (_MAX_WORKER_OUTPUT_BYTES + 1),
                    stderr="",
                )

            worker = ContainerBatchAsrWorker(
                image=IMAGE_ID,
                model_dir=model_dir,
                lock=self.lock,
                run_as_uid=1000,
                run_as_gid=1000,
                checked_head=CHECKED_HEAD,
                storage_namespace=STORAGE_NAMESPACE,
                runner=runner,
            )

            with self.assertRaisesRegex(WorkerExecutionError, "exceeded"):
                worker.run(
                    BatchAsrJob(
                        "job-1",
                        input_path,
                        root / "result.json",
                        language="en",
                        input_sha256=AUDIO_SHA256,
                    )
                )

    def test_rejects_missing_or_mismatched_audio_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_dir = root / "model"
            model_dir.mkdir()
            input_path = root / "speech.wav"
            input_path.write_bytes(b"wav")
            payloads = []
            missing_audio = _valid_worker_result(self.lock)
            missing_audio.pop("audio")
            payloads.append(("missing", missing_audio))
            mismatched_audio = _valid_worker_result(self.lock)
            audio = dict(mismatched_audio["audio"])  # type: ignore[arg-type]
            audio["sha256"] = "0" * 64
            mismatched_audio["audio"] = audio
            payloads.append(("mismatched", mismatched_audio))

            for case, payload in payloads:
                with self.subTest(case=case):
                    result_path = root / f"{case}.json"

                    def runner(
                        *args: object,
                        **kwargs: object,
                    ) -> subprocess.CompletedProcess[str]:
                        del args, kwargs
                        return subprocess.CompletedProcess(
                            args=["docker"],
                            returncode=0,
                            stdout=json.dumps(payload) + "\n",
                            stderr="",
                        )

                    worker = ContainerBatchAsrWorker(
                        image=IMAGE_ID,
                        model_dir=model_dir,
                        lock=self.lock,
                        run_as_uid=1000,
                        run_as_gid=1000,
                        checked_head=CHECKED_HEAD,
                        storage_namespace=STORAGE_NAMESPACE,
                        runner=runner,
                    )

                    with self.assertRaises(WorkerExecutionError):
                        worker.run(
                            BatchAsrJob(
                                "job-1",
                                input_path,
                                result_path,
                                language="en",
                                input_sha256=AUDIO_SHA256,
                            )
                        )
                    self.assertFalse(result_path.exists())

    def test_rejects_implicit_latest_image(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                ContainerBatchAsrWorker(
                    image="yap-asr",
                    model_dir=Path(directory),
                    lock=self.lock,
                    run_as_uid=1000,
                    run_as_gid=1000,
                    checked_head=CHECKED_HEAD,
                    storage_namespace=STORAGE_NAMESPACE,
                )
            with self.assertRaises(ValueError):
                ContainerBatchAsrWorker(
                    image="yap-asr:latest",
                    model_dir=Path(directory),
                    lock=self.lock,
                    run_as_uid=1000,
                    run_as_gid=1000,
                    checked_head=CHECKED_HEAD,
                    storage_namespace=STORAGE_NAMESPACE,
                )
            with self.assertRaises(ValueError):
                ContainerBatchAsrWorker(
                    image="yap-asr:phase4-0123456789abcdef",
                    model_dir=Path(directory),
                    lock=self.lock,
                    run_as_uid=1000,
                    run_as_gid=1000,
                    checked_head=CHECKED_HEAD,
                    storage_namespace=STORAGE_NAMESPACE,
                )


if __name__ == "__main__":
    unittest.main()
