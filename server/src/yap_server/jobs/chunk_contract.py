from __future__ import annotations

from pathlib import Path
from typing import Mapping

from .contract_values import mapping


def find_chunk(
    request: Mapping[str, object],
    *,
    track_id: str,
    sequence_start: int,
    sequence_end: int,
) -> Mapping[str, object]:
    chunks = request.get("chunks")
    if not isinstance(chunks, list):
        raise ValueError("chunks must be an array")
    for value in chunks:
        chunk = mapping(value, "chunks[]")
        replay_key = mapping(chunk.get("replayKey"), "chunks[].replayKey")
        if (
            replay_key.get("trackId") == track_id
            and replay_key.get("sequenceStart") == sequence_start
            and replay_key.get("sequenceEnd") == sequence_end
        ):
            return chunk
    raise KeyError("declared chunk was not found")


def chunk_path(job_root: Path, replay_key: Mapping[str, object]) -> Path:
    return job_root / "chunks" / (
        f"{replay_key['trackId']}-{replay_key['sequenceStart']}-"
        f"{replay_key['sequenceEnd']}.pcm"
    )


def receipt_key(
    job_id: str,
    replay_key: Mapping[str, object],
) -> tuple[object, ...]:
    return (
        job_id,
        replay_key["schemaVersion"],
        replay_key["sessionId"],
        replay_key["trackId"],
        replay_key["sequenceStart"],
        replay_key["sequenceEnd"],
    )


def validated_single_track_chunks(
    values: list[object],
) -> list[Mapping[str, object]]:
    chunks = [mapping(value, "chunks[]") for value in values]
    chunks.sort(
        key=lambda chunk: (
            mapping(chunk.get("replayKey"), "replayKey")["sequenceStart"],
            mapping(chunk.get("replayKey"), "replayKey")["sequenceEnd"],
        )
    )
    track_ids = {
        mapping(chunk.get("replayKey"), "replayKey").get("trackId")
        for chunk in chunks
    }
    if len(track_ids) != 1:
        raise ValueError("the Phase 5 ASR slice accepts exactly one audio track")
    expected_start_ms = 0
    expected_sequence_start = 0
    for chunk in chunks:
        replay_key = mapping(chunk.get("replayKey"), "replayKey")
        start_ms = chunk.get("startMs")
        duration_ms = chunk.get("durationMs")
        content = mapping(chunk.get("contentIdentity"), "contentIdentity")
        if start_ms != expected_start_ms:
            raise ValueError("the Phase 5 ASR slice requires contiguous chunk timing")
        if not isinstance(duration_ms, int) or isinstance(duration_ms, bool):
            raise ValueError("chunk duration must be an integer")
        byte_length = content.get("byteLength")
        if (
            not isinstance(byte_length, int)
            or isinstance(byte_length, bool)
            or byte_length * 1000 != duration_ms * 16000 * 2
        ):
            raise ValueError("chunk duration does not match mono 16 kHz PCM length")
        sequence_start = replay_key.get("sequenceStart")
        sequence_end = replay_key.get("sequenceEnd")
        if (
            not isinstance(sequence_start, int)
            or not isinstance(sequence_end, int)
            or sequence_end < sequence_start
            or sequence_start != expected_sequence_start
        ):
            raise ValueError("chunk sequence ranges must be contiguous")
        expected_start_ms += duration_ms
        expected_sequence_start = sequence_end + 1
    return chunks
