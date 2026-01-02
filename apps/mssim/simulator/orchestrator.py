from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from .deployment import Deployment, ServiceDiscoveryInfo
from .simulator_config import SimulatorConfig
from .trace_config import TraceConfig
from .utils import normalize_service_name
from .yaml_utils import dump_yaml

LOADGEN_SERVICE_NAME = "load_generator"
FRONTEND_SERVICE_NAME = normalize_service_name("USER")
LOADGEN_OUTPUT_MOUNT = "/app/loadgen_output"
CONTAINER_CPU_LIMIT = 1
CONTAINER_MEM_LIMIT = "10GB"
DEFAULT_SVC_PORT = 50051
PROJECT_NAME = "mssim"
NETWORK_NAME = "microservice_net"
GENERIC_SERVICE_IMAGE_ENV = "GENERIC_SERVICE_IMAGE"


@dataclass(slots=True)
class ComposeService:
    image: str
    environment: dict[str, str]
    networks: list[str]
    volumes: list[str] = field(default_factory=list)
    deploy: dict[str, Any] | None = None
    scale: int | None = None
    container_name: str | None = None

    def as_dict(self) -> dict[str, Any]:
        service: dict[str, Any] = {
            "image": self.image,
            "environment": self.environment,
            "networks": self.networks,
        }
        if self.volumes:
            service["volumes"] = self.volumes
        if self.deploy:
            service["deploy"] = self.deploy
        if self.scale is not None:
            service["scale"] = self.scale
        if self.container_name:
            service["container_name"] = self.container_name
        return service


def generate_service_configs(
    services: Iterable[str],
    sim_cfg: SimulatorConfig,
    deployment_output_path: Path,
    trace_dir: Path,
) -> Deployment:
    """Create service discovery records for each service and persist them."""
    deployment = Deployment()
    
    # Use the docker compose project name if provided, otherwise fall back to default
    project_name = os.environ.get("DOCKER_COMPOSE_PROJECT_NAME", PROJECT_NAME)
    
    # Extract graph name from trace directory (e.g., "trace-analysis/graphs/S_14677443" -> "s-14677443")
    # Normalize for Docker compatibility: replace "S_" with "s-" for use in service names and IPs
    graph_name = trace_dir.name.replace("S_", "s-")

    for original_service_name in services:
        # Replace "user" with "user-{graph_name}" for frontend services
        if original_service_name == FRONTEND_SERVICE_NAME:
            service_name = f"{FRONTEND_SERVICE_NAME}-{graph_name}"
        else:
            service_name = original_service_name
        
        # Use original service name for replica lookup (config may be keyed by "user")
        replicas = sim_cfg.replicas.count_for(original_service_name)
        print(f"Service: {service_name}, Port: {DEFAULT_SVC_PORT}")
        deployment.add_service(
            service_name,
            ServiceDiscoveryInfo(
                ip=f"{project_name}-{service_name}",
                port=DEFAULT_SVC_PORT,
                replicas=replicas,
            ),
        )

    deployment.export_to_file(deployment_output_path)
    print(f"Created deployment config at {deployment_output_path}")
    return deployment


def generate_frontend_config(
    deployment: Deployment,
    frontend_output_path: Path,
) -> None:
    """Generate frontend.json configuration for the load generator."""
    # Use the docker compose project name if provided, otherwise fall back to default
    project_name = os.environ.get("DOCKER_COMPOSE_PROJECT_NAME", PROJECT_NAME)
    
    # Get SLO from environment or use default
    default_slo_ms = int(os.environ.get("SLO_MS", "400"))
    
    frontend_targets = []
    for name, info in deployment.services.items():
        # Check if service name is "user" or starts with "user-" (frontend service)
        if name == FRONTEND_SERVICE_NAME or name.startswith(FRONTEND_SERVICE_NAME + "-"):
            # Extract graph name from service name
            # If name is "user", use "user" as graph name
            # If name is "user-{graph_name}", extract the graph name part
            if name == FRONTEND_SERVICE_NAME:
                graph_name = FRONTEND_SERVICE_NAME
            else:
                graph_name = name[len(FRONTEND_SERVICE_NAME) + 1:]
            
            frontend_targets.append({
                "service": name,
                "ip": info.ip,
                "port": info.port,
                "replicas": info.replicas,
                "graph": graph_name,
                "slo_ms": default_slo_ms,
                "probability": 1.0,  # Will be normalized below
            })
    
    if not frontend_targets:
        raise RuntimeError(
            f"Frontend service not found in deployment: expected names starting with {FRONTEND_SERVICE_NAME}"
        )
    
    # Normalize probabilities to sum to 1.0
    if len(frontend_targets) > 1:
        prob_per_target = 1.0 / len(frontend_targets)
        for target in frontend_targets:
            target["probability"] = prob_per_target
    
    frontend_output_path.parent.mkdir(parents=True, exist_ok=True)
    frontend_output_path.write_text(json.dumps(frontend_targets, indent=2))
    print(f"Created frontend config at {frontend_output_path}")


