from __future__ import annotations

import io
import json
import socket
import time
from http.client import HTTPMessage
from typing import Any, BinaryIO, Mapping, Protocol

from yap_server.jobs import JobServiceError


MAX_REQUEST_BODY_BYTES = 1024 * 1024
REQUEST_IO_TIMEOUT_SECONDS = 2.0
REQUEST_WALL_CLOCK_DEADLINE_SECONDS = 2.0
_BODY_DRAIN_READ_BYTES = 64 * 1024


class RequestBodyHandler(Protocol):
    headers: HTTPMessage
    rfile: BinaryIO
    close_connection: bool


class _DeadlineSocketReader(io.RawIOBase):
    def __init__(self, connection: socket.socket, deadline: float) -> None:
        super().__init__()
        self._connection = connection
        self._deadline = deadline

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: Any) -> int:
        remaining = self._deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("request wall-clock deadline exceeded")
        self._connection.settimeout(min(REQUEST_IO_TIMEOUT_SECONDS, remaining))
        return self._connection.recv_into(buffer)


def request_deadline() -> float:
    return time.monotonic() + REQUEST_WALL_CLOCK_DEADLINE_SECONDS


def bounded_socket_reader(
    connection: socket.socket,
    deadline: float,
) -> io.BufferedReader:
    return io.BufferedReader(_DeadlineSocketReader(connection, deadline))


class BoundedRequestBody:
    """Owns bounded HTTP body/header parsing for one request handler."""

    def __init__(self, handler: RequestBodyHandler) -> None:
        self._handler = handler
        self._content_length: int | None = None
        self._body_bytes_read = 0

    def capture_content_length(self) -> None:
        if self._handler.headers.get("Transfer-Encoding") is not None:
            self._handler.close_connection = True
            raise JobServiceError(
                400,
                "INVALID_REQUEST_BODY",
                "Transfer-encoded request bodies are not supported.",
            )

        content_lengths = self._handler.headers.get_all("Content-Length", [])
        if not content_lengths:
            self._content_length = None
            return
        if len(content_lengths) != 1:
            self._handler.close_connection = True
            raise JobServiceError(
                400,
                "INVALID_CONTENT_LENGTH",
                "Content-Length must appear exactly once.",
            )

        try:
            content_length = int(content_lengths[0], 10)
        except ValueError:
            content_length = -1
        if content_length < 0:
            self._handler.close_connection = True
            raise JobServiceError(
                400,
                "INVALID_CONTENT_LENGTH",
                "Content-Length must be a non-negative integer.",
            )
        if content_length > MAX_REQUEST_BODY_BYTES:
            self._handler.close_connection = True
            raise JobServiceError(
                413,
                "REQUEST_TOO_LARGE",
                "Request body exceeds the 1048576-byte limit.",
            )
        self._content_length = content_length

    def required_content_length(self) -> int:
        if self._content_length is None:
            raise JobServiceError(
                400,
                "CONTENT_LENGTH_REQUIRED",
                "A bounded Content-Length is required.",
            )
        return self._content_length

    def read_exact(self, content_length: int) -> bytes:
        body = self._handler.rfile.read(content_length)
        self._body_bytes_read += len(body)
        if len(body) != content_length:
            self._handler.close_connection = True
            raise JobServiceError(
                400,
                "INCOMPLETE_REQUEST_BODY",
                "Request body ended before Content-Length bytes were received.",
            )
        return body

    def discard_unread(self) -> None:
        if self._content_length is None:
            return
        remaining = max(0, self._content_length - self._body_bytes_read)
        try:
            while remaining:
                body = self._handler.rfile.read(min(remaining, _BODY_DRAIN_READ_BYTES))
                if not body:
                    self._handler.close_connection = True
                    return
                consumed = len(body)
                self._body_bytes_read += consumed
                remaining -= consumed
        except (OSError, TimeoutError, ValueError):
            self._handler.close_connection = True

    def read_json(self) -> Mapping[str, object]:
        if self._handler.headers.get_content_type() != "application/json":
            raise JobServiceError(
                415,
                "UNSUPPORTED_MEDIA_TYPE",
                "JSON operations require application/json.",
            )
        content_length = self.required_content_length()
        try:
            payload = json.loads(self.read_exact(content_length))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise JobServiceError(
                400,
                "INVALID_JSON",
                "Request body is not valid JSON.",
            ) from error
        if not isinstance(payload, dict):
            raise JobServiceError(
                400,
                "INVALID_REQUEST_BODY",
                "Request body must be a JSON object.",
            )
        return payload

    def required_header(
        self,
        name: str,
        *,
        code: str = "INVALID_CHUNK_HEADERS",
        message: str = "Every required chunk identity header must appear exactly once.",
    ) -> str:
        values = self._handler.headers.get_all(name, [])
        if len(values) != 1 or not values[0]:
            raise JobServiceError(400, code, message)
        return values[0]

    def integer_header(self, name: str) -> int:
        value = self.required_header(name)
        try:
            return int(value, 10)
        except ValueError as error:
            raise JobServiceError(
                400,
                "INVALID_CHUNK_HEADERS",
                "Chunk numeric headers must be decimal integers.",
            ) from error
