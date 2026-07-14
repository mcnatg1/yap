from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import io
from importlib.metadata import version as package_version
import json
import os
from pathlib import Path
import re
import stat
import sys
import time
import wave

from yap_server.pools.model_lock import (
    ModelPoolLock,
    load_model_pool_lock,
    verify_model_artifacts,
)


SAMPLE_RATE_HZ = 16000
MAX_AUDIO_SECONDS = 4 * 60 * 60
_MAX_WAV_OVERHEAD_BYTES = 16 * 1024 * 1024
MAX_ENCODED_AUDIO_BYTES = (
    SAMPLE_RATE_HZ * MAX_AUDIO_SECONDS * 2 + _MAX_WAV_OVERHEAD_BYTES
)
_JOB_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")


class WorkerInputError(ValueError):
    """An input is outside the Phase 4 worker's bounded PCM contract."""


@dataclass(frozen=True)
class PcmAudio:
    pcm_bytes: bytes
    sample_rate: int
    frame_count: int
    duration_ms: int
    sha256: str


def validate_job_id(value: str) -> str:
    if not _JOB_ID.fullmatch(value):
        raise WorkerInputError("job id must be an opaque path-safe identifier")
    return value


def read_pcm16_wav(
    path: Path,
    *,
    max_audio_seconds: int = MAX_AUDIO_SECONDS,
) -> PcmAudio:
    if max_audio_seconds < 1:
        raise ValueError("max_audio_seconds must be positive")
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise WorkerInputError("input audio is missing") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise WorkerInputError("input audio must be a regular file")

    max_encoded_bytes = (
        SAMPLE_RATE_HZ * max_audio_seconds * 2 + _MAX_WAV_OVERHEAD_BYTES
    )
    if metadata.st_size > max_encoded_bytes:
        raise WorkerInputError("input audio exceeds the bounded encoded size")
    with path.open("rb") as encoded:
        encoded_bytes = encoded.read(max_encoded_bytes + 1)
    if len(encoded_bytes) > max_encoded_bytes:
        raise WorkerInputError("input audio exceeds the bounded encoded size")

    try:
        with wave.open(io.BytesIO(encoded_bytes), "rb") as source:
            if source.getnchannels() != 1:
                raise WorkerInputError("input audio must be mono")
            if source.getsampwidth() != 2:
                raise WorkerInputError("input audio must use signed PCM16 samples")
            if source.getframerate() != SAMPLE_RATE_HZ:
                raise WorkerInputError("input audio must be 16 kHz")
            if source.getcomptype() != "NONE":
                raise WorkerInputError("compressed WAV input is not supported")
            frame_count = source.getnframes()
            if frame_count < 1:
                raise WorkerInputError("input audio must contain at least one frame")
            if frame_count > SAMPLE_RATE_HZ * max_audio_seconds:
                raise WorkerInputError("input audio exceeds the bounded duration")
            pcm_bytes = source.readframes(frame_count)
    except (EOFError, wave.Error) as error:
        raise WorkerInputError("input audio is not a valid PCM WAV") from error
    if len(pcm_bytes) != frame_count * 2:
        raise WorkerInputError("input audio ended before its declared frame count")

    return PcmAudio(
        pcm_bytes=pcm_bytes,
        sample_rate=SAMPLE_RATE_HZ,
        frame_count=frame_count,
        duration_ms=max(1, round(frame_count * 1000 / SAMPLE_RATE_HZ)),
        sha256=hashlib.sha256(encoded_bytes).hexdigest(),
    )


def _decoded_text(value: object) -> str:
    if isinstance(value, str):
        text = value
    elif isinstance(value, (list, tuple)) and len(value) == 1 and isinstance(value[0], str):
        text = value[0]
    else:
        raise RuntimeError("ASR decoder returned an unexpected result")
    text = " ".join(text.split())
    if not text:
        raise RuntimeError("ASR decoder returned an empty transcript")
    return text


