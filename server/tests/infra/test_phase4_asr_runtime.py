import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
PYPROJECT = REPO_ROOT / "server" / "pyproject.toml"
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"
LOCK_PATH = REPO_ROOT / "server" / "model-pools.lock.json"
DOCKERFILE = REPO_ROOT / "server" / "runtime" / "asr" / "Dockerfile"
DOCKERIGNORE = REPO_ROOT / "server" / ".dockerignore"
REQUIREMENTS = REPO_ROOT / "server" / "runtime" / "asr" / "requirements.lock"
RUNTIME_NOTICE = REPO_ROOT / "server" / "runtime" / "asr" / "THIRD_PARTY_NOTICES.md"
MODEL_LICENSE = (
    REPO_ROOT
    / "server"
    / "runtime"
    / "asr"
    / "licenses"
    / "COHERE_TRANSCRIBE_APACHE-2.0.txt"
)
GATE = REPO_ROOT / "infra" / "yap-server-node" / "phase4-asr-gate.sh"


class Phase4AsrRuntimeContractTests(unittest.TestCase):
    def test_server_and_hosted_checks_pin_python_312(self) -> None:
        pyproject = PYPROJECT.read_text(encoding="utf-8")
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn('requires-python = ">=3.12,<3.13"', pyproject)
        self.assertEqual(workflow.count('python-version: "3.12"'), 2)
        self.assertNotIn('python-version: "3.13"', workflow)

    def test_container_uses_the_locked_arm64_base_digest(self) -> None:
        lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
        dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        expected = lock["runtime"]["image"] + "@" + lock["runtime"]["digest"]

        self.assertIn(f"FROM {expected}", dockerfile)
        self.assertNotIn(":latest", dockerfile)
        self.assertEqual(lock["runtime"]["sourceTag"], "26.06-py3")
        self.assertEqual(lock["runtime"]["pythonVersion"], "3.12")
        self.assertEqual(
            lock["runtime"]["torchVersion"],
            "2.13.0a0+8145d630e8.nv26.06",
        )
        self.assertEqual(lock["runtime"]["cudaVersion"], "13.3.0")
        self.assertEqual(lock["runtime"]["torchCudaVersion"], "13.3")
        self.assertIn("sys.version_info[:2] == (3, 12)", dockerfile)
        self.assertIn("CohereAsrForConditionalGeneration", dockerfile)
        self.assertIn("import librosa, soundfile", dockerfile)
        self.assertIn('["runtime"]["overlayPackages"]', dockerfile)
        self.assertIn(
            "COPY runtime/asr/THIRD_PARTY_NOTICES.md "
            "/opt/yap-server/THIRD_PARTY_NOTICES.md",
            dockerfile,
        )
        for license_id in (
            "Apache-2.0",
            "BSD-3-Clause",
            "ISC",
            "LGPL-2.1-or-later",
            "MIT",
            "LicenseRef-NVIDIA-AI",
        ):
            self.assertIn(license_id, dockerfile)
        self.assertNotIn("AutoModelForTDT", dockerfile)
        self.assertIn(
            "triton-kernels 1.0.0+gitb7fa781f.nv26.6 requires pytest",
            dockerfile,
        )
        self.assertNotIn("pip check || true", dockerfile)
        self.assertIn("USER 10001:10001", dockerfile)
        self.assertIn("YAP_MODEL_LOCK", dockerfile)

    def test_python_runtime_wheels_are_version_and_hash_locked(self) -> None:
        requirements = REQUIREMENTS.read_text(encoding="utf-8")
        expected_packages = {
            "audioread": "3.1.0",
            "joblib": "1.5.3",
            "lazy-loader": "0.5",
            "librosa": "0.11.0",
            "msgpack": "1.2.1",
            "narwhals": "2.24.0",
            "pooch": "1.9.0",
            "scikit-learn": "1.9.0",
            "sentencepiece": "0.2.1",
            "soundfile": "0.14.0",
            "soxr": "1.1.0",
            "threadpoolctl": "3.6.0",
            "tokenizers": "0.22.2",
            "transformers": "5.13.1",
        }
        locked_packages = {}
        for line in requirements.splitlines():
            if not line.endswith(" \\"):
                continue
            name, version = line[:-2].split("==", maxsplit=1)
            locked_packages[name] = version
        self.assertEqual(locked_packages, expected_packages)
        self.assertNotIn("huggingface-hub==", requirements)
        self.assertNotIn("torch==", requirements)
        self.assertNotIn(">=", requirements)
        requirement_lines = [
            line for line in requirements.splitlines()
            if line and not line.startswith(("#", " ", "--hash"))
        ]
        self.assertEqual(len(requirement_lines), len(expected_packages))
        self.assertEqual(
            len(re.findall(r"--hash=sha256:[0-9a-f]{64}", requirements)),
            len(expected_packages),
        )

    def test_gate_observes_host_boundary_and_publishes_only_after_teardown(self) -> None:
        script = GATE.read_text(encoding="utf-8")
        self.assertIn("docker build", script)
        self.assertIn("phase4_gate", script)
        self.assertIn("phase4_evidence", script)
        self.assertIn(
            'git -C "$repo_root" rev-parse --is-inside-work-tree',
            script,
        )
        self.assertIn("status --porcelain=v1 --untracked-files=normal", script)
        self.assertNotIn('if [ -d "$repo_root/.git" ]', script)
        self.assertNotIn("--restart", script)
        for line in script.splitlines():
            option = line.strip()
            self.assertFalse(option.startswith(("-p ", "-p=", "--publish")))
        self.assertIn("ss -H -lntu", script)
        self.assertIn("docker ps -a", script)
        self.assertIn("pgrep -af", script)
        self.assertIn("sudo -n ufw status verbose", script)
        self.assertIn("systemctl list-unit-files", script)
        self.assertLess(script.index('capture_host_boundary "$gate_tmp/before"'), script.index("docker build"))
        self.assertLess(script.index("phase4_gate"), script.index('capture_host_boundary "$gate_tmp/after"'))
        self.assertLess(script.index('capture_host_boundary "$gate_tmp/after"'), script.index("phase4_evidence"))
        for mutation in (
            "ufw allow",
            "ufw delete",
            "ufw enable",
            "systemctl enable",
            "systemctl start",
            "systemctl restart",
        ):
            self.assertNotIn(mutation, script.lower())

    def test_worker_build_context_excludes_ignored_executable_and_private_state(self) -> None:
        patterns = set(DOCKERIGNORE.read_text(encoding="utf-8").splitlines())

        for pattern in (
            "**/__pycache__/",
            "**/*.py[cod]",
            ".venv/",
            ".env*",
            "data/",
            "logs/",
            "models/",
            "volumes/",
            ".runtime/",
        ):
            self.assertIn(pattern, patterns)

    def test_runtime_notice_covers_the_locked_overlay_and_native_libraries(self) -> None:
        lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
        notice = RUNTIME_NOTICE.read_text(encoding="utf-8")

        for package_name in lock["runtime"]["overlayPackages"]:
            self.assertIn(f"`{package_name}`", notice)
        self.assertIn("libsndfile", notice)
        self.assertIn("LGPL-2.1", notice)

    def test_container_carries_the_model_license_text(self) -> None:
        dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        license_text = MODEL_LICENSE.read_text(encoding="utf-8")

        self.assertIn("Apache License", license_text)
        self.assertIn("Version 2.0, January 2004", license_text)
        self.assertIn(
            "COPY runtime/asr/licenses /opt/yap-server/licenses",
            dockerfile,
        )


if __name__ == "__main__":
    unittest.main()
