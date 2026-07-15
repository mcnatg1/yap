import json
from http import HTTPStatus
from typing import Any, Mapping
from urllib.parse import urlsplit

from yap_server.jobs import JobServiceError

from .routes import SUPPORTED_HTTP_VERSIONS


MAX_LOG_METHOD_CHARS = 16
MAX_LOG_PATH_CHARS = 128


def _bounded(value: str | None, maximum: int) -> str:
    if not isinstance(value, str):
        return ""
    if len(value) <= maximum:
        return value
    return value[:maximum] + "…"


def _sanitized_log_path(value: object) -> str:
    if not isinstance(value, str):
        return ""
    try:
        return urlsplit(value).path
    except ValueError:
        return ""


class ResponseMixin:
    def _request_size_is_allowed(self) -> bool:
        try:
            self._request_body.capture_content_length()
        except JobServiceError as error:
            self._send_error(
                HTTPStatus(error.status),
                code=error.code,
                message=error.message,
                retryable=error.retryable,
            )
            return False
        return True

    def _send_error(
        self,
        status: HTTPStatus,
        *,
        code: str,
        message: str,
        retryable: bool = False,
        headers: Mapping[str, str] | None = None,
    ) -> None:
        self._send_json(
            status,
            {
                "code": code,
                "message": message,
                "retryable": retryable,
                "requestId": self._request_id,
            },
            headers=headers,
        )

    def _send_version_not_supported(self) -> None:
        self.request_version = "HTTP/1.0"
        self.close_connection = True
        self._send_error(
            HTTPStatus.HTTP_VERSION_NOT_SUPPORTED,
            code="HTTP_VERSION_NOT_SUPPORTED",
            message="HTTP/1.0 or HTTP/1.1 is required.",
        )

    def _send_json(
        self,
        status: HTTPStatus,
        payload: Mapping[str, object],
        *,
        headers: Mapping[str, str] | None = None,
    ) -> None:
        # On Windows, closing a socket with unread request bytes can emit a TCP
        # reset that discards an otherwise valid error response. Every declared
        # body is bounded before dispatch, so consume the bounded remainder at
        # the response boundary without allowing drain failures to mask it.
        self._request_body.discard_unread()
        body = json.dumps(
            payload,
            ensure_ascii=True,
            separators=(",", ":"),
        ).encode("utf-8")
        self._log_structured_request(int(status))
        try:
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Length", str(len(body)))
            if headers:
                for name, value in headers.items():
                    self.send_header(name, value)
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
        except OSError:
            self.close_connection = True

    def _log_structured_request(self, status: int) -> None:
        if self._request_logged:
            return
        self._request_logged = True
        event = {
            "event": "http_request",
            "method": _bounded(getattr(self, "command", ""), MAX_LOG_METHOD_CHARS),
            "path": _bounded(
                _sanitized_log_path(getattr(self, "path", "")),
                MAX_LOG_PATH_CHARS,
            ),
            "status": status,
            "requestId": self._request_id,
        }
        self._request_logger.info(
            json.dumps(event, ensure_ascii=True, separators=(",", ":"))
        )

    def send_error(
        self,
        code: int,
        message: str | None = None,
        explain: str | None = None,
    ) -> None:
        del message, explain
        try:
            status = HTTPStatus(code)
        except ValueError:
            status = HTTPStatus.INTERNAL_SERVER_ERROR
        if status == HTTPStatus.HTTP_VERSION_NOT_SUPPORTED:
            self._send_version_not_supported()
            return
        if self.request_version not in SUPPORTED_HTTP_VERSIONS:
            self.request_version = "HTTP/1.0"
            self.close_connection = True
        self._send_error(
            status,
            code="HTTP_ERROR",
            message="The HTTP request could not be processed.",
        )

    def log_message(self, format: str, *args: Any) -> None:
        del format, args

    def log_error(self, format: str, *args: Any) -> None:
        del args
        if format == "Request timed out: %r" and not self._request_logged:
            self._send_error(
                HTTPStatus.REQUEST_TIMEOUT,
                code="REQUEST_TIMEOUT",
                message="Request headers were not completed in time.",
            )
