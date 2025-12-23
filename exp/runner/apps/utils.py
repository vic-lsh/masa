"""
Shared utilities for application plugins.
"""

import re
from typing import Optional


def normalize_features_to_tag(features: Optional[str]) -> str:
    """
    Normalize cargo feature flags into a deterministic, valid docker tag.

    Cargo features are comma-separated (e.g., "feat-b,feat-a").
    Docker tags must be lowercase alphanumeric with periods, dashes, or underscores.

    Args:
        features: Comma-separated cargo features or None

    Returns:
        A normalized docker tag string (e.g., "feat-a-feat-b" or "latest")
    """
    if not features or features.strip() == "":
        return "latest"

    # Split by comma, strip whitespace, and sort for determinism
    feature_list = [f.strip() for f in features.split(",")]
    feature_list = [f for f in feature_list if f]  # Remove empty strings

    if not feature_list:
        return "latest"

    # Sort for determinism (case-insensitive for consistency)
    feature_list.sort(key=str.lower)

    # Join with dashes, ensuring valid docker tag characters
    # Replace any invalid characters with dashes
    tag = "-".join(feature_list)

    # Docker tags: lowercase alphanumeric, periods, dashes, underscores only
    # Also ensure it doesn't start with a period or dash
    tag = re.sub(r"[^a-zA-Z0-9._-]", "-", tag)
    tag = tag.lower()
    tag = re.sub(r"^[.-]+", "", tag)  # Remove leading periods or dashes
    tag = re.sub(r"-+", "-", tag)  # Collapse multiple dashes

    return tag if tag else "latest"





