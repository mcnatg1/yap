from __future__ import annotations

import os
from pathlib import Path
import stat


def read_regular_file(path: Path, maximum_bytes: int) -> bytes:
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"required persisted file is missing: {path.name}") from error
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_size > maximum_bytes
    ):
        raise ValueError(f"persisted file is unsafe or oversized: {path.name}")

    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise ValueError(f"persisted file is unsafe or oversized: {path.name}") from error
    try:
        opened = os.fstat(descriptor)
        if (
            not stat.S_ISREG(opened.st_mode)
            or (metadata.st_dev, metadata.st_ino) != (opened.st_dev, opened.st_ino)
            or opened.st_size > maximum_bytes
        ):
            raise ValueError(f"persisted file is unsafe or oversized: {path.name}")

        blocks: list[bytes] = []
        remaining = maximum_bytes + 1
        while remaining:
            block = os.read(descriptor, min(remaining, 1024 * 1024))
            if not block:
                break
            blocks.append(block)
            remaining -= len(block)
        body = b"".join(blocks)
        if len(body) > maximum_bytes:
            raise ValueError(f"persisted file is unsafe or oversized: {path.name}")
        return body
    except OSError as error:
        raise ValueError(f"persisted file is unsafe or oversized: {path.name}") from error
    finally:
        os.close(descriptor)


def read_regular_text(path: Path, maximum_bytes: int) -> str:
    try:
        return read_regular_file(path, maximum_bytes).decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError(f"persisted file is not UTF-8: {path.name}") from error
