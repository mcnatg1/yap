from __future__ import annotations

import os
from dataclasses import dataclass
from ipaddress import ip_address
from typing import Mapping


DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 18765
PRIVATE_BIND_OPT_IN = "YAP_SERVER_ALLOW_PRIVATE_BIND"


def _is_loopback(host: str) -> bool:
    if host.casefold().rstrip(".") == "localhost":
        return True
    try:
        return ip_address(host).is_loopback
    except ValueError:
        return False


def ensure_bind_is_allowed(
    host: str,
    environ: Mapping[str, str] | None = None,
) -> None:
    source = os.environ if environ is None else environ
    if _is_loopback(host):
        return
    if source.get(PRIVATE_BIND_OPT_IN) == "1":
        return
    raise ValueError(
        f"YAP_SERVER_HOST must be loopback unless {PRIVATE_BIND_OPT_IN}=1"
    )


@dataclass(frozen=True, slots=True)
class ServerSettings:
    host: str = DEFAULT_HOST
    port: int = DEFAULT_PORT

    def __post_init__(self) -> None:
        if not isinstance(self.host, str) or not self.host.strip():
            raise ValueError("YAP_SERVER_HOST must not be empty")
        if isinstance(self.port, bool) or not isinstance(self.port, int):
            raise ValueError("YAP_SERVER_PORT must be an integer")
        if not 0 <= self.port <= 65535:
            raise ValueError("YAP_SERVER_PORT must be between 0 and 65535")

    @classmethod
    def from_env(cls) -> ServerSettings:
        host = os.environ.get("YAP_SERVER_HOST", DEFAULT_HOST).strip()
        port_text = os.environ.get("YAP_SERVER_PORT", str(DEFAULT_PORT)).strip()
        try:
            port = int(port_text, 10)
        except ValueError as error:
            raise ValueError("YAP_SERVER_PORT must be an integer") from error

        settings = cls(host=host, port=port)
        ensure_bind_is_allowed(settings.host)
        return settings
