from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from yap_server.pools.batch_asr import BatchAsrJob, ContainerBatchAsrWorker

from .batch_asr_fixtures import (
    AUDIO_SHA256,
    CHECKED_HEAD,
    IMAGE_ID,
    STORAGE_NAMESPACE,
    test_lock as _test_lock,
)


class ContainerBatchAsrCommandTests(unittest.TestCase):
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

    def test_rejects_implicit_latest_image(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            for image in (
                "yap-asr",
                "yap-asr:latest",
                "yap-asr:phase4-0123456789abcdef",
            ):
                with self.subTest(image=image):
                    with self.assertRaises(ValueError):
                        ContainerBatchAsrWorker(
                            image=image,
                            model_dir=Path(directory),
                            lock=self.lock,
                            run_as_uid=1000,
                            run_as_gid=1000,
                            checked_head=CHECKED_HEAD,
                            storage_namespace=STORAGE_NAMESPACE,
                        )
