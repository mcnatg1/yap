import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SERVER_LAUNCH = (
    REPO_ROOT / "infra" / "yap-server-node" / "phase5-batch-server.sh"
)


class Phase5BatchRuntimeContractTests(unittest.TestCase):
    def test_launch_is_clean_head_foreground_and_loopback_only(self) -> None:
        script = SERVER_LAUNCH.read_text(encoding="utf-8")

        self.assertIn('git -C "$repo_root" rev-parse --is-inside-work-tree', script)
        self.assertIn('git -C "$repo_root" rev-parse HEAD', script)
        self.assertIn("status --porcelain=v1 --untracked-files=normal", script)
        self.assertIn("python3.12 -m yap_server", script)
        self.assertIn("exec env", script)
        self.assertIn("YAP_SERVER_HOST=127.0.0.1", script)
        self.assertIn("YAP_SERVER_PORT=18765", script)
        self.assertIn("YAP_PHASE5_BATCH_ENABLED=1", script)
        self.assertIn('YAP_PHASE5_CHECKED_HEAD="$YAP_CHECKED_HEAD"', script)
        self.assertIn('YAP_PHASE5_WORKER_IMAGE="$YAP_PHASE5_WORKER_IMAGE"', script)
        self.assertIn('YAP_PHASE5_MODEL_LOCK="$YAP_PHASE5_MODEL_LOCK"', script)
        self.assertIn('YAP_PHASE5_MODEL_DIR="$YAP_PHASE5_MODEL_DIR"', script)
        self.assertIn('YAP_PHASE5_STORAGE_DIR="$YAP_PHASE5_STORAGE_DIR"', script)
        self.assertIn('storage_mode="$(stat -Lc \'%a\'', script)
        self.assertIn('if [ "$storage_mode" != "700" ]', script)
        for forbidden in (
            "0.0.0.0",
            "localhost",
            "nohup",
            "systemctl",
            "ufw ",
            "--publish",
        ):
            self.assertNotIn(forbidden, script)

    def test_launch_requires_the_checked_head_custom_worker_inputs(self) -> None:
        script = SERVER_LAUNCH.read_text(encoding="utf-8")

        for required in (
            "YAP_CHECKED_HEAD:?",
            "YAP_PHASE5_MODEL_DIR:?",
            "YAP_PHASE5_STORAGE_DIR:?",
            "YAP_PHASE5_WORKER_IMAGE:?",
        ):
            self.assertIn(required, script)
        self.assertNotIn("nvcr.io/nvidia/pytorch", script)
        self.assertNotIn("model-pools.lock.json@", script)


if __name__ == "__main__":
    unittest.main()
