from http import HTTPStatus

from yap_server.jobs import JobServiceError

from .routes import CHUNK_PATH, COMMIT_PATH, JOB_PATH, RESULT_PATH


class JobRequestMixin:
    def _dispatch_job_request(self, path: str) -> None:
        assert self._job_service is not None
        try:
            if path == "/v1/jobs" and self.command == "POST":
                idempotency_key = self._request_body.required_header(
                    "Idempotency-Key",
                    code="IDEMPOTENCY_KEY_REQUIRED",
                    message="Job creation requires exactly one idempotency key.",
                )
                payload = self._request_body.read_json()
                self._send_json(
                    HTTPStatus.ACCEPTED,
                    self._job_service.create(
                        payload,
                        idempotency_key=idempotency_key,
                    ),
                )
                return

            chunk_match = CHUNK_PATH.fullmatch(path)
            if chunk_match is not None and self.command == "PUT":
                if self.headers.get_content_type() != "application/octet-stream":
                    raise JobServiceError(
                        415,
                        "UNSUPPORTED_MEDIA_TYPE",
                        "Chunk uploads require application/octet-stream.",
                    )
                content_length = self._request_body.required_content_length()
                plan = self._job_service.prepare_chunk_upload(
                    chunk_match.group("job_id"),
                    track_id=chunk_match.group("track_id"),
                    sequence_start=int(chunk_match.group("sequence_start"), 10),
                    sequence_end=int(chunk_match.group("sequence_end"), 10),
                    idempotency_key=self._request_body.required_header("Idempotency-Key"),
                    content_sha256=self._request_body.required_header("X-Yap-Content-SHA256"),
                    audio_codec=self._request_body.required_header("X-Yap-Audio-Codec"),
                    sample_rate_hz=self._request_body.integer_header("X-Yap-Sample-Rate-Hz"),
                    channels=self._request_body.integer_header("X-Yap-Channels"),
                    content_length=content_length,
                )
                receipt = self._job_service.accept_chunk(
                    plan,
                    self._request_body.read_exact(content_length),
                )
                status = (
                    HTTPStatus.OK
                    if receipt.get("disposition") == "replayed"
                    else HTTPStatus.CREATED
                )
                self._send_json(status, receipt)
                return

            commit_match = COMMIT_PATH.fullmatch(path)
            if commit_match is not None and self.command == "POST":
                payload = self._request_body.read_json()
                self._send_json(
                    HTTPStatus.ACCEPTED,
                    self._job_service.commit(commit_match.group("job_id"), payload),
                )
                return

            result_match = RESULT_PATH.fullmatch(path)
            if result_match is not None and self.command == "GET":
                self._send_json(
                    HTTPStatus.OK,
                    self._job_service.get_result(result_match.group("job_id")),
                )
                return

            job_match = JOB_PATH.fullmatch(path)
            if job_match is not None and self.command == "DELETE":
                self._send_json(
                    HTTPStatus.ACCEPTED,
                    self._job_service.cancel(job_match.group("job_id")),
                )
                return
            if job_match is not None and self.command == "GET":
                self._send_json(
                    HTTPStatus.OK,
                    self._job_service.get(job_match.group("job_id")),
                )
                return
        except JobServiceError as error:
            self._send_error(
                HTTPStatus(error.status),
                code=error.code,
                message=error.message,
                retryable=error.retryable,
            )
            return
        except KeyError:
            self._send_error(
                HTTPStatus.NOT_FOUND,
                code="JOB_NOT_FOUND",
                message="Recording job not found.",
            )
            return
        except TimeoutError:
            self.close_connection = True
            self._send_error(
                HTTPStatus.REQUEST_TIMEOUT,
                code="REQUEST_TIMEOUT",
                message="The bounded request did not complete in time.",
                retryable=True,
            )
            return
        except ConnectionError:
            self.close_connection = True
            return
        except OSError:
            self._send_error(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                code="SERVER_STORAGE_ERROR",
                message="Private recording storage could not complete the request.",
                retryable=True,
            )
            return
        except (TypeError, ValueError):
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_REQUEST_BODY",
                message="Request body does not match the operation contract.",
            )
            return

        self._send_error(
            HTTPStatus.NOT_IMPLEMENTED,
            code="NOT_IMPLEMENTED",
            message="This operation is not implemented in the Phase 5 batch slice.",
        )
