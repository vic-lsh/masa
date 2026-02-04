"""Utilities for parsing and grouping Docker container names."""

import re


def extract_service_name(container_name: str) -> str:
    """
    Extract service name from container name, removing replica numbers and project prefixes.

    Container naming patterns:
    - Hotel: hotel-{slug}-{digest}-{service}-{replica}
             e.g., "hotel-exp1-abc123def456-rate-service-1" -> "rate-service"
    - Synthetic: {service}-{replica} or local-{service}-{replica}
                 e.g., "local-child-service-1" -> "child-service"
    - MSSIM: mssim-{slug}-{digest}-{service}-{replica}
             e.g., "mssim-exp1-abc123-frontend-1" -> "frontend"
    - Socialnet: {prefix}-{service}-{replica} or {prefix}_{service}
                 e.g., "socialnet-user-timeline-service-1" -> "user-timeline-service"

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

    # Handle mssim prefix: mssim-{slug}-{digest}-
    elif name.startswith("mssim-"):
        # Match pattern: mssim-{slug}-{hexdigest}-{service}-{replica}
        match = re.match(r"mssim-[a-z0-9-]+-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)

    # Handle socialnet prefix patterns
    elif name.startswith("socialnet-") or name.startswith("socialnet_"):
        # Remove socialnet prefix
        name = re.sub(r"^socialnet[-_]", "", name)

    # Handle synthetic local prefix
    elif name.startswith("local-"):
        name = name[6:]  # Remove "local-"

    # Handle synthetic prefix
    elif name.startswith("synthetic-"):
        # Match pattern: synthetic-{slug}-{hexdigest}-{service}-{replica}
        match = re.match(r"synthetic-[a-z0-9-]+-[a-f0-9]{12}-(.*)", name)
        if match:
            name = match.group(1)
        else:
            name = name[10:]  # Remove "synthetic-"

    elif name.startswith("synthetic_"):
        name = name[10:]  # Remove "synthetic_"

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
