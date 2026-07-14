from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile


_SNAPSHOT_LIMIT_BYTES = 16 * 1024 * 1024
_RESULT_LIMIT_BYTES = 2 * 1024 * 1024
_EVIDENCE_LIMIT_BYTES = 2 * 1024 * 1024
_UNCHANGED_SNAPSHOTS = ("listeners.txt", "firewall.txt", "services.txt")
_EMPTY_RUNTIME_SNAPSHOTS = ("containers.txt", "workers.txt")


def _read_regular_file(path: Path, *, limit_bytes: int) -> bytes:
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError(f"Phase 4 evidence input is not a regular file: {path.name}")
    if metadata.st_size > limit_bytes:
        raise RuntimeError(f"Phase 4 evidence input exceeds its bound: {path.name}")
    with path.open("rb") as source:
        payload = source.read(limit_bytes + 1)
    if len(payload) > limit_bytes:
        raise RuntimeError(f"Phase 4 evidence input exceeds its bound: {path.name}")
    return payload


def _snapshot(root: Path, name: str) -> bytes:
    return _read_regular_file(root / name, limit_bytes=_SNAPSHOT_LIMIT_BYTES)


def _digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def finalize_host_boundary_evidence(
    *,
    before_dir: Path,
    after_dir: Path,
    inference_result_path: Path,
    inference_evidence_path: Path,
    result_path: Path,
    evidence_path: Path,
) -> dict[str, object]:
    before = {name: _snapshot(before_dir, name) for name in _UNCHANGED_SNAPSHOTS}
    after = {name: _snapshot(after_dir, name) for name in _UNCHANGED_SNAPSHOTS}
    for name in _UNCHANGED_SNAPSHOTS:
        if before[name] != after[name]:
            raise RuntimeError(f"Phase 4 changed observed host state: {name}")

    for name in _EMPTY_RUNTIME_SNAPSHOTS:
        before_value = _snapshot(before_dir, name)
        after_value = _snapshot(after_dir, name)
        if before_value.strip():
            raise RuntimeError(f"Phase 4 started with residual runtime state: {name}")
        if after_value.strip():
            raise RuntimeError(f"Phase 4 left residual runtime state: {name}")

    result_bytes = _read_regular_file(
        inference_result_path,
        limit_bytes=_RESULT_LIMIT_BYTES,
    )
    evidence_bytes = _read_regular_file(
        inference_evidence_path,
        limit_bytes=_EVIDENCE_LIMIT_BYTES,
    )
    try:
        evidence = json.loads(evidence_bytes)
    except json.JSONDecodeError as error:
        raise RuntimeError("Phase 4 inference evidence is invalid JSON") from error
    if not isinstance(evidence, dict) or evidence.get("schemaVersion") != 1:
        raise RuntimeError("Phase 4 inference evidence has an invalid schema")
    if evidence.get("resultSha256") != _digest(result_bytes):
        raise RuntimeError("Phase 4 result digest does not match its inference evidence")
    boundary = evidence.get("boundary")
    if not isinstance(boundary, dict) or (
        boundary.get("network") != "none" or boundary.get("workerCount") != 1
    ):
        raise RuntimeError("Phase 4 inference evidence has an invalid worker boundary")

    finalized_boundary = dict(boundary)
    finalized_boundary.update(
        {
            "ports": [],
            "persistentService": False,
            "hostObservation": "verified",
            "observedHostBoundary": {
                "listenerStateUnchanged": True,
                "firewallStateUnchanged": True,
                "serviceUnitsUnchanged": True,
                "remainingPhase4Containers": 0,
                "remainingWorkerProcesses": 0,
                "snapshotSha256": {
                    name.removesuffix(".txt"): _digest(before[name])
                    for name in _UNCHANGED_SNAPSHOTS
                },
            },
        }
    )
    evidence["boundary"] = finalized_boundary

    _write_bytes_atomic(result_path, result_bytes)
    _write_json_atomic(evidence_path, evidence)
    return evidence


def _write_bytes_atomic(path: Path, payload: bytes) -> None:
    destination = path.resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            prefix=f".{destination.name}.",
            suffix=".tmp",
            dir=destination.parent,
            delete=False,
        ) as temporary:
            temporary_name = temporary.name
            temporary.write(payload)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_name, destination)
        temporary_name = None
    finally:
        if temporary_name is not None:
            Path(temporary_name).unlink(missing_ok=True)


def _write_json_atomic(path: Path, payload: dict[str, object]) -> None:
    serialized = (
        json.dumps(payload, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    _write_bytes_atomic(path, serialized)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Finalize Phase 4 evidence after observed host-boundary teardown"
    )
    parser.add_argument("--before", required=True)
    parser.add_argument("--after", required=True)
    parser.add_argument("--inference-result", required=True)
    parser.add_argument("--inference-evidence", required=True)
    parser.add_argument("--result", required=True)
    parser.add_argument("--evidence", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    finalized = finalize_host_boundary_evidence(
        before_dir=Path(arguments.before),
        after_dir=Path(arguments.after),
        inference_result_path=Path(arguments.inference_result),
        inference_evidence_path=Path(arguments.inference_evidence),
        result_path=Path(arguments.result),
        evidence_path=Path(arguments.evidence),
    )
    print(json.dumps(finalized, ensure_ascii=True, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
