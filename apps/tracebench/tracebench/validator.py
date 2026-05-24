from __future__ import annotations

from .trace_config import TraceConfig


class ValidationError(RuntimeError):
    """Raised when the trace configuration fails validation."""


def validate_config(config: TraceConfig) -> None:
    if not config.call_graph.services():
        raise ValidationError("Configuration must define at least one service")

