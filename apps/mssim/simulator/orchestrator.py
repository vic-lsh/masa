from __future__ import annotations

import json
import os
import shutil
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
    all_services: Iterable[str],
    sim_cfg: SimulatorConfig,
    deployment_output_path: Path,
    callgraph_dirs: list[Path],
) -> Deployment:
    """Create service discovery records for each service and persist them.
    
    Unions services from all call graphs. For frontend services (USER), creates one per graph.
    For other services, creates one instance that handles all graphs.
    """
    deployment = Deployment()
    
    # Use the docker compose project name if provided, otherwise fall back to default
    project_name = os.environ.get("DOCKER_COMPOSE_PROJECT_NAME", PROJECT_NAME)
    
    # Track which services we've added (for non-frontend services, only add once)
    added_services: set[str] = set()
    
    # For each callgraph, create frontend services
    for callgraph_dir in callgraph_dirs:
        graph_name = callgraph_dir.name.replace("S_", "s-")
        frontend_service_name = f"{FRONTEND_SERVICE_NAME}-{graph_name}"
        
        # Use original service name for replica lookup
        replicas = sim_cfg.replicas.count_for(FRONTEND_SERVICE_NAME)
        print(f"Service: {frontend_service_name}, Port: {DEFAULT_SVC_PORT}")
        deployment.add_service(
            frontend_service_name,
            ServiceDiscoveryInfo(
                ip=f"{project_name}-{frontend_service_name}",
                port=DEFAULT_SVC_PORT,
                replicas=replicas,
            ),
        )
        added_services.add(frontend_service_name)
    
    # Add all other services (non-frontend) only once
    for service_name in all_services:
        if service_name == FRONTEND_SERVICE_NAME:
            continue  # Already handled above
        
        if service_name not in added_services:
            replicas = sim_cfg.replicas.count_for(service_name)
            print(f"Service: {service_name}, Port: {DEFAULT_SVC_PORT}")
            deployment.add_service(
                service_name,
                ServiceDiscoveryInfo(
                    ip=f"{project_name}-{service_name}",
                    port=DEFAULT_SVC_PORT,
                    replicas=replicas,
                ),
            )
            added_services.add(service_name)

    deployment.export_to_file(deployment_output_path)
    print(f"Created deployment config at {deployment_output_path}")
    return deployment


