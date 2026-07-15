from __future__ import annotations

import json
import logging
import socket
import sys
import threading
import time
from http.server import HTTPServer, ThreadingHTTPServer
from ipaddress import ip_address
from typing import Any


MAX_CONCURRENT_REQUEST_THREADS = 8
REQUEST_THREAD_ACQUIRE_TIMEOUT_SECONDS = 2.0
MAINTENANCE_INTERVAL_SECONDS = 60.0

_REQUEST_LOGGER = logging.getLogger("yap_server.requests")


class YapHTTPServer(HTTPServer):
    def handle_error(self, request: Any, client_address: Any) -> None:
        del request, client_address
        if isinstance(sys.exc_info()[1], ConnectionError):
            return
        _log_unhandled_request_failure(self)


class ThreadingYapHTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = MAX_CONCURRENT_REQUEST_THREADS * 2

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        self._request_slots = threading.BoundedSemaphore(MAX_CONCURRENT_REQUEST_THREADS)
        self._job_service_for_maintenance: object | None = None
        self._next_maintenance_at = 0.0
        super().__init__(*args, **kwargs)

    def service_actions(self) -> None:
        super().service_actions()
        now = time.monotonic()
        if now < self._next_maintenance_at:
            return
        self._next_maintenance_at = now + MAINTENANCE_INTERVAL_SECONDS
        maintenance = getattr(self._job_service_for_maintenance, "prune_expired", None)
        if callable(maintenance):
            try:
                maintenance()
            except OSError:
                # Retention remains pending and is retried on the next bounded
                # maintenance interval. Do not let filesystem details escape
                # through the server loop's default traceback handling.
                pass

    def process_request(self, request: Any, client_address: Any) -> None:
        if not self._request_slots.acquire(
            timeout=REQUEST_THREAD_ACQUIRE_TIMEOUT_SECONDS
        ):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self._request_slots.release()
            raise

    def process_request_thread(self, request: Any, client_address: Any) -> None:
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._request_slots.release()

    def handle_error(self, request: Any, client_address: Any) -> None:
        del request, client_address
        if isinstance(sys.exc_info()[1], ConnectionError):
            return
        _log_unhandled_request_failure(self)


def _log_unhandled_request_failure(server: HTTPServer) -> None:
    logger = getattr(server, "_request_error_logger", _REQUEST_LOGGER)
    try:
        logger.info(
            json.dumps(
                {"event": "http_request_failure", "status": 500},
                ensure_ascii=True,
                separators=(",", ":"),
            )
        )
    except Exception:
        # Error reporting is itself an outer trust boundary. A broken logger
        # must not restore BaseServer's exception traceback or leak request data.
        pass


class _IPv6HTTPServer(YapHTTPServer):
    address_family = socket.AF_INET6


class _IPv6ThreadingHTTPServer(ThreadingYapHTTPServer):
    address_family = socket.AF_INET6


def server_type(host: str, *, threaded: bool) -> type[HTTPServer]:
    try:
        if ip_address(host).version == 6:
            return _IPv6ThreadingHTTPServer if threaded else _IPv6HTTPServer
    except ValueError:
        pass
    return ThreadingYapHTTPServer if threaded else YapHTTPServer
