import hashlib
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path
from unittest.mock import patch

from yap_server.pools.batch_asr_worker import (
    MAX_ENCODED_AUDIO_BYTES,
    WorkerInputError,
    read_pcm16_wav,
    validate_job_id,
)


REPO_ROOT = Path(__file__).resolve().parents[3]


def _write_wav(
    path: Path,
    *,
    channels: int = 1,
    sample_rate: int = 16000,
    sample: bytes = b"\x00\x00",
    frame_count: int = 1600,
) -> bytes:
    frames = sample * frame_count * channels
    with wave.open(str(path), "wb") as output:
        output.setnchannels(channels)
        output.setsampwidth(2)
        output.setframerate(sample_rate)
        output.writeframes(frames)
    return path.read_bytes()


class BatchAsrWorkerTests(unittest.TestCase):
    def test_worker_module_does_not_import_gpu_stack_on_health_process_import(self) -> None:
        script = (
            "import sys; import yap_server.pools.batch_asr_worker; "
            "assert 'torch' not in sys.modules; "
            "assert 'transformers' not in sys.modules; "
            "assert 'numpy' not in sys.modules"
        )
        completed = subprocess.run(
            [sys.executable, "-c", script],
            cwd=REPO_ROOT,
            env={**dict(__import__("os").environ), "PYTHONPATH": "server/src"},
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_reads_only_bounded_mono_16khz_pcm16_wav(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "speech.wav"
            encoded = _write_wav(path)

            audio = read_pcm16_wav(path)

            self.assertEqual(audio.sample_rate, 16000)
            self.assertEqual(audio.frame_count, 1600)
            self.assertEqual(audio.duration_ms, 100)
            self.assertEqual(audio.sha256, hashlib.sha256(encoded).hexdigest())
            self.assertEqual(len(audio.pcm_bytes), 3200)

    def test_nonempty_audio_has_a_positive_attested_duration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "one-frame.wav"
            _write_wav(path, frame_count=1)

            audio = read_pcm16_wav(path)

            self.assertEqual(audio.frame_count, 1)
            self.assertEqual(audio.duration_ms, 1)

    def test_rejects_stereo_and_wrong_sample_rate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            stereo = Path(directory) / "stereo.wav"
            wrong_rate = Path(directory) / "wrong-rate.wav"
            _write_wav(stereo, channels=2)
            _write_wav(wrong_rate, sample_rate=8000)

            with self.assertRaises(WorkerInputError):
                read_pcm16_wav(stereo)
            with self.assertRaises(WorkerInputError):
                read_pcm16_wav(wrong_rate)

    def test_rejects_encoded_audio_larger_than_the_worker_contract(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "oversized.wav"
            path.write_bytes(b"RIFF")
            with path.open("r+b") as oversized:
                oversized.truncate(MAX_ENCODED_AUDIO_BYTES + 1)

            with self.assertRaises(WorkerInputError):
                read_pcm16_wav(path)

    def test_digest_and_pcm_come_from_one_encoded_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "speech.wav"
            original = _write_wav(path, sample=b"\x00\x00")
            replacement_path = Path(directory) / "replacement.wav"
            replacement = _write_wav(replacement_path, sample=b"\x01\x00")
            real_wave_open = wave.open

            def replace_path_then_open(file: object, mode: str) -> wave.Wave_read:
                path.write_bytes(replacement)
                return real_wave_open(file, mode)

            with patch(
                "yap_server.pools.batch_asr_worker.wave.open",
                side_effect=replace_path_then_open,
            ):
                audio = read_pcm16_wav(path)

            self.assertEqual(audio.sha256, hashlib.sha256(original).hexdigest())
            self.assertEqual(audio.pcm_bytes, b"\x00\x00" * 1600)

    def test_job_ids_are_opaque_and_path_safe(self) -> None:
        self.assertEqual(validate_job_id("job-01_a.b"), "job-01_a.b")
        for invalid in ("", "../job", "job/id", " job", "x" * 129):
            with self.subTest(invalid=invalid):
                with self.assertRaises(WorkerInputError):
                    validate_job_id(invalid)


if __name__ == "__main__":
    unittest.main()