def transcribe(
    *,
    job_id: str,
    model_dir: Path,
    lock: ModelPoolLock,
    audio: PcmAudio,
    language: str,
    punctuation: bool,
) -> dict[str, object]:
    # These imports stay inside the isolated worker. Importing the health service
    # or queue/router code never loads CUDA, Torch, NumPy, or Transformers.
    import numpy as np
    import torch
    from transformers import AutoProcessor, CohereAsrForConditionalGeneration

    if not torch.cuda.is_available():
        raise RuntimeError("the Phase 4 ASR worker requires an NVIDIA GPU")

    load_started = time.monotonic()
    processor = AutoProcessor.from_pretrained(
        str(model_dir),
        local_files_only=True,
    )
    model = CohereAsrForConditionalGeneration.from_pretrained(
        str(model_dir),
        local_files_only=True,
        dtype=torch.bfloat16,
    )
    model.to("cuda")
    model.eval()
    torch.cuda.synchronize()
    model_load_ms = round((time.monotonic() - load_started) * 1000)

    samples = np.frombuffer(audio.pcm_bytes, dtype="<i2").astype(np.float32)
    samples /= 32768.0
    inputs = processor(
        audio=samples,
        sampling_rate=audio.sample_rate,
        return_tensors="pt",
        language=language,
        punctuation=punctuation,
    )
    audio_chunk_index = inputs.get("audio_chunk_index")
    inputs = inputs.to(device=model.device, dtype=model.dtype)

    inference_started = time.monotonic()
    with torch.inference_mode():
        output = model.generate(**inputs, max_new_tokens=256)
    torch.cuda.synchronize()
    inference_ms = round((time.monotonic() - inference_started) * 1000)
    transcript = _decoded_text(
        processor.decode(
            output,
            skip_special_tokens=True,
            audio_chunk_index=audio_chunk_index,
            language=language,
        )
    )

    return {
        "schemaVersion": 1,
        "jobId": job_id,
        "model": {
            "poolId": lock.pool_id,
            "id": lock.model_id,
            "revision": lock.model_revision,
        },
        "audio": {
            "sha256": audio.sha256,
            "durationMs": audio.duration_ms,
            "sampleRateHz": audio.sample_rate,
        },
        "transcript": {
            "text": transcript,
            "language": language,
            "punctuation": punctuation,
        },
        "runtime": {
            "device": "cuda",
            "deviceName": str(torch.cuda.get_device_name(0)),
            "computeCapability": list(torch.cuda.get_device_capability(0)),
            "pythonVersion": sys.version.split()[0],
            "torchVersion": str(torch.__version__),
            "torchCudaVersion": str(torch.version.cuda),
            "overlayPackages": {
                name: package_version(name)
                for name, _expected_version in lock.runtime_overlay_packages
            },
            "dtype": str(model.dtype).removeprefix("torch."),
            "modelLoadMs": model_load_ms,
            "inferenceMs": inference_ms,
        },
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run one offline Phase 4 batch-ASR job")
    parser.add_argument("--lock", default=os.environ.get("YAP_MODEL_LOCK"))
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--input", required=True)
    parser.add_argument("--job-id", required=True)
    parser.add_argument("--language", required=True)
    parser.add_argument("--no-punctuation", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    try:
        if not arguments.lock:
            raise WorkerInputError("a model lock is required")
        job_id = validate_job_id(arguments.job_id)
        lock = load_model_pool_lock(Path(arguments.lock))
        if arguments.language not in lock.supported_languages:
            raise WorkerInputError("language is not supported by the locked model")
        model_dir = Path(arguments.model_dir).resolve(strict=True)
        verify_model_artifacts(lock, model_dir)
        audio = read_pcm16_wav(Path(arguments.input).resolve(strict=True))
        result = transcribe(
            job_id=job_id,
            model_dir=model_dir,
            lock=lock,
            audio=audio,
            language=arguments.language,
            punctuation=not arguments.no_punctuation,
        )
    except (OSError, ValueError) as error:
        payload = {
            "schemaVersion": 1,
            "code": "WORKER_INPUT_INVALID",
            "message": str(error),
        }
        print(json.dumps(payload, ensure_ascii=True, separators=(",", ":")), file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=True, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
