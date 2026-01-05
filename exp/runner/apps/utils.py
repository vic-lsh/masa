"""
Shared utilities for application plugins.
"""

import os
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


def get_docker_progress_flag() -> str:
    """
    Get the appropriate docker build progress flag based on environment.

    In CI environments (detected via CI environment variable), use 'plain'
    progress mode since TTY is not available. Otherwise, use 'tty' for
    better interactive output.

    Returns:
        Progress flag string: '--progress=plain' in CI, '--progress=tty' otherwise
    """
    # Check for CI environment variable (set by most CI systems including GitLab CI)
    if os.environ.get("CI", "").lower() in ("true", "1", "yes"):
        return "--progress=plain"
    return "--progress=tty"


def update_hotel_config_with_prefix(hotel_config: dict, prefix: str) -> dict:
    """
    Update hotel configuration to use prefixed container/service names.

    This modifies the service discovery names and database connection strings
    to use the PROJECT_PREFIX, allowing multiple hotel deployments to run in parallel.

    Args:
        hotel_config: The hotel configuration dictionary
        prefix: The PROJECT_PREFIX to prepend to container/service names

    Returns:
        Updated hotel configuration with prefixed names
    """
    import copy

    config = copy.deepcopy(hotel_config)

    # Update service IP addresses (DNS names) to use prefixed aliases
    # The docker-compose file creates network aliases like ${PROJECT_PREFIX}geo-service
    # We need to update from "local-geo-service" to "{prefix}geo-service"

    services_to_update = [
        "geo",
        "profile",
        "rate",
        "recommendation",
        "reservation",
        "review",
        "search",
        "user",
    ]

    for service in services_to_update:
        if service in config and "ip" in config[service]:
            # Replace "local-{service}-service" with "{prefix}{service}-service"
            old_name = f"local-{service}-service"
            new_name = f"{prefix}{service}-service"
            config[service]["ip"] = config[service]["ip"].replace(old_name, new_name)

    # Update MongoDB and Redis connection strings
    # These use container_name which includes the prefix
    if "profile" in config:
        if "mongodbAddr" in config["profile"]:
            config["profile"]["mongodbAddr"] = config["profile"]["mongodbAddr"].replace(
                "profile_mongo", f"{prefix}profile_mongo"
            )
        if "redisAddr" in config["profile"]:
            config["profile"]["redisAddr"] = config["profile"]["redisAddr"].replace(
                "profile_redis", f"{prefix}profile_redis"
            )
        # Also handle memcachedAddr if present
        if "memcachedAddr" in config["profile"]:
            config["profile"]["memcachedAddr"] = config["profile"][
                "memcachedAddr"
            ].replace("profile_memcached", f"{prefix}profile_memcached")

    if "rate" in config:
        if "mongodbAddr" in config["rate"]:
            config["rate"]["mongodbAddr"] = config["rate"]["mongodbAddr"].replace(
                "rate_mongo", f"{prefix}rate_mongo"
            )
        if "redisAddr" in config["rate"]:
            config["rate"]["redisAddr"] = config["rate"]["redisAddr"].replace(
                "rate_redis", f"{prefix}rate_redis"
            )
        if "memcachedAddr" in config["rate"]:
            config["rate"]["memcachedAddr"] = config["rate"]["memcachedAddr"].replace(
                "rate_memcached", f"{prefix}rate_memcached"
            )

    if "reservation" in config:
        if "mongodbAddr" in config["reservation"]:
            config["reservation"]["mongodbAddr"] = config["reservation"][
                "mongodbAddr"
            ].replace("reservation_mongo", f"{prefix}reservation_mongo")
        if "redisAddr" in config["reservation"]:
            config["reservation"]["redisAddr"] = config["reservation"][
                "redisAddr"
            ].replace("reservation_redis", f"{prefix}reservation_redis")
        if "memcachedAddr" in config["reservation"]:
            config["reservation"]["memcachedAddr"] = config["reservation"][
                "memcachedAddr"
            ].replace("reservation_memcached", f"{prefix}reservation_memcached")

    if "user" in config:
        if "mongodbAddr" in config["user"]:
            config["user"]["mongodbAddr"] = config["user"]["mongodbAddr"].replace(
                "user_mongo", f"{prefix}user_mongo"
            )

    if "review" in config:
        if "mongodbAddr" in config["review"]:
            config["review"]["mongodbAddr"] = config["review"]["mongodbAddr"].replace(
                "review_mongo", f"{prefix}review_mongo"
            )
        if "redisAddr" in config["review"]:
            config["review"]["redisAddr"] = config["review"]["redisAddr"].replace(
                "review_redis", f"{prefix}review_redis"
            )
        if "memcachedAddr" in config["review"]:
            config["review"]["memcachedAddr"] = config["review"][
                "memcachedAddr"
            ].replace("review_memcached", f"{prefix}review_memcached")

    return config
