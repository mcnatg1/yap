from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
from typing import Mapping, Sequence
import wave

from yap_server.bounded_file import read_regular_file


MAX_STATE_BYTES = 2 * 1024 * 1024


@dataclass(frozen=True, slots=True)
class PcmChunkSource:
    path: Path
    byte_length: int
    sha256: str


def publish_wav(destination: Path, chunk_sources: Sequence[PcmChunkSource]) -> None:
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w+b",
            dir=destination.parent,
            prefix=".input-",
            suffix=".wav.part",
            delete=False,
        ) as temporary:
            temporary_path = Path(temporary.name)
            with wave.open(temporary, "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16000)
                for source in chunk_sources:
                    body = read_regular_file(source.path, source.byte_length)
                    if (
                        len(body) != source.byte_length
                        or hashlib.sha256(body).hexdigest() != source.sha256
                    ):
                        raise ValueError(
                            "an uploaded chunk no longer matches its identity"
                        )
                    output.writeframesraw(body)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_path, destination)
        temporary_path = None
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def publish_json(destination: Path, payload: Mapping[str, object]) -> None:
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            dir=destination.parent,
            prefix=".result-",
            delete=False,
        ) as temporary:
            temporary_path = Path(temporary.name)
            json.dump(payload, temporary, ensure_ascii=True, separators=(",", ":"))
            temporary.write("\n")
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_path, destination)
        temporary_path = None
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def read_json_file(path: Path) -> dict[str, object]:
    body = read_regular_file(path, MAX_STATE_BYTES)
    try:
        value = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"persisted JSON is invalid: {path.name}") from error
    if not isinstance(value, dict):
        raise ValueError(f"persisted JSON must be an object: {path.name}")
    return value


def unlink_private_regular_file(path: Path, label: str) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{label} is unsafe")
    path.unlink()
