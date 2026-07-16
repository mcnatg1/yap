from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

from yap_server.jobs.artifacts import PcmChunkSource, publish_wav, read_regular_file


def test_regular_file_read_rechecks_growth_after_metadata(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    artifact = tmp_path / "state.json"
    artifact.write_bytes(b"{}")
    original_lstat = Path.lstat
    grew = False

    def grow_after_metadata(path: Path):  # type: ignore[no-untyped-def]
        nonlocal grew
        metadata = original_lstat(path)
        if path == artifact and not grew:
            grew = True
            artifact.write_bytes(b"x" * 9)
        return metadata

    monkeypatch.setattr(Path, "lstat", grow_after_metadata)

    with pytest.raises(ValueError, match="unsafe or oversized"):
        read_regular_file(artifact, 8)


def test_regular_file_read_accepts_content_at_the_exact_bound(tmp_path: Path) -> None:
    artifact = tmp_path / "chunk.pcm"
    artifact.write_bytes(b"12345678")

    assert read_regular_file(artifact, 8) == b"12345678"


def test_wav_publication_revalidates_declared_chunk_identity(tmp_path: Path) -> None:
    chunk = tmp_path / "chunk.pcm"
    original = b"\x00\x00\x01\x00"
    chunk.write_bytes(original)
    source = PcmChunkSource(
        path=chunk,
        byte_length=len(original),
        sha256=hashlib.sha256(original).hexdigest(),
    )
    chunk.write_bytes(b"\x02\x00\x03\x00")
    destination = tmp_path / "input.wav"

    with pytest.raises(ValueError, match="no longer matches"):
        publish_wav(destination, [source])

    assert not destination.exists()
    assert not list(tmp_path.glob(".input-*.wav.part"))
