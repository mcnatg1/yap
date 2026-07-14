import unittest

from yap_server.workload_router import (
    DuplicateWorkload,
    RouterBackpressure,
    WorkloadRequest,
    WorkloadRouter,
)


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

    def test_live_work_preempts_queued_batch_work(self) -> None:
        router = WorkloadRouter()
        router.enqueue(WorkloadRequest("batch-1", "owner-a", "batch"))
        router.enqueue(WorkloadRequest("live-1", "owner-b", "live"))

        dispatched = router.dispatch()

        self.assertIsNotNone(dispatched)
        assert dispatched is not None
        self.assertEqual(dispatched.request.job_id, "live-1")
        self.assertEqual(dispatched.route.target, "streaming-asr")

    def test_batch_work_runs_after_a_bounded_live_priority_streak(self) -> None:
        router = WorkloadRouter(
            max_pending=4,
            max_pending_per_owner=3,
            max_consecutive_live=2,
        )
        router.enqueue(WorkloadRequest("batch-1", "batch-owner", "batch"))
        router.enqueue(WorkloadRequest("live-1", "live-owner", "live"))
        router.enqueue(WorkloadRequest("live-2", "live-owner", "live"))
        router.enqueue(WorkloadRequest("live-3", "live-owner", "live"))

        dispatched = [router.dispatch(), router.dispatch(), router.dispatch()]

        self.assertTrue(all(item is not None for item in dispatched))
        self.assertEqual(
            [item.request.job_id for item in dispatched if item is not None],
            ["live-1", "live-2", "batch-1"],
        )

    def test_round_robins_owners_within_a_pool(self) -> None:
        router = WorkloadRouter()
        router.enqueue(WorkloadRequest("a-1", "owner-a", "batch"))
        router.enqueue(WorkloadRequest("a-2", "owner-a", "batch"))
        router.enqueue(WorkloadRequest("b-1", "owner-b", "batch"))
        router.enqueue(WorkloadRequest("b-2", "owner-b", "batch"))

        job_ids = []
        for _ in range(4):
            dispatched = router.dispatch()
            self.assertIsNotNone(dispatched)
            assert dispatched is not None
            job_ids.append(dispatched.request.job_id)

        self.assertEqual(job_ids, ["a-1", "b-1", "a-2", "b-2"])

    def test_dispatches_only_to_an_available_pool(self) -> None:
        router = WorkloadRouter()
        router.enqueue(WorkloadRequest("live-1", "owner-a", "live"))
        router.enqueue(WorkloadRequest("batch-1", "owner-b", "batch"))

        dispatched = router.dispatch(available_targets={"batch-asr"})

        self.assertIsNotNone(dispatched)
        assert dispatched is not None
        self.assertEqual(dispatched.request.job_id, "batch-1")
        self.assertEqual(router.pending_count, 1)

    def test_applies_total_and_per_owner_backpressure(self) -> None:
        router = WorkloadRouter(max_pending=3, max_pending_per_owner=2)
        router.enqueue(WorkloadRequest("a-1", "owner-a", "batch"))
        router.enqueue(WorkloadRequest("a-2", "owner-a", "batch"))

        with self.assertRaises(RouterBackpressure):
            router.enqueue(WorkloadRequest("a-3", "owner-a", "batch"))

        router.enqueue(WorkloadRequest("b-1", "owner-b", "batch"))
        with self.assertRaises(RouterBackpressure):
            router.enqueue(WorkloadRequest("c-1", "owner-c", "batch"))

    def test_rejects_duplicate_pending_job_id(self) -> None:
        router = WorkloadRouter()
        request = WorkloadRequest("batch-1", "owner-a", "batch")
        router.enqueue(request)

        with self.assertRaises(DuplicateWorkload):
            router.enqueue(request)

    def test_validates_internal_queue_identity(self) -> None:
        with self.assertRaises(ValueError):
            WorkloadRequest("../job", "owner-a", "batch")
        with self.assertRaises(ValueError):
            WorkloadRequest("job-1", "", "batch")


if __name__ == "__main__":
    unittest.main()
