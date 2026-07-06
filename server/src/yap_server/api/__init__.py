"""HTTP and WSS entrypoints."""

from yap_server.api.health import HealthView, health

__all__ = ["HealthView", "health"]
