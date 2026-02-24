"""
Centralized utilities for experiment runner naming conventions.

This module provides consistent logic for generating project names and extracting
service names, ensuring all applications (hotel, mssim, synthetic, socialnet)
follow the same conventions.
"""

import hashlib
import re
from typing import Optional


def generate_project_name(
    *,
    prefix: str,
    experiment_name: str,
    iteration: int,
    policy: str,
    extra_suffix: Optional[str] = None,
) -> str:
    """
    Generate a safe, deterministic Docker Compose project name.

    Format: {prefix}-{slug}-{digest}

    Args:
        prefix: Application prefix (e.g., "hotel", "mssim", "syn", "socialnet")
        experiment_name: Name of the experiment
        iteration: Iteration number
        policy: Policy string (can contain special chars)
        extra_suffix: Optional extra string to include in the digest calculation
                      (e.g., RPS value for MSSIM)

    Returns:
        A project name safe for Docker Compose usage.
    """
    raw_parts = [experiment_name, str(iteration), policy]
    if extra_suffix:
        raw_parts.append(str(extra_suffix))

    raw = "|".join(raw_parts)

    # 12-char hex digest for uniqueness
    digest = hashlib.sha256(raw.encode("utf-8")).hexdigest()[:12]

    # Sanitized slug (max 12 chars)
    slug = re.sub(r"[^a-z0-9]+", "-", experiment_name.lower()).strip("-")[:12] or "exp"

    return f"{prefix}-{slug}-{digest}"
