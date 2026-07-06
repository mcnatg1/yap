from dataclasses import dataclass
from typing import Literal


WorkloadKind = Literal["live", "batch"]
RouteTarget = Literal["streaming-asr", "batch-asr"]


@dataclass(frozen=True)
class WorkloadRoute:
    kind: WorkloadKind
    target: RouteTarget


class WorkloadRouter:
    def route(self, kind: WorkloadKind) -> WorkloadRoute:
        if kind == "live":
            return WorkloadRoute(kind=kind, target="streaming-asr")
        if kind == "batch":
            return WorkloadRoute(kind=kind, target="batch-asr")
        raise ValueError(f"Unsupported workload kind: {kind}")

