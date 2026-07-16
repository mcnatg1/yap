from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from yap_server.pools.batch_asr import (
    _MAX_WORKER_OUTPUT_BYTES,
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
    valid_worker_result,
)


class ContainerBatchAsrResultTests(unittest.TestCase):
    def setUp(self) -> None:
        self.lock = _test_lock()

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
                    stdout=json.dumps(valid_worker_result(self.lock)) + "\n",
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
            payload = valid_worker_result(self.lock)
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
            missing_audio = valid_worker_result(self.lock)
            missing_audio.pop("audio")
            payloads.append(("missing", missing_audio))
            mismatched_audio = valid_worker_result(self.lock)
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
