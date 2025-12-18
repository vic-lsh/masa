from __future__ import annotations


def normalize_service_name(raw: str) -> str:
    """Normalize service names to match Docker-friendly formatting."""
    return raw.replace("_", "-").lower()


def ensure_str_path(path) -> str:
    """Convert Path-like values to strings without touching None."""
    if path is None:
        return path
    return str(path)

