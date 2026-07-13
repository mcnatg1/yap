"""Dependency-free server wire shapes. No model weights live here."""

from .contract import HealthView, ServerCapabilities

__all__ = ["HealthView", "ServerCapabilities"]
