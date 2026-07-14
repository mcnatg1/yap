from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
from typing import Any


_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_REVISION = re.compile(r"^[0-9a-f]{40}$")


class ModelArtifactError(RuntimeError):
    """Raised when a model or fixture differs from its immutable lock."""


@dataclass(frozen=True)
class LockedArtifact:
    path: str
    size: int
    sha256: str


@dataclass(frozen=True)
class LockedFixture:
    path: str
    source: str
    license: str
    sha256: str
    golden_transcript: str


@dataclass(frozen=True)
class ModelPoolLock:
    schema_version: int
    runtime_image: str
    runtime_source: str
    runtime_license: str
    runtime_platform: str
    runtime_digest: str
    runtime_source_tag: str
    runtime_python_version: str
    runtime_torch_version: str
    runtime_cuda_version: str
    runtime_torch_cuda_version: str
    runtime_overlay_packages: tuple[tuple[str, str], ...]
    pool_id: str
    model_id: str
    model_revision: str
    model_license: str
    model_source: str
    model_distribution_id: str
    model_distribution_revision: str
    model_distribution_source: str
    model_distribution_provenance: str
    supported_languages: tuple[str, ...]
    artifacts: tuple[LockedArtifact, ...]
    fixture: LockedFixture


def _mapping(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be an object")
    return value


def _string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


def _sha256(value: Any, field: str) -> str:
    text = _string(value, field)
    if not _SHA256.fullmatch(text):
        raise ValueError(f"{field} must be a lowercase SHA-256")
    return text


def load_model_pool_lock(path: Path) -> ModelPoolLock:
    payload = _mapping(json.loads(path.read_text(encoding="utf-8")), "root")
    if payload.get("schemaVersion") != 1:
        raise ValueError("unsupported model-pool lock schema")

    runtime = _mapping(payload.get("runtime"), "runtime")
    runtime_image = _string(runtime.get("image"), "runtime.image")
    runtime_source = _string(runtime.get("source"), "runtime.source")
    runtime_license = _string(runtime.get("license"), "runtime.license")
    runtime_platform = _string(runtime.get("platform"), "runtime.platform")
    runtime_digest = _string(runtime.get("digest"), "runtime.digest")
    runtime_source_tag = _string(runtime.get("sourceTag"), "runtime.sourceTag")
    runtime_python_version = _string(
        runtime.get("pythonVersion"),
        "runtime.pythonVersion",
    )
    runtime_torch_version = _string(
        runtime.get("torchVersion"),
        "runtime.torchVersion",
    )
    runtime_cuda_version = _string(
        runtime.get("cudaVersion"),
        "runtime.cudaVersion",
    )
    runtime_torch_cuda_version = _string(
        runtime.get("torchCudaVersion"),
        "runtime.torchCudaVersion",
    )
    raw_overlay_packages = _mapping(
        runtime.get("overlayPackages"),
        "runtime.overlayPackages",
    )
    if not raw_overlay_packages:
        raise ValueError("runtime.overlayPackages must not be empty")
    overlay_packages: list[tuple[str, str]] = []
    for package_name, package_version in raw_overlay_packages.items():
        if not isinstance(package_name, str) or not re.fullmatch(
            r"[a-z0-9][a-z0-9._-]*",
            package_name,
        ):
            raise ValueError("runtime.overlayPackages contains an invalid package name")
        overlay_packages.append(
            (
                package_name,
                _string(
                    package_version,
                    f"runtime.overlayPackages.{package_name}",
                ),
            )
        )
    if runtime_platform != "linux/arm64":
        raise ValueError("the Phase 4 runtime must be pinned for linux/arm64")
    if runtime_image.endswith(":latest"):
        raise ValueError("runtime image cannot use latest")
    if runtime_source_tag == "latest":
        raise ValueError("runtime source tag cannot use latest")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", runtime_digest):
        raise ValueError("runtime.digest must be a lowercase SHA-256 digest")
    if not re.fullmatch(r"[0-9]+\.[0-9]+", runtime_python_version):
        raise ValueError("runtime.pythonVersion must be a major.minor version")
    if not re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", runtime_cuda_version):
        raise ValueError("runtime.cudaVersion must be a numeric CUDA version")
    if not re.fullmatch(
        r"[0-9]+\.[0-9]+(?:\.[0-9]+)?",
        runtime_torch_cuda_version,
    ):
        raise ValueError("runtime.torchCudaVersion must be a numeric CUDA version")

    pool = _mapping(payload.get("pool"), "pool")
    pool_id = _string(pool.get("id"), "pool.id")
    model = _mapping(pool.get("model"), "pool.model")
    model_id = _string(model.get("id"), "pool.model.id")
    model_revision = _string(model.get("revision"), "pool.model.revision")
    if not _REVISION.fullmatch(model_revision):
        raise ValueError("pool.model.revision must be a full immutable commit")
    distribution = _mapping(model.get("distribution"), "pool.model.distribution")
    model_distribution_revision = _string(
        distribution.get("revision"),
        "pool.model.distribution.revision",
    )
    if not _REVISION.fullmatch(model_distribution_revision):
        raise ValueError(
            "pool.model.distribution.revision must be a full immutable commit"
        )

    raw_languages = pool.get("supportedLanguages")
    if not isinstance(raw_languages, list) or not raw_languages:
        raise ValueError("pool.supportedLanguages must be a non-empty array")
    if not all(
        isinstance(language, str) and re.fullmatch(r"[a-z]{2}", language)
        for language in raw_languages
    ):
        raise ValueError("pool.supportedLanguages must contain ISO 639-1 codes")
    if raw_languages != sorted(set(raw_languages)):
        raise ValueError("pool.supportedLanguages must be sorted and unique")

    raw_artifacts = pool.get("artifacts")
    if not isinstance(raw_artifacts, list) or not raw_artifacts:
        raise ValueError("pool.artifacts must be a non-empty array")
    artifacts: list[LockedArtifact] = []
    seen: set[str] = set()
    for index, raw in enumerate(raw_artifacts):
        item = _mapping(raw, f"pool.artifacts[{index}]")
        artifact_path = _string(item.get("path"), f"pool.artifacts[{index}].path")
        parsed_path = PurePosixPath(artifact_path)
        if (
            parsed_path.is_absolute()
            or len(parsed_path.parts) != 1
            or parsed_path.name in ("", ".", "..")
        ):
            raise ValueError("model artifact paths must be single safe file names")
        if artifact_path in seen:
            raise ValueError("model artifact paths must be unique")
        size = item.get("size")
        if not isinstance(size, int) or isinstance(size, bool) or size < 1:
            raise ValueError("model artifact sizes must be positive integers")
        artifacts.append(
            LockedArtifact(
                path=artifact_path,
                size=size,
                sha256=_sha256(
                    item.get("sha256"),
                    f"pool.artifacts[{index}].sha256",
                ),
            )
        )
        seen.add(artifact_path)

    raw_fixture = _mapping(payload.get("fixture"), "fixture")
    fixture_path = _string(raw_fixture.get("path"), "fixture.path")
    parsed_fixture_path = PurePosixPath(fixture_path)
    if parsed_fixture_path.is_absolute() or ".." in parsed_fixture_path.parts:
        raise ValueError("fixture.path must stay inside the repository")
    fixture = LockedFixture(
        path=fixture_path,
        source=_string(raw_fixture.get("source"), "fixture.source"),
        license=_string(raw_fixture.get("license"), "fixture.license"),
        sha256=_sha256(raw_fixture.get("sha256"), "fixture.sha256"),
        golden_transcript=_string(
            raw_fixture.get("goldenTranscript"),
            "fixture.goldenTranscript",
        ),
    )
    return ModelPoolLock(
        schema_version=1,
        runtime_image=runtime_image,
        runtime_source=runtime_source,
        runtime_license=runtime_license,
        runtime_platform=runtime_platform,
        runtime_digest=runtime_digest,
        runtime_source_tag=runtime_source_tag,
        runtime_python_version=runtime_python_version,
        runtime_torch_version=runtime_torch_version,
        runtime_cuda_version=runtime_cuda_version,
        runtime_torch_cuda_version=runtime_torch_cuda_version,
        runtime_overlay_packages=tuple(sorted(overlay_packages)),
        pool_id=pool_id,
        model_id=model_id,
        model_revision=model_revision,
        model_license=_string(model.get("license"), "pool.model.license"),
        model_source=_string(model.get("source"), "pool.model.source"),
        model_distribution_id=_string(
            distribution.get("id"),
            "pool.model.distribution.id",
        ),
        model_distribution_revision=model_distribution_revision,
        model_distribution_source=_string(
            distribution.get("source"),
            "pool.model.distribution.source",
        ),
        model_distribution_provenance=_string(
            distribution.get("provenance"),
            "pool.model.distribution.provenance",
        ),
        supported_languages=tuple(raw_languages),
        artifacts=tuple(artifacts),
        fixture=fixture,
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(4 * 1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_model_artifacts(lock: ModelPoolLock, model_dir: Path) -> None:
    root = model_dir.resolve(strict=True)
    if not root.is_dir():
        raise ModelArtifactError("model root is not a directory")
    for artifact in lock.artifacts:
        candidate = root / artifact.path
        try:
            metadata = candidate.lstat()
        except FileNotFoundError as error:
            raise ModelArtifactError(f"missing locked artifact: {artifact.path}") from error
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise ModelArtifactError(f"locked artifact is not a regular file: {artifact.path}")
        if metadata.st_size != artifact.size:
            raise ModelArtifactError(f"locked artifact size differs: {artifact.path}")
        if sha256_file(candidate) != artifact.sha256:
            raise ModelArtifactError(f"locked artifact digest differs: {artifact.path}")


def verify_fixture(lock: ModelPoolLock, repo_root: Path) -> Path:
    root = repo_root.resolve(strict=True)
    candidate = root / Path(lock.fixture.path)
    try:
        metadata = candidate.lstat()
    except FileNotFoundError as error:
        raise ModelArtifactError("locked ASR fixture is missing") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ModelArtifactError("locked ASR fixture is not a regular file")
    resolved = candidate.resolve(strict=True)
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise ModelArtifactError("locked ASR fixture escapes the repository") from error
    if sha256_file(candidate) != lock.fixture.sha256:
        raise ModelArtifactError("locked ASR fixture digest differs")
    return resolved
