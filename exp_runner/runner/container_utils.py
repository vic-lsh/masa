"""Utilities for parsing and grouping Docker container names."""

import re


def extract_service_name(container_name: str) -> str:
    """
    Extract service name from container name, removing replica numbers and project prefixes.

    Container naming patterns:
    - Hotel: hotel-{slug}-{digest}-{service}-{replica}
             e.g., "hotel-exp1-abc123def456-rate-service-1" -> "rate-service"
    - Synthbench: {service}-{replica} or local-{service}-{replica}
                 e.g., "local-child-service-1" -> "child-service"
    - Tracebench: tracebench-{slug}-{digest}-{service}-{replica}
             e.g., "tracebench-exp1-abc123-frontend-1" -> "frontend"
    - Socialnet: {prefix}-{service}-{replica} or {prefix}_{service}
                 e.g., "socialnet-ci2-9d93f9cef14a-compose-post-service-1" -> "compose-post-service"
    - Generic: {prefix}-{slug}-{digest}-{service}-{replica} (digest is 12 hex chars)
                 e.g., "syn-exp1-abc123def456-child-service-1" -> "child-service"

    Args:
        container_name: Full container name

    Returns:
        Service name without replica number and project prefix
    """
    # Remove common prefixes
    name = container_name

    # Handle hotel prefix: hotel-{slug}-{digest}-
    if name.startswith("hotel-"):
        # Match pattern: hotel-{slug}-{hexdigest}-{service}-{replica}
        match = re.match(r"hotel-[a-z0-9-]+-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)

    # Handle tracebench prefix: tracebench-{slug}-{digest}-
    elif name.startswith("tracebench-"):
        # Match pattern: tracebench-{slug}-{hexdigest}-{service}-{replica}
        match = re.match(r"tracebench-[a-z0-9-]+-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)

    # Handle socialnet prefix patterns. The plugin uses prefix "sn" so the
    # generated project_name fits inside the 63-char k8s DNS label limit
    # (see SocialnetApp.prepare_workload). Older artifacts may still use the
    # historical "socialnet-" prefix, so strip both.
    elif (
        name.startswith("sn-")
        or name.startswith("socialnet-")
        or name.startswith("socialnet_")
    ):
        # Match pattern: {sn|socialnet}-{slug}-{hexdigest}-{service}-{replica}
        match = re.match(r"(?:sn|socialnet)-[a-z0-9-]+-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)
        else:
            # Fallback: Remove the bare prefix
            name = re.sub(r"^(?:sn|socialnet)[-_]", "", name)

    # Handle synthbench local prefix
    elif name.startswith("local-"):
        name = name[6:]  # Remove "local-"

    # Handle synthbench prefix
    elif name.startswith("synthbench-"):
        # Match pattern: synthbench-{slug}-{hexdigest}-{service}-{replica}
        match = re.match(r"synthbench-[a-z0-9-]+-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)
        else:
            name = name[len("synthbench-") :]

    elif name.startswith("synthbench_"):
        name = name[len("synthbench_") :]

    # Generic fallback: Match pattern with 12-char hex digest
    # e.g., {prefix}-{slug}-{digest}-{service}-{replica}
    # This handles any app using the _safe_project_name convention (e.g., syn-, tracebench-, etc.)
    # if it wasn't caught by specific prefixes above.
    else:
        # Look for -{digest}- where digest is exactly 12 hex chars
        # We use search instead of match to find it anywhere in the string
        match = re.search(r"-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)

    # Remove trailing replica number (e.g., "-1", "-2")
    # Match pattern: {service}-{number} at the end
    name = re.sub(r"-\d+$", "", name)

    # Handle special cases for load generators and network components
    # Keep the full name if it's a loadgen or network component
    if "loadgen" in name.lower() or "client" in name.lower() or "bench" in name.lower():
        # For load generators, keep the entire name after prefix removal
        return name

    if "network" in name.lower():
        return name

    return name


def parse_container_name(container_name: str) -> dict:
    """
    Parse container name into components.

    Args:
        container_name: Full container name

    Returns:
        Dict with keys:
            - full_name: Original container name
            - service_name: Service name (grouped across replicas)
            - replica_num: Replica number if present, otherwise None
            - is_loadgen: True if this is a load generator container
    """
    service_name = extract_service_name(container_name)

    # Extract replica number from original name
    replica_match = re.search(r"-(\d+)$", container_name)
    replica_num = int(replica_match.group(1)) if replica_match else None

    # Check if this is a load generator
    name_lower = container_name.lower()
    is_loadgen = (
        "loadgen" in name_lower
        or "client" in name_lower
        and "bench" in name_lower
        or "load_generator" in name_lower
    )

    return {
        "full_name": container_name,
        "service_name": service_name,
        "replica_num": replica_num,
        "is_loadgen": is_loadgen,
    }


def group_containers_by_service(container_names: list[str]) -> dict[str, list[str]]:
    """
    Group container names by service.

    Args:
        container_names: List of container names

    Returns:
        Dict mapping service_name -> list of container names belonging to that service
    """
    service_groups: dict[str, list[str]] = {}

    for container_name in container_names:
        parsed = parse_container_name(container_name)
        service_name = parsed["service_name"]

        if service_name not in service_groups:
            service_groups[service_name] = []

        service_groups[service_name].append(container_name)

    return service_groups
