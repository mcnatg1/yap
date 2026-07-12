from dataclasses import dataclass
from typing import Literal


@dataclass(frozen=True, slots=True)
class ServerCapabilities:
    batch_jobs: bool
    live_streaming: bool
    job_status: bool

    def to_wire(self) -> dict[str, bool]:
        return {
            "batchJobs": self.batch_jobs,
            "liveStreaming": self.live_streaming,
            "jobStatus": self.job_status,
        }


@dataclass(frozen=True, slots=True)
class HealthView:
    service: str
    status: Literal["ok"]
    api_version: str
    auth: Literal["not_configured", "required"]
    capabilities: ServerCapabilities

    def to_wire(self) -> dict[str, object]:
        return {
            "service": self.service,
            "status": self.status,
            "apiVersion": self.api_version,
            "auth": self.auth,
            "capabilities": self.capabilities.to_wire(),
        }
