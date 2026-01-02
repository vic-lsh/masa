from __future__ import annotations

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
) -> Deployment:
    """Create service discovery records for each service and persist them."""
    deployment = Deployment()

    for service_name in services:
        replicas = sim_cfg.replicas.count_for(service_name)
        print(f"Service: {service_name}, Port: {DEFAULT_SVC_PORT}")
        deployment.add_service(
            service_name,
            ServiceDiscoveryInfo(
                ip=f"{PROJECT_NAME}-{service_name}",
                port=DEFAULT_SVC_PORT,
                replicas=replicas,
            ),
        )

    deployment.export_to_file(deployment_output_path)
    print(f"Created deployment config at {deployment_output_path}")
    return deployment


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

    for service_name in config.call_graph.services():
        info = deployment.services.get(service_name)
        if info is None:
            raise RuntimeError(f"Service not found in deployment: {service_name}")
        services_section[service_name] = _make_service_def(
            service_name,
            info.port,
            sim_cfg,
            trace_dir,
            deployment_output_path,
        ).as_dict()

    services_section[LOADGEN_SERVICE_NAME] = _make_load_generator_config_yaml(
        trace_dir,
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
) -> ComposeService:
    image_name = os.environ.get(GENERIC_SERVICE_IMAGE_ENV, "generic_service")
    return ComposeService(
        image=image_name,
        scale=sim_cfg.replicas.count_for(service_name),
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
    environment = {
        "SERVICE_NAME": service_name,
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
    trace_dir: Path,
    deployment: Deployment,
) -> ComposeService:
    frontend_info = deployment.services.get(FRONTEND_SERVICE_NAME)
    if frontend_info is None:
        raise RuntimeError(
            f"Frontend service not found in deployment: {FRONTEND_SERVICE_NAME}"
        )

    environment = {
        "PORT": str(frontend_info.port),
        "IP": frontend_info.ip,
    }
    environment.update(_collect_optional_env("DURATION", "RPS", "SLO_MS", "WARMUP_SEC"))

    host_data_dir = os.environ.get("HOST_TRACE_DIR", str(trace_dir))
    volumes = [f"{host_data_dir}:{LOADGEN_OUTPUT_MOUNT}"]

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
