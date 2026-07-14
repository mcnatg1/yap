import hashlib
import io
import tempfile
import unittest
from pathlib import Path
from urllib.request import Request

from yap_server.pools.model_assets import (
    _PinnedArtifactRedirectHandler,
    artifact_url,
    sync_model_artifacts,
)
from yap_server.pools.model_lock import (
    LockedArtifact,
    LockedFixture,
    ModelArtifactError,
    ModelPoolLock,
)


class _Response(io.BytesIO):
    def __init__(self, content: bytes, *, status: int) -> None:
        super().__init__(content)
        self.status = status

    def __enter__(self) -> "_Response":
        return self

    def __exit__(self, *args: object) -> None:
        self.close()


def _lock(content: bytes) -> ModelPoolLock:
    return ModelPoolLock(
        schema_version=1,
        runtime_image="registry.example/asr",
        runtime_source="https://example.invalid/runtime",
        runtime_license="Example runtime license",
        runtime_platform="linux/arm64",
        runtime_digest="sha256:" + "a" * 64,
        runtime_source_tag="1.2.3",
        runtime_python_version="3.12",
        runtime_torch_version="2.13.0a0+example",
        runtime_cuda_version="13.3.0",
        runtime_torch_cuda_version="13.3",
        runtime_overlay_packages=(("transformers", "5.13.1"),),
        pool_id="example-batch",
        model_id="example/model",
        model_revision="b" * 40,
        model_license="Apache-2.0",
        model_source="https://example.invalid/model",
        model_distribution_id="example/model-mirror",
        model_distribution_revision="d" * 40,
        model_distribution_source="https://example.invalid/model-mirror",
        model_distribution_provenance="verified test distribution",
        supported_languages=("en",),
        artifacts=(
            LockedArtifact(
                path="weights.bin",
                size=len(content),
                sha256=hashlib.sha256(content).hexdigest(),
            ),
        ),
        fixture=LockedFixture(
            path="fixture.wav",
            source="https://example.invalid/fixture.wav",
            license="CC-BY-4.0",
            sha256="c" * 64,
            golden_transcript="fixture",
        ),
    )


class ModelAssetTests(unittest.TestCase):
    def test_url_uses_the_exact_locked_revision(self) -> None:
        lock = _lock(b"weights")
        self.assertEqual(
            artifact_url(lock, lock.artifacts[0]),
            "https://huggingface.co/example/model-mirror/resolve/"
            + "d" * 40
            + "/weights.bin",
        )

    def test_downloads_and_then_reuses_only_verified_content(self) -> None:
        content = b"immutable model artifact"
        lock = _lock(content)
        calls = 0

        def opener(request: Request, **kwargs: object) -> _Response:
            nonlocal calls
            del request, kwargs
            calls += 1
            return _Response(content, status=200)

        with tempfile.TemporaryDirectory() as directory:
            model_dir = Path(directory) / "model"
            sync_model_artifacts(lock, model_dir, opener=opener)
            sync_model_artifacts(lock, model_dir, opener=opener)

            self.assertEqual((model_dir / "weights.bin").read_bytes(), content)
            self.assertEqual(calls, 1)
            self.assertFalse((model_dir / "weights.bin.part").exists())

    def test_resumes_a_partial_download_with_an_http_range(self) -> None:
        content = b"0123456789abcdef"
        lock = _lock(content)
        observed_range: str | None = None

        def opener(request: Request, **kwargs: object) -> _Response:
            nonlocal observed_range
            del kwargs
            observed_range = request.get_header("Range")
            return _Response(content[5:], status=206)

        with tempfile.TemporaryDirectory() as directory:
            model_dir = Path(directory) / "model"
            model_dir.mkdir()
            (model_dir / "weights.bin.part").write_bytes(content[:5])

            sync_model_artifacts(lock, model_dir, opener=opener)

            self.assertEqual(observed_range, "bytes=5-")
            self.assertEqual((model_dir / "weights.bin").read_bytes(), content)

    def test_rejects_a_response_before_it_writes_past_the_locked_size(self) -> None:
        content = b"lock"
        lock = _lock(content)

        def opener(request: Request, **kwargs: object) -> _Response:
            del request, kwargs
            return _Response(content + b"x" * 100, status=200)

        with tempfile.TemporaryDirectory() as directory:
            model_dir = Path(directory) / "model"

            with self.assertRaises(ModelArtifactError):
                sync_model_artifacts(lock, model_dir, opener=opener)

            self.assertFalse((model_dir / "weights.bin").exists())
            self.assertFalse((model_dir / "weights.bin.part").exists())

    def test_redirects_stay_on_https_hugging_face_distribution_hosts(self) -> None:
        handler = _PinnedArtifactRedirectHandler()
        request = Request("https://huggingface.co/example/model/resolve/revision/file")

        redirected = handler.redirect_request(
            request,
            None,
            302,
            "Found",
            {},
            "https://cdn-lfs.hf.co/example/file",
        )

        assert redirected is not None
        self.assertEqual(redirected.full_url, "https://cdn-lfs.hf.co/example/file")
        for unsafe in (
            "http://cdn-lfs.hf.co/example/file",
            "https://127.0.0.1/internal",
            "https://169.254.169.254/latest/meta-data",
            "https://example.invalid/file",
            "https://user:secret@huggingface.co/file",
        ):
            with self.subTest(unsafe=unsafe):
                with self.assertRaises(ModelArtifactError):
                    handler.redirect_request(
                        request,
                        None,
                        302,
                        "Found",
                        {},
                        unsafe,
                    )


if __name__ == "__main__":
    unittest.main()
