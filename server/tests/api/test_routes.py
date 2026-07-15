import json

from .api_fixtures import HealthServerTestCase, MAX_REQUEST_BODY_BYTES


class HealthRoutingTests(HealthServerTestCase):
    def test_health_returns_contract_json_and_no_store_headers(self) -> None:
        status, headers, body = self._request("/v1/health")

        self.assertEqual(status, 200)
        self.assert_json_headers(headers, body)
        self.assertEqual(
            json.loads(body),
            {
                "service": "yap-server",
                "status": "ok",
                "apiVersion": "1",
                "auth": "not_configured",
                "capabilities": {
                    "batchJobs": False,
                    "liveStreaming": False,
                    "jobStatus": False,
                },
            },
        )

    def test_unknown_route_returns_the_stable_json_error(self) -> None:
        status, headers, body = self._request("/v1/unknown")

        self.assert_error(
            status,
            headers,
            body,
            expected_status=404,
            code="NOT_FOUND",
            message="Route not found.",
        )

    def test_non_get_health_method_returns_405(self) -> None:
        for method in ("POST", "TRACE"):
            with self.subTest(method=method):
                status, headers, body = self._request(
                    "/v1/health",
                    method=method,
                    data=b"" if method == "POST" else None,
                )

                self.assertEqual(headers["Allow"], "GET")
                self.assert_error(
                    status,
                    headers,
                    body,
                    expected_status=405,
                    code="METHOD_NOT_ALLOWED",
                    message="Method not allowed for this route.",
                )

    def test_oversized_request_is_rejected_before_body_read(self) -> None:
        status, headers, body = self._request(
            "/v1/health",
            method="POST",
            headers={"Content-Length": str(MAX_REQUEST_BODY_BYTES + 1)},
        )

        self.assert_error(
            status,
            headers,
            body,
            expected_status=413,
            code="REQUEST_TOO_LARGE",
            message="Request body exceeds the 1048576-byte limit.",
        )

    def test_contract_only_routes_return_501(self) -> None:
        routes = (
            ("POST", "/v1/jobs"),
            ("GET", "/v1/jobs/job-01"),
            ("DELETE", "/v1/jobs/job-01"),
            ("PUT", "/v1/jobs/job-01/chunks/mic/0-15"),
            ("POST", "/v1/jobs/job-01/commit"),
            ("GET", "/v1/live"),
        )
        for method, path in routes:
            with self.subTest(method=method, path=path):
                status, headers, body = self._request(path, method=method)
                self.assert_error(
                    status,
                    headers,
                    body,
                    expected_status=501,
                    code="NOT_IMPLEMENTED",
                    message="This route is contract-only in Phase 3.",
                )

    def test_invalid_chunk_range_is_not_a_contract_route(self) -> None:
        for suffix in ("not-a-range", "0-15-99", "-1-15"):
            with self.subTest(suffix=suffix):
                status, headers, body = self._request(
                    f"/v1/jobs/job-01/chunks/mic/{suffix}",
                    method="PUT",
                )
                self.assert_error(
                    status,
                    headers,
                    body,
                    expected_status=404,
                    code="NOT_FOUND",
                    message="Route not found.",
                )

    def test_request_logging_is_one_bounded_structured_line(self) -> None:
        path = "/v1/" + ("x" * 5000)
        status, _, _ = self._request(path)

        self.assertEqual(status, 404)
        self.assertEqual(len(self.logger.messages), 1)
        line = self.logger.messages[0]
        self.assertNotIn("\n", line)
        self.assertLessEqual(len(line), 1024)
        event = json.loads(line)
        self.assertEqual(event["event"], "http_request")
        self.assertEqual(event["method"], "GET")
        self.assertEqual(event["status"], 404)
        self.assertLessEqual(len(event["path"]), 513)
