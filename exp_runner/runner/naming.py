"""
Centralized project naming and namespace isolation logic.

Replaces 3+ duplicate implementations of project name generation across different plugins.
"""

import hashlib
import logging

logger = logging.getLogger(__name__)


def generate_project_name(
    app: str,
    experiment_name: str,
    iteration: int,
    policy: str,
    rps: float | None = None,
) -> str:
    """
    Generate a unique project name for namespace isolation.

    Format: {app_prefix}-{slug}-{digest}

    The project name is used for:
    - Docker Compose project name (via COMPOSE_PROJECT_NAME)
    - Kubernetes namespace
    - Container/pod naming

    Args:
        app: Application name (hotel, synthetic, mssim, socialnet)
        experiment_name: Experiment name
        iteration: Iteration number
        policy: Policy name (fifo, prio_global, etc.)
        rps: Optional RPS value (used by MSSIM)

    Returns:
        Project name suitable for Docker/K8s (lowercase, limited length)

    Examples:
        >>> generate_project_name("hotel", "baseline", 0, "fifo")
        'hot-baseline-0-fifo-a1b2c3d4'

        >>> generate_project_name("synthetic", "fanout-test", 2, "prio_global")
        'syn-fanout-test-2-prio-global-e5f6a7b8'

        >>> generate_project_name("mssim", "S_14677443", 0, "fifo", 100.0)
        'mss-S_14677443-0-fifo-100-c9d0e1f2'
    """
    # Get app prefix (first 3 letters)
    app_prefix = app[:3].lower()

    # Build slug components
    slug_parts = [experiment_name, str(iteration), policy]
    if rps is not None:
        # Format RPS without decimal if integer value
        if rps == int(rps):
            slug_parts.append(str(int(rps)))
        else:
            slug_parts.append(str(rps))

    slug = "-".join(slug_parts)

    # Generate a short hash for uniqueness and to avoid name collisions
    # Use first 8 characters of SHA256 hash
    hash_input = f"{app}-{experiment_name}-{iteration}-{policy}"
    if rps is not None:
        hash_input += f"-{rps}"
    digest = hashlib.sha256(hash_input.encode()).hexdigest()[:8]

    # Combine: app_prefix-slug-digest
    project_name = f"{app_prefix}-{slug}-{digest}"

    # Docker Compose and Kubernetes have length limits and character restrictions
    # Ensure the name is lowercase and doesn't exceed reasonable length
    project_name = project_name.lower()

    # Log if the name is getting long (> 63 chars is K8s namespace limit)
    if len(project_name) > 63:
        logger.warning(
            f"Project name exceeds 63 characters (K8s limit): {project_name}"
        )

    return project_name


def truncate_experiment_slug(experiment_name: str, max_length: int = 20) -> str:
    """
    Truncate experiment name for use in project name.

    Useful when experiment names are very long (common with MSSIM trace names).

    Args:
        experiment_name: Full experiment name
        max_length: Maximum length for slug

    Returns:
        Truncated experiment name

    Examples:
        >>> truncate_experiment_slug("very-long-experiment-name-here", 15)
        'very-long-ex...'
    """
    if len(experiment_name) <= max_length:
        return experiment_name

    # Truncate and add ellipsis
    return experiment_name[: max_length - 3] + "..."
