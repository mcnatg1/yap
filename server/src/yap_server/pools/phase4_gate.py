from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from typing import Callable

from yap_server.pools.batch_asr import (
    BatchAsrJob,
    BatchAsrPool,
    ContainerBatchAsrWorker,
)
from yap_server.pools.model_lock import (
    load_model_pool_lock,
    verify_fixture,
    verify_model_artifacts,
)
from yap_server.workload_router import WorkloadRequest, WorkloadRouter


_GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
_PHASE4_DEVICE_NAME = "NVIDIA GB10"
_PHASE4_COMPUTE_CAPABILITY = [12, 1]
_PHASE4_DTYPE = "bfloat16"


def inspect_container_image(
    image: str,
    checked_head: str,
    *,
    docker_binary: str = "docker",
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> dict[str, object]:
    completed = runner(
        [docker_binary, "image", "inspect", image],
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
        stdin=subprocess.DEVNULL,
    )
    if completed.returncode != 0:
        raise RuntimeError("could not inspect the Phase 4 worker image")
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("worker image inspection returned invalid JSON") from error
    if not isinstance(payload, list) or len(payload) != 1 or not isinstance(payload[0], dict):
        raise RuntimeError("worker image inspection returned an unexpected shape")
    record = payload[0]
    config = record.get("Config")
    labels = config.get("Labels") if isinstance(config, dict) else None
    if not isinstance(labels, dict) or labels.get("org.opencontainers.image.revision") != checked_head:
        raise RuntimeError("worker image revision label does not match the checked head")
    image_id = record.get("Id")
    architecture = record.get("Architecture")
    repo_digests = record.get("RepoDigests")
    if not isinstance(image_id, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
        raise RuntimeError("worker image ID is invalid")
    if architecture != "arm64":
        raise RuntimeError("worker image architecture is not ARM64")
    if repo_digests is None:
        repo_digests = []
    if not isinstance(repo_digests, list) or not all(
        isinstance(item, str) for item in repo_digests
    ):
        raise RuntimeError("worker image repository digests are invalid")
    return {
        "reference": image,
        "id": image_id,
        "architecture": architecture,
        "repoDigests": repo_digests,
        "revision": checked_head,
    }


def normalized_words(value: str) -> list[str]:
    return re.findall(r"[\w']+", value.casefold(), flags=re.UNICODE)


def word_error_rate(reference: str, hypothesis: str) -> float:
    expected = normalized_words(reference)
    actual = normalized_words(hypothesis)
    if not expected:
        raise ValueError("WER reference cannot be empty")
    previous = list(range(len(actual) + 1))
    for expected_word in expected:
        current = [previous[0] + 1]
        for index, actual_word in enumerate(actual, start=1):
            current.append(
                min(
                    current[-1] + 1,
                    previous[index] + 1,
                    previous[index - 1] + (expected_word != actual_word),
                )
            )
        previous = current
    return previous[-1] / len(expected)


def validate_gb10_runtime(runtime: object) -> None:
    if not isinstance(runtime, dict) or runtime.get("device") != "cuda":
        raise RuntimeError("worker result did not attest CUDA execution")
    if runtime.get("deviceName") != _PHASE4_DEVICE_NAME:
        raise RuntimeError("worker result did not attest the Phase 4 GB10 device")
    if runtime.get("computeCapability") != _PHASE4_COMPUTE_CAPABILITY:
        raise RuntimeError(
            "worker result did not attest GB10 compute capability 12.1"
        )
    if runtime.get("dtype") != _PHASE4_DTYPE:
        raise RuntimeError("worker result did not attest BF16 model execution")


def run_gate(
    *,
    checked_head: str,
    image: str,
    lock_path: Path,
    model_dir: Path,
    repo_root: Path,
    result_path: Path,
    evidence_path: Path,
    max_wer: float,
) -> dict[str, object]:
    if not _GIT_SHA.fullmatch(checked_head):
        raise ValueError("checked head must be a full lowercase Git SHA")
    if not 0 <= max_wer <= 1:
        raise ValueError("max WER must be between zero and one")

    lock = load_model_pool_lock(lock_path)
    verify_model_artifacts(lock, model_dir)
    fixture = verify_fixture(lock, repo_root)
    container = inspect_container_image(image, checked_head)
    inspected_image_id = container.get("id")
    if not isinstance(inspected_image_id, str):
        raise RuntimeError("worker image inspection did not return an immutable image ID")
    getuid = getattr(os, "getuid", None)
    getgid = getattr(os, "getgid", None)
    if not callable(getuid) or not callable(getgid):
        raise RuntimeError("the Phase 4 container gate requires a POSIX service identity")
    worker = ContainerBatchAsrWorker(
        image=inspected_image_id,
        model_dir=model_dir,
        lock=lock,
        run_as_uid=getuid(),
        run_as_gid=getgid(),
    )
    router = WorkloadRouter(max_pending=4, max_pending_per_owner=2)
    request = WorkloadRequest("phase4-asr-gate", "phase4-gate", "batch")
    router.enqueue(request)
    dispatched = router.dispatch(available_targets={"batch-asr"})
    if dispatched is None or dispatched.request != request:
        raise RuntimeError("batch workload did not dispatch to the reference pool")

    pool = BatchAsrPool(worker, max_workers=1, max_queued=0)
    try:
        result = pool.submit(
            BatchAsrJob(
                request.job_id,
                fixture,
                result_path,
                language="en",
                input_sha256=lock.fixture.sha256,
            )
        ).result(timeout=35 * 60)
    finally:
        pool.shutdown()

    model = result.get("model")
    runtime = result.get("runtime")
    transcript = result.get("transcript")
    if not isinstance(model, dict) or (
        model.get("id") != lock.model_id
        or model.get("revision") != lock.model_revision
        or model.get("poolId") != lock.pool_id
    ):
        raise RuntimeError("worker result did not attest the locked model")
    validate_gb10_runtime(runtime)
    if not isinstance(transcript, dict) or not isinstance(transcript.get("text"), str):
        raise RuntimeError("worker result did not contain a transcript")
    measured_wer = word_error_rate(lock.fixture.golden_transcript, transcript["text"])
    if measured_wer > max_wer:
        raise RuntimeError(
            f"fixture WER {measured_wer:.4f} exceeds the {max_wer:.4f} gate"
        )

    result_digest = hashlib.sha256(result_path.read_bytes()).hexdigest()
    evidence: dict[str, object] = {
        "schemaVersion": 1,
        "phase": 4,
        "checkedHead": checked_head,
        "container": container,
        "model": model,
        "fixture": {
            "sha256": lock.fixture.sha256,
            "license": lock.fixture.license,
        },
        "wordErrorRate": measured_wer,
        "maximumWordErrorRate": max_wer,
        "resultSha256": result_digest,
        "runtime": runtime,
        "boundary": {
            "network": "none",
            "workerCount": 1,
            "hostObservation": "pending-wrapper-read-back",
        },
    }
    _write_json_atomic(evidence_path, evidence)
    return evidence


def _write_json_atomic(path: Path, payload: dict[str, object]) -> None:
    destination = path.resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            prefix=f".{destination.name}.",
            suffix=".tmp",
            dir=destination.parent,
            delete=False,
        ) as temporary:
            temporary_name = temporary.name
            json.dump(payload, temporary, ensure_ascii=True, indent=2, sort_keys=True)
            temporary.write("\n")
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_name, destination)
        temporary_name = None
    finally:
        if temporary_name is not None:
            Path(temporary_name).unlink(missing_ok=True)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run the transient Phase 4 GB10 ASR gate")
    parser.add_argument("--checked-head", required=True)
    parser.add_argument("--image", required=True)
    parser.add_argument("--lock", required=True)
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--repo-root", required=True)
    parser.add_argument("--result", required=True)
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--max-wer", type=float, default=0.12)
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    evidence = run_gate(
        checked_head=arguments.checked_head,
        image=arguments.image,
        lock_path=Path(arguments.lock),
        model_dir=Path(arguments.model_dir),
        repo_root=Path(arguments.repo_root),
        result_path=Path(arguments.result),
        evidence_path=Path(arguments.evidence),
        max_wer=arguments.max_wer,
    )
    print(json.dumps(evidence, ensure_ascii=True, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
