import unittest

from yap_server.workload_router import WorkloadRouter


class WorkloadRouterTests(unittest.TestCase):
    def test_routes_live_to_streaming_asr_pool(self) -> None:
        route = WorkloadRouter().route("live")

        self.assertEqual(route.kind, "live")
        self.assertEqual(route.target, "streaming-asr")

    def test_routes_batch_to_batch_asr_pool(self) -> None:
        route = WorkloadRouter().route("batch")

        self.assertEqual(route.kind, "batch")
        self.assertEqual(route.target, "batch-asr")

    def test_rejects_unknown_workload_kind(self) -> None:
        with self.assertRaises(ValueError):
            WorkloadRouter().route("unknown")  # type: ignore[arg-type]


if __name__ == "__main__":
    unittest.main()

