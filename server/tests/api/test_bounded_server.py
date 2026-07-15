import io
import tempfile
import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from yap_server.api.app import MAX_CONCURRENT_REQUEST_THREADS, create_server
from yap_server.config import ServerSettings
from yap_server.jobs import RecordingJobService

from .api_fixtures import (
    _BlockingStatusService,
    _CapturingLogger,
    _ControlledProcessor,
    _phase5_job_request,
)


class BoundedBatchJobServerTests(unittest.TestCase):
    def test_storage_maintenance_failure_stays_private_and_keeps_serving(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_ControlledProcessor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:00:00Z",
            )
            server = create_server(
                ServerSettings(host="127.0.0.1", port=0),
                logger=_CapturingLogger(),
                job_service=service,
            )
            serving = threading.Thread(
                target=server.serve_forever,
                kwargs={"poll_interval": 0.01},
                daemon=True,
            )
            attempted = threading.Event()
            private_path = "C:/private/recordings/patient-audio.wav"

            def fail_maintenance() -> None:
                attempted.set()
                raise OSError(f"could not prune {private_path}")

            stderr = io.StringIO()
            try:
                with redirect_stderr(stderr):
                    with patch.object(
                        service,
                        "prune_expired",
                        side_effect=fail_maintenance,
                    ):
                        serving.start()
                        self.assertTrue(attempted.wait(timeout=1))
                        self.assertTrue(serving.is_alive())
            finally:
                server.shutdown()
                serving.join(timeout=2)
                server.server_close()

            self.assertFalse(serving.is_alive())
            self.assertNotIn(private_path, stderr.getvalue())

    def test_idle_server_runs_terminal_retention_without_a_new_request(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clock = {"now": "2026-07-14T21:00:00Z"}
            service = RecordingJobService(
                root,
                processor=_ControlledProcessor(),
                supported_languages=("en",),
                now=lambda: clock["now"],
            )
            request = _phase5_job_request()
            request["metadata"]["retentionExpiresAtUtc"] = "2026-07-15T00:00:00Z"
            created = service.create(request)
            service.cancel(created["jobId"])
            expired_root = root / "jobs" / created["jobId"]
            server = create_server(
                ServerSettings(host="127.0.0.1", port=0),
                logger=_CapturingLogger(),
                job_service=service,
            )
            serving = threading.Thread(
                target=server.serve_forever,
                kwargs={"poll_interval": 0.01},
                daemon=True,
            )
            clock["now"] = "2026-07-16T00:00:00Z"
            serving.start()
            try:
                deadline = time.monotonic() + 1
                while expired_root.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertFalse(expired_root.exists())
            finally:
                server.shutdown()
                serving.join(timeout=2)
                server.server_close()

    def test_batch_request_threads_are_bounded_under_concurrent_load(self) -> None:
        maximum_threads = MAX_CONCURRENT_REQUEST_THREADS
        service = _BlockingStatusService(maximum_threads)
        server = create_server(
            ServerSettings(host="127.0.0.1", port=0),
            logger=_CapturingLogger(),
            job_service=service,  # type: ignore[arg-type]
        )
        host, port = server.server_address[:2]
        serving = threading.Thread(
            target=server.serve_forever,
            kwargs={"poll_interval": 0.01},
            daemon=True,
        )
        serving.start()

        def request_status(index: int) -> int:
            with urlopen(
                f"http://{host}:{port}/v1/jobs/job-load-{index}",
                timeout=5,
            ) as response:
                response.read()
                return response.status

        try:
            with ThreadPoolExecutor(max_workers=maximum_threads + 2) as requests:
                responses = [
                    requests.submit(request_status, index)
                    for index in range(maximum_threads + 2)
                ]
                self.assertTrue(service.saturated.wait(timeout=2))
                time.sleep(0.1)
                self.assertLessEqual(service.maximum_active, maximum_threads)
                service.release.set()
                self.assertEqual(
                    [response.result(timeout=5) for response in responses],
                    [200] * (maximum_threads + 2),
                )
        finally:
            service.release.set()
            server.shutdown()
            server.server_close()
            serving.join(timeout=2)
            self.assertFalse(serving.is_alive())