def generate_docker_compose(
    output_path: Path,
    config: TraceConfig,
    trace_dir: Path,
    sim_cfg: SimulatorConfig,
    deployment: Deployment,
    deployment_output_path: Path,
) -> None:
    """Write a docker-compose document covering all services and the load generator."""
    services_section: dict[str, dict[str, Any]] = {}
    
    # Extract graph name from trace directory (same logic as in generate_service_configs)
    graph_name = trace_dir.name.replace("S_", "s-")

    for original_service_name in config.call_graph.services():
        # Map original service name to deployment service name (user -> user-{graph_name})
        if original_service_name == FRONTEND_SERVICE_NAME:
            deployment_service_name = f"{FRONTEND_SERVICE_NAME}-{graph_name}"
        else:
            deployment_service_name = original_service_name
        
        info = deployment.services.get(deployment_service_name)
        if info is None:
            raise RuntimeError(f"Service not found in deployment: {deployment_service_name} (original: {original_service_name})")
        # Use deployment_service_name as the docker compose service name to match deployment
        services_section[deployment_service_name] = _make_service_def(
            deployment_service_name,
            info.port,
            sim_cfg,
            trace_dir,
            deployment_output_path,
            original_service_name,  # Pass original for replica lookup
        ).as_dict()

    # Generate frontend.json in the same directory as deployment.json
    frontend_output_path = deployment_output_path.parent / "frontend.json"
    generate_frontend_config(deployment, frontend_output_path)

    services_section[LOADGEN_SERVICE_NAME] = _make_load_generator_config_yaml(
        deployment_output_path.parent,
        deployment,
    ).as_dict()

    compose_doc = {
        "services": services_section,
        "networks": {
            NETWORK_NAME: {"driver": "bridge"},
        },
    }

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(dump_yaml(compose_doc))
    print(f"docker-compose.yml written to {output_path}")


def launch_simulation_from_trace(
    config: TraceConfig,
    trace_dir: Path,
    sim_config: SimulatorConfig,
    docker_compose_output_path: Path,
    deployment_output_path: Path,
) -> None:
    """Generate deployment metadata and the docker-compose file for a trace replay."""
    deployment = generate_service_configs(
        config.call_graph.services(),
        sim_config,
        deployment_output_path,
        trace_dir,
    )

    print("Generated deployment:")
    for name, info in deployment.services.items():
        print(f"  Service: {name}, Info: {info}")

    generate_docker_compose(
        docker_compose_output_path,
        config,
        trace_dir,
        sim_config,
        deployment,
        deployment_output_path,
    )


def _make_service_def(
    service_name: str,
    svc_port: int,
    sim_cfg: SimulatorConfig,
    trace_dir: Path,
    deployment_output_path: Path,
    original_service_name: str | None = None,
) -> ComposeService:
    image_name = os.environ.get(GENERIC_SERVICE_IMAGE_ENV, "generic_service")
    # Use original service name for replica lookup if provided (for renamed services like user -> user-{graph})
    replica_lookup_name = original_service_name if original_service_name is not None else service_name
    return ComposeService(
        image=image_name,
        scale=sim_cfg.replicas.count_for(replica_lookup_name),
        deploy={
            "resources": {
                "limits": {
                    "cpus": str(CONTAINER_CPU_LIMIT),
                    "memory": str(CONTAINER_MEM_LIMIT),
                }
            }
        },
        environment=_make_environment_def(service_name, svc_port),
        volumes=_make_volumes_def(trace_dir, deployment_output_path),
        networks=[NETWORK_NAME],
    )


def _make_environment_def(service_name: str, svc_port: int) -> dict[str, str]:
    # For user-* services, set SERVICE_NAME to "USER" instead of the deployment service name
    if service_name == FRONTEND_SERVICE_NAME or service_name.startswith(FRONTEND_SERVICE_NAME + "-"):
        env_service_name = "USER"
    else:
        env_service_name = service_name
    
    environment = {
        "SERVICE_NAME": env_service_name,
        "SERVICE_PORT": str(svc_port),
        "CONFIG_PATH": "/app/config",
        "DEPLOYMENT_CONFIG_PATH": "/app/config/deployment.json",
    }

    environment.update(_collect_optional_env("FEATURE"))
    return environment


def _make_volumes_def(trace_dir: Path, deployment_output_path: Path) -> list[str]:
    config_dir_mapping = f"{trace_dir}:/app/config"
    deployment_mapping = f"{deployment_output_path}:/app/config/deployment.json"
    return [config_dir_mapping, deployment_mapping]


def _make_load_generator_config_yaml(
    frontend_config_dir: Path,
    deployment: Deployment,
) -> ComposeService:
    environment: dict[str, str] = {}

    environment.update(_collect_optional_env("DURATION", "RPS", "RPS_VALUES", "MAX_IN_FLIGHT", "STATS_INTERVAL_SEC"))

    host_data_dir = os.environ.get("HOST_TRACE_DIR", None)
    frontend_json_path = frontend_config_dir / "frontend.json"
    host_frontend_target = f"{frontend_json_path}:/app/frontend.json:ro"
    volumes = [f"{host_data_dir}:{LOADGEN_OUTPUT_MOUNT}", host_frontend_target]

    return ComposeService(
        image="mssim_load_generator",
        container_name=LOADGEN_SERVICE_NAME,
        environment=environment,
        volumes=volumes,
        networks=[NETWORK_NAME],
    )


def _collect_optional_env(*names: str) -> dict[str, str]:
    """Return environment variables that are set from the current process."""
    return {name: value for name in names if (value := os.environ.get(name))}