def generate_frontend_config(
    deployment: Deployment,
    frontend_output_path: Path,
    callgraph_dirs: list[Path],
) -> None:
    """Generate frontend.json configuration for the load generator."""
    # Use the docker compose project name if provided, otherwise fall back to default
    project_name = os.environ.get("DOCKER_COMPOSE_PROJECT_NAME", PROJECT_NAME)
    
    # Get SLO from environment or use default
    default_slo_ms = int(os.environ.get("SLO_MS", "400"))
    
    frontend_targets = []
    for callgraph_dir in callgraph_dirs:
        graph_name = callgraph_dir.name.replace("S_", "s-")
        frontend_service_name = f"{FRONTEND_SERVICE_NAME}-{graph_name}"
        
        info = deployment.services.get(frontend_service_name)
        if info is None:
            continue
        
        frontend_targets.append({
            "service": frontend_service_name,
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


def _merge_callgraph_directories(
    callgraph_dirs: list[Path],
    merged_config_dir: Path,
) -> None:
    """Merge multiple callgraph directories into a single config directory.
    
    Merges:
    - edges.csv: Not merged (services load from their graph-specific config)
    - interface_distribution.json: Merged with graph_id keys
    - latency_percentiles.json: Merged with graph_id keys  
    - call_sequence.json: Merged with graph_id keys (already has this structure)
    """
    merged_config_dir.mkdir(parents=True, exist_ok=True)
    
    # Merge call_sequence.json files
    merged_call_sequence: dict[str, Any] = {}
    for callgraph_dir in callgraph_dirs:
        graph_id = callgraph_dir.name  # Use directory name as graph_id (e.g., "S_14677443")
        call_sequence_path = callgraph_dir / "call_sequence.json"
        if call_sequence_path.exists():
            call_sequence_data = json.loads(call_sequence_path.read_text())
            # If the file already has graph_id structure, use it; otherwise wrap it
            if graph_id in call_sequence_data:
                merged_call_sequence[graph_id] = call_sequence_data[graph_id]
            elif len(call_sequence_data) == 1:
                # Single graph_id in file, use it
                merged_call_sequence[graph_id] = list(call_sequence_data.values())[0]
            else:
                # Multiple graph_ids, merge all
                merged_call_sequence.update(call_sequence_data)
    
    if merged_call_sequence:
        (merged_config_dir / "call_sequence.json").write_text(
            json.dumps(merged_call_sequence, indent=2)
        )
    
    # Merge interface_distribution.json files
    merged_interface_dist: dict[str, Any] = {}
    for callgraph_dir in callgraph_dirs:
        graph_id = callgraph_dir.name
        interface_dist_path = callgraph_dir / "interface_distribution.json"
        if interface_dist_path.exists():
            interface_dist_data = json.loads(interface_dist_path.read_text())
            merged_interface_dist[graph_id] = interface_dist_data
    
    if merged_interface_dist:
        (merged_config_dir / "interface_distribution.json").write_text(
            json.dumps(merged_interface_dist, indent=2)
        )
    
    # Merge latency_percentiles.json files
    merged_latency: dict[str, Any] = {}
    for callgraph_dir in callgraph_dirs:
        graph_id = callgraph_dir.name
        latency_path = callgraph_dir / "latency_percentiles.json"
        if latency_path.exists():
            latency_data = json.loads(latency_path.read_text())
            merged_latency[graph_id] = latency_data
    
    if merged_latency:
        (merged_config_dir / "latency_percentiles.json").write_text(
            json.dumps(merged_latency, indent=2)
        )
    
    # Copy edges.csv from first callgraph (services will load from graph-specific paths if needed)
    # Actually, we need edges.csv for each graph. Let's copy all of them with graph_id prefix
    for callgraph_dir in callgraph_dirs:
        graph_id = callgraph_dir.name
        edges_path = callgraph_dir / "edges.csv"
        if edges_path.exists():
            shutil.copy2(edges_path, merged_config_dir / f"edges_{graph_id}.csv")
    
    # Also copy the first one as edges.csv for backward compatibility
    if callgraph_dirs:
        first_edges = callgraph_dirs[0] / "edges.csv"
        if first_edges.exists():
            shutil.copy2(first_edges, merged_config_dir / "edges.csv")


def generate_docker_compose(
    output_path: Path,
    trace_configs: list[TraceConfig],
    callgraph_dirs: list[Path],
    sim_cfg: SimulatorConfig,
    deployment: Deployment,
    deployment_output_path: Path,
) -> None:
    """Write a docker-compose document covering all services and the load generator."""
    services_section: dict[str, dict[str, Any]] = {}
    
    # Get all services from all configs (union)
    all_services: set[str] = set()
    for config in trace_configs:
        all_services.update(config.call_graph.services())

    # Add services to docker-compose
    for service_name in all_services:
        if service_name == FRONTEND_SERVICE_NAME:
            # Frontend services are handled per-graph
            continue
        
        info = deployment.services.get(service_name)
        if info is None:
            continue
        
        services_section[service_name] = _make_service_def(
            service_name,
            info.port,
            sim_cfg,
            callgraph_dirs,
            deployment_output_path,
            service_name,  # Pass service name for replica lookup
        ).as_dict()
    
    # Add frontend services (one per graph)
    for callgraph_dir in callgraph_dirs:
        graph_name = callgraph_dir.name.replace("S_", "s-")
        frontend_service_name = f"{FRONTEND_SERVICE_NAME}-{graph_name}"
        info = deployment.services.get(frontend_service_name)
        if info:
            services_section[frontend_service_name] = _make_service_def(
                frontend_service_name,
                info.port,
                sim_cfg,
                callgraph_dirs,
                deployment_output_path,
                FRONTEND_SERVICE_NAME,  # Pass original for replica lookup
            ).as_dict()

    # Generate frontend.json in the same directory as deployment.json
    frontend_output_path = deployment_output_path.parent / "frontend.json"
    generate_frontend_config(deployment, frontend_output_path, callgraph_dirs)

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
    trace_configs: list[TraceConfig],
    callgraph_dirs: list[Path],
    sim_config: SimulatorConfig,
    docker_compose_output_path: Path,
    deployment_output_path: Path,
) -> None:
    """Generate deployment metadata and the docker-compose file for a trace replay."""
    # Union all services from all call graphs
    all_services = TraceConfig.union_services(trace_configs)
    
    # No longer merge - we'll mount each call graph directory separately
    # The generic service will read from multiple directories
    
    deployment = generate_service_configs(
        all_services,
        sim_config,
        deployment_output_path,
        callgraph_dirs,
    )

    print("Generated deployment:")
    for name, info in deployment.services.items():
        print(f"  Service: {name}, Info: {info}")

    generate_docker_compose(
        docker_compose_output_path,
        trace_configs,
        callgraph_dirs,
        sim_config,
        deployment,
        deployment_output_path,
    )


def _make_service_def(
    service_name: str,
    svc_port: int,
    sim_cfg: SimulatorConfig,
    callgraph_dirs: list[Path],
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
        environment=_make_environment_def(service_name, svc_port, callgraph_dirs),
        volumes=_make_volumes_def(callgraph_dirs, deployment_output_path),
        networks=[NETWORK_NAME],
    )


def _make_environment_def(service_name: str, svc_port: int, callgraph_dirs: list[Path]) -> dict[str, str]:
    # For user-* services, set SERVICE_NAME to "USER" instead of the deployment service name
    if service_name == FRONTEND_SERVICE_NAME or service_name.startswith(FRONTEND_SERVICE_NAME + "-"):
        env_service_name = "USER"
    else:
        env_service_name = service_name
    
    # No need to pass CALLGRAPH_DIRS - the generic service will enumerate /app/callgraphs
    environment = {
        "SERVICE_NAME": env_service_name,
        "SERVICE_PORT": str(svc_port),
        "DEPLOYMENT_CONFIG_PATH": "/app/config/deployment.json",
    }

    environment.update(_collect_optional_env("FEATURE"))
    return environment


def _make_volumes_def(callgraph_dirs: list[Path], deployment_output_path: Path) -> list[str]:
    volumes = []
    
    # Mount each call graph directory under /app/callgraphs/{graph_name}
    for callgraph_dir in callgraph_dirs:
        graph_name = callgraph_dir.name
        container_path = f"/app/callgraphs/{graph_name}"
        volumes.append(f"{callgraph_dir}:{container_path}:ro")
    
    # Mount deployment config
    deployment_mapping = f"{deployment_output_path}:/app/config/deployment.json:ro"
    volumes.append(deployment_mapping)
    
    return volumes


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
        environment=environment,
        volumes=volumes,
        networks=[NETWORK_NAME],
    )


def _collect_optional_env(*names: str) -> dict[str, str]:
    """Return environment variables that are set from the current process."""
    return {name: value for name in names if (value := os.environ.get(name))}
