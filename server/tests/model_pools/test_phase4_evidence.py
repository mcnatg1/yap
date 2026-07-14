import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from yap_server.pools.phase4_evidence import finalize_host_boundary_evidence


SNAPSHOT_FILES = {
    "listeners.txt": b"tcp LISTEN 0 128 0.0.0.0:22\n",
    "firewall.txt": b"ufw\nStatus: active\n",
    "services.txt": b"",
    "containers.txt": b"",
    "workers.txt": b"",
}


def _write_snapshot(root: Path, values: dict[str, bytes] = SNAPSHOT_FILES) -> None:
    root.mkdir()
    for name, value in values.items():
        (root / name).write_bytes(value)


class Phase4EvidenceTests(unittest.TestCase):
    def test_publishes_only_after_host_boundary_is_observed_unchanged(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before = root / "before"
            after = root / "after"
            _write_snapshot(before)
            _write_snapshot(after)
            inference_result = root / "inference-result.json"
            result_bytes = b'{"schemaVersion":1}\n'
            inference_result.write_bytes(result_bytes)
            inference_evidence = root / "inference-evidence.json"
            inference_evidence.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "resultSha256": hashlib.sha256(result_bytes).hexdigest(),
                        "boundary": {
                            "network": "none",
                            "workerCount": 1,
                        },
                    }
                ),
                encoding="utf-8",
            )
            result = root / "result.json"
            evidence = root / "evidence.json"

            finalized = finalize_host_boundary_evidence(
                before_dir=before,
                after_dir=after,
                inference_result_path=inference_result,
                inference_evidence_path=inference_evidence,
                result_path=result,
                evidence_path=evidence,
            )

            self.assertEqual(result.read_bytes(), result_bytes)
            self.assertEqual(json.loads(evidence.read_text(encoding="utf-8")), finalized)
            boundary = finalized["boundary"]
            self.assertEqual(boundary["ports"], [])
            self.assertFalse(boundary["persistentService"])
            self.assertEqual(boundary["hostObservation"], "verified")
            observed = boundary["observedHostBoundary"]
            self.assertTrue(observed["listenerStateUnchanged"])
            self.assertTrue(observed["firewallStateUnchanged"])
            self.assertTrue(observed["serviceUnitsUnchanged"])
            self.assertEqual(observed["remainingPhase4Containers"], 0)
            self.assertEqual(observed["remainingWorkerProcesses"], 0)

    def test_rejects_a_listener_or_firewall_change(self) -> None:
        for changed_file in ("listeners.txt", "firewall.txt"):
            with self.subTest(changed_file=changed_file):
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    before = root / "before"
                    after = root / "after"
                    _write_snapshot(before)
                    changed = dict(SNAPSHOT_FILES)
                    changed[changed_file] = SNAPSHOT_FILES[changed_file] + b"changed\n"
                    _write_snapshot(after, changed)
                    inference_result, inference_evidence = self._inference_files(root)

                    with self.assertRaises(RuntimeError):
                        finalize_host_boundary_evidence(
                            before_dir=before,
                            after_dir=after,
                            inference_result_path=inference_result,
                            inference_evidence_path=inference_evidence,
                            result_path=root / "result.json",
                            evidence_path=root / "evidence.json",
                        )

    def test_rejects_a_residual_phase4_container_or_worker(self) -> None:
        for changed_file in ("containers.txt", "workers.txt"):
            with self.subTest(changed_file=changed_file):
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    before = root / "before"
                    after = root / "after"
                    _write_snapshot(before)
                    changed = dict(SNAPSHOT_FILES)
                    changed[changed_file] = b"left-behind\n"
                    _write_snapshot(after, changed)
                    inference_result, inference_evidence = self._inference_files(root)

                    with self.assertRaises(RuntimeError):
                        finalize_host_boundary_evidence(
                            before_dir=before,
                            after_dir=after,
                            inference_result_path=inference_result,
                            inference_evidence_path=inference_evidence,
                            result_path=root / "result.json",
                            evidence_path=root / "evidence.json",
                        )

    @staticmethod
    def _inference_files(root: Path) -> tuple[Path, Path]:
        result = root / "inference-result.json"
        result_bytes = b'{"schemaVersion":1}\n'
        result.write_bytes(result_bytes)
        evidence = root / "inference-evidence.json"
        evidence.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "resultSha256": hashlib.sha256(result_bytes).hexdigest(),
                    "boundary": {"network": "none", "workerCount": 1},
                }
            ),
            encoding="utf-8",
        )
        return result, evidence


if __name__ == "__main__":
    unittest.main()
