from __future__ import annotations

import argparse
import os
from pathlib import Path
import stat
from typing import BinaryIO, Callable
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

from yap_server.pools.model_lock import (
    LockedArtifact,
    ModelArtifactError,
    ModelPoolLock,
    load_model_pool_lock,
    sha256_file,
    verify_model_artifacts,
)


_COPY_BYTES = 4 * 1024 * 1024
_ARTIFACT_HOST_SUFFIXES = ("huggingface.co", "hf.co")
Opener = Callable[..., BinaryIO]


class _PinnedArtifactRedirectHandler(HTTPRedirectHandler):
    def redirect_request(
        self,
        request: Request,
        response: object,
        code: int,
        message: str,
        headers: object,
        new_url: str,
    ) -> Request | None:
        _validate_artifact_destination(new_url)
        return super().redirect_request(
            request,
            response,
            code,
            message,
            headers,
            new_url,
        )


def artifact_url(lock: ModelPoolLock, artifact: LockedArtifact) -> str:
    encoded_path = quote(artifact.path, safe="")
    return (
        f"https://huggingface.co/{lock.model_distribution_id}/resolve/"
        f"{lock.model_distribution_revision}/{encoded_path}"
    )


def sync_model_artifacts(
    lock: ModelPoolLock,
    model_dir: Path,
    *,
    opener: Opener | None = None,
    timeout_seconds: float = 120.0,
) -> None:
    if timeout_seconds <= 0:
        raise ValueError("download timeout must be positive")
    model_dir.mkdir(parents=True, exist_ok=True)
    root = model_dir.resolve(strict=True)
    if not root.is_dir():
        raise ModelArtifactError("model destination is not a directory")

    resolved_opener = opener or build_opener(_PinnedArtifactRedirectHandler()).open
    for artifact in lock.artifacts:
        destination = root / artifact.path
        if _matches(destination, artifact):
            print(f"verified {artifact.path}")
            continue
        _reject_unsafe_existing(destination)
        destination.unlink(missing_ok=True)
        _download_artifact(
            artifact_url(lock, artifact),
            destination,
            artifact,
            opener=resolved_opener,
            timeout_seconds=timeout_seconds,
        )
        print(f"downloaded {artifact.path}")
    verify_model_artifacts(lock, root)


def _validate_artifact_destination(url: str) -> None:
    parsed = urlsplit(url)
    hostname = parsed.hostname.lower().rstrip(".") if parsed.hostname else ""
    try:
        port = parsed.port
    except ValueError as error:
        raise ModelArtifactError("model artifact redirect has an invalid port") from error
    if (
        parsed.scheme.lower() != "https"
        or parsed.username is not None
        or parsed.password is not None
        or port not in (None, 443)
        or not any(
            hostname == suffix or hostname.endswith(f".{suffix}")
            for suffix in _ARTIFACT_HOST_SUFFIXES
        )
    ):
        raise ModelArtifactError("model artifact redirect left the approved HTTPS hosts")


def _matches(path: Path, artifact: LockedArtifact) -> bool:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        return False
    return metadata.st_size == artifact.size and sha256_file(path) == artifact.sha256


def _reject_unsafe_existing(path: Path) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ModelArtifactError("model destination contains an unsafe artifact path")


def _download_artifact(
    url: str,
    destination: Path,
    artifact: LockedArtifact,
    *,
    opener: Opener,
    timeout_seconds: float,
) -> None:
    partial = destination.with_name(destination.name + ".part")
    _reject_unsafe_existing(partial)
    offset = partial.stat().st_size if partial.exists() else 0
    if offset > artifact.size:
        partial.unlink()
        offset = 0
    if offset == artifact.size:
        if _matches(partial, artifact):
            os.replace(partial, destination)
            return
        partial.unlink()
        offset = 0

    request = Request(
        url,
        headers={
            "Accept": "application/octet-stream",
            "User-Agent": "yap-phase4-model-fetch/1",
            **({"Range": f"bytes={offset}-"} if offset else {}),
        },
    )
    with opener(request, timeout=timeout_seconds) as response:
        status = getattr(response, "status", None)
        append = offset > 0 and status == 206
        if offset and not append:
            offset = 0
        mode = "ab" if append else "wb"
        downloaded = offset
        oversized = False
        with partial.open(mode) as output:
            while True:
                block = response.read(_COPY_BYTES)
                if not block:
                    break
                if downloaded + len(block) > artifact.size:
                    oversized = True
                    break
                output.write(block)
                downloaded += len(block)
            if not oversized:
                output.flush()
                os.fsync(output.fileno())

    if oversized:
        partial.unlink(missing_ok=True)
        raise ModelArtifactError(
            f"downloaded artifact exceeded locked size: {artifact.path}"
        )

    if not _matches(partial, artifact):
        raise ModelArtifactError(f"downloaded artifact failed verification: {artifact.path}")
    partial.chmod(0o640)
    os.replace(partial, destination)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Acquire or verify the immutable Phase 4 ASR model artifacts"
    )
    parser.add_argument("--lock", required=True)
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--verify-only", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    lock = load_model_pool_lock(Path(arguments.lock))
    model_dir = Path(arguments.model_dir)
    if arguments.verify_only:
        verify_model_artifacts(lock, model_dir)
    else:
        sync_model_artifacts(lock, model_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
