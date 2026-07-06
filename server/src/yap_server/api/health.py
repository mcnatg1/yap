from dataclasses import asdict, dataclass


@dataclass(frozen=True)
class HealthView:
    status: str = "ok"
    service: str = "yap-server"
    version: str = "0.1.0"


def health() -> dict[str, str]:
    return asdict(HealthView())

