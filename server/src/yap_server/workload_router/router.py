from collections import Counter, deque
from dataclasses import dataclass
import re
from typing import Collection, Literal


WorkloadKind = Literal["live", "batch"]
RouteTarget = Literal["streaming-asr", "batch-asr"]
_WORKLOAD_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_OWNER_KEY = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:@-]{0,255}$")


@dataclass(frozen=True)
class WorkloadRoute:
    kind: WorkloadKind
    target: RouteTarget


@dataclass(frozen=True)
class WorkloadRequest:
    job_id: str
    owner_key: str
    kind: WorkloadKind

    def __post_init__(self) -> None:
        if not _WORKLOAD_ID.fullmatch(self.job_id):
            raise ValueError("job_id must be an opaque path-safe identifier")
        if not _OWNER_KEY.fullmatch(self.owner_key):
            raise ValueError("owner_key must be a non-empty internal queue key")
        if self.kind not in ("live", "batch"):
            raise ValueError(f"Unsupported workload kind: {self.kind}")


@dataclass(frozen=True)
class RoutedWorkload:
    request: WorkloadRequest
    route: WorkloadRoute


class RouterBackpressure(RuntimeError):
    """Raised when a bounded router queue cannot admit more work."""


class DuplicateWorkload(ValueError):
    """Raised when a pending job identifier is submitted twice."""


class WorkloadRouter:
    """Bounded in-memory dispatch with bounded live priority and owner fairness.

    ``owner_key`` is an internal queue key. It is deliberately not part of the
    Phase 3/5 wire request and must eventually be derived by the authenticated
    server boundary rather than accepted from a client payload.

    Live work is preferred while both targets are ready, but one batch job is
    forced after ``max_consecutive_live`` live dispatches so background work
    cannot starve under sustained interactive load.
    """

    def __init__(
        self,
        *,
        max_pending: int = 128,
        max_pending_per_owner: int = 16,
        max_consecutive_live: int = 8,
    ) -> None:
        if max_pending < 1 or max_pending_per_owner < 1 or max_consecutive_live < 1:
            raise ValueError("router queue limits must be positive")
        if max_pending_per_owner > max_pending:
            raise ValueError("per-owner limit cannot exceed the total limit")
        self._max_pending = max_pending
        self._max_pending_per_owner = max_pending_per_owner
        self._max_consecutive_live = max_consecutive_live
        self._consecutive_live_dispatches = 0
        self._queues: dict[WorkloadKind, dict[str, deque[WorkloadRequest]]] = {
            "live": {},
            "batch": {},
        }
        self._owner_order: dict[WorkloadKind, deque[str]] = {
            "live": deque(),
            "batch": deque(),
        }
        self._pending_ids: set[str] = set()
        self._pending_by_owner: Counter[str] = Counter()

    @property
    def pending_count(self) -> int:
        return len(self._pending_ids)

    def route(self, kind: WorkloadKind) -> WorkloadRoute:
        if kind == "live":
            return WorkloadRoute(kind=kind, target="streaming-asr")
        if kind == "batch":
            return WorkloadRoute(kind=kind, target="batch-asr")
        raise ValueError(f"Unsupported workload kind: {kind}")

    def enqueue(self, request: WorkloadRequest) -> WorkloadRoute:
        if request.job_id in self._pending_ids:
            raise DuplicateWorkload(f"workload {request.job_id!r} is already pending")
        if self.pending_count >= self._max_pending:
            raise RouterBackpressure("router total pending limit reached")
        if self._pending_by_owner[request.owner_key] >= self._max_pending_per_owner:
            raise RouterBackpressure("router per-owner pending limit reached")

        owner_queues = self._queues[request.kind]
        if request.owner_key not in owner_queues:
            owner_queues[request.owner_key] = deque()
            self._owner_order[request.kind].append(request.owner_key)
        owner_queues[request.owner_key].append(request)
        self._pending_ids.add(request.job_id)
        self._pending_by_owner[request.owner_key] += 1
        return self.route(request.kind)

    def dispatch(
        self,
        *,
        available_targets: Collection[RouteTarget] | None = None,
    ) -> RoutedWorkload | None:
        targets = (
            frozenset(available_targets)
            if available_targets is not None
            else frozenset(("streaming-asr", "batch-asr"))
        )
        live_route = self.route("live")
        batch_route = self.route("batch")
        live_ready = live_route.target in targets and bool(self._owner_order["live"])
        batch_ready = batch_route.target in targets and bool(self._owner_order["batch"])

        if (
            batch_ready
            and self._consecutive_live_dispatches >= self._max_consecutive_live
        ):
            request = self._pop_fair("batch")
            if request is not None:
                self._consecutive_live_dispatches = 0
                return RoutedWorkload(request=request, route=batch_route)
        if live_ready:
            request = self._pop_fair("live")
            if request is not None:
                if batch_ready:
                    self._consecutive_live_dispatches += 1
                else:
                    self._consecutive_live_dispatches = 0
                return RoutedWorkload(request=request, route=live_route)
        if batch_ready:
            request = self._pop_fair("batch")
            if request is not None:
                self._consecutive_live_dispatches = 0
                return RoutedWorkload(request=request, route=batch_route)
        return None

    def _pop_fair(self, kind: WorkloadKind) -> WorkloadRequest | None:
        owners = self._owner_order[kind]
        queues = self._queues[kind]
        if not owners:
            return None

        owner_key = owners.popleft()
        queue = queues[owner_key]
        request = queue.popleft()
        if queue:
            owners.append(owner_key)
        else:
            del queues[owner_key]

        self._pending_ids.remove(request.job_id)
        self._pending_by_owner[owner_key] -= 1
        if self._pending_by_owner[owner_key] == 0:
            del self._pending_by_owner[owner_key]
        return request
