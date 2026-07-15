from __future__ import annotations

import logging
from functools import partial
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any
from urllib.parse import urlsplit
from uuid import uuid4

from yap_server.api.health import health
from yap_server.config import ServerSettings
from yap_server.config.settings import ensure_bind_is_allowed
from yap_server.jobs import RecordingJobService

from .http_server import (
    MAX_CONCURRENT_REQUEST_THREADS,
    ThreadingYapHTTPServer,
    server_type,
)
from .job_requests import JobRequestMixin
from .request_io import (
    BoundedRequestBody,
    REQUEST_IO_TIMEOUT_SECONDS,
    bounded_socket_reader,
    request_deadline,
)
from .responses import ResponseMixin
from .routes import SUPPORTED_HTTP_VERSIONS, allowed_methods as methods_for_path


_REQUEST_LOGGER = logging.getLogger("yap_server.requests")


class _HealthRequestHandler(JobRequestMixin, ResponseMixin, BaseHTTPRequestHandler):
    server_version = "yap-server"
    sys_version = ""
    timeout = REQUEST_IO_TIMEOUT_SECONDS

    def __init__(
        self,
        *args: Any,
        request_logger: logging.Logger,
        job_service: RecordingJobService | None,
        **kwargs: Any,
    ) -> None:
        self._request_logger = request_logger
        self._job_service = job_service
        self._request_id = f"req-{uuid4().hex}"
        self._request_logged = False
        self._request_body = BoundedRequestBody(self)
        self.requestline = ""
        self.request_version = ""
        self.command = ""
        super().__init__(*args, **kwargs)

    def setup(self) -> None:
        deadline = request_deadline()
        super().setup()
        original_rfile = self.rfile
        self.rfile = bounded_socket_reader(self.connection, deadline)
        original_rfile.close()

    def do_GET(self) -> None:
        self._dispatch()

    def do_POST(self) -> None:
        self._dispatch()

    def do_PUT(self) -> None:
        self._dispatch()

    def do_DELETE(self) -> None:
        self._dispatch()

    def do_CONNECT(self) -> None:
        self._dispatch()

    def do_PATCH(self) -> None:
        self._dispatch()

    def do_HEAD(self) -> None:
        self._dispatch()

    def do_OPTIONS(self) -> None:
        self._dispatch()

    def do_TRACE(self) -> None:
        self._dispatch()

    def _dispatch(self) -> None:
        if self.request_version not in SUPPORTED_HTTP_VERSIONS:
            self._send_version_not_supported()
            return

        if not self._request_size_is_allowed():
            return

        try:
            path = urlsplit(self.path).path
        except ValueError:
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_REQUEST_TARGET",
                message="Request target is invalid.",
            )
            return
        allowed_methods = methods_for_path(path)
        if allowed_methods is None:
            self._send_error(
                HTTPStatus.NOT_FOUND,
                code="NOT_FOUND",
                message="Route not found.",
            )
            return

        if self.command not in allowed_methods:
            self._send_error(
                HTTPStatus.METHOD_NOT_ALLOWED,
                code="METHOD_NOT_ALLOWED",
                message="Method not allowed for this route.",
                headers={"Allow": ", ".join(sorted(allowed_methods))},
            )
            return

        if path == "/v1/health":
            self._send_json(
                HTTPStatus.OK,
                health(batch_jobs=self._job_service is not None),
            )
            return

        if self._job_service is not None and path != "/v1/live":
            self._dispatch_job_request(path)
            return

        self._send_error(
            HTTPStatus.NOT_IMPLEMENTED,
            code="NOT_IMPLEMENTED",
            message="This route is contract-only in Phase 3.",
        )


def create_server(
    settings: ServerSettings,
    *,
    logger: logging.Logger | None = None,
    job_service: RecordingJobService | None = None,
) -> HTTPServer:
    ensure_bind_is_allowed(settings.host)
    request_logger = logger or _REQUEST_LOGGER
    handler = partial(
        _HealthRequestHandler,
        request_logger=request_logger,
        job_service=job_service,
    )
    server = server_type(
        settings.host,
        threaded=job_service is not None,
    )((settings.host, settings.port), handler)
    server._request_error_logger = request_logger
    if isinstance(server, ThreadingYapHTTPServer):
        server._job_service_for_maintenance = job_service
    return server


def serve(
    settings: ServerSettings,
    *,
    job_service: RecordingJobService | None = None,
) -> None:
    with create_server(settings, job_service=job_service) as server:
        server.serve_forever()
