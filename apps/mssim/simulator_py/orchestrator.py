from __future__ import annotations

import os
from pathlib import Path
from typing import Iterable

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


def generate_service_configs(
    services: Iterable[str],
    sim_cfg: SimulatorConfig,
    deployment_output_path: Path,
) -> Deployment:
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
    services_section = {}

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
        )

    services_section[LOADGEN_SERVICE_NAME] = _make_load_generator_config_yaml(
        trace_dir,
        deployment,
    )

    compose_doc = {
        "services": services_section,
        "networks": {
            "microservice_net": {"driver": "bridge"},
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
) -> dict:
    return {
        "image": "generic_service",
        "scale": sim_cfg.replicas.count_for(service_name),
        "deploy": {
            "resources": {
                "limits": {
                    "cpus": str(CONTAINER_CPU_LIMIT),
                    "memory": str(CONTAINER_MEM_LIMIT),
                }
            }
        },
        "environment": _make_environment_def(service_name, svc_port),
        "volumes": _make_volumes_def(trace_dir, deployment_output_path),
        "networks": ["microservice_net"],
    }


def _make_environment_def(service_name: str, svc_port: int) -> dict:
    environment = {
        "SERVICE_NAME": service_name,
        "SERVICE_PORT": str(svc_port),
        "CONFIG_PATH": "/app/config",
        "DEPLOYMEN_CONFIG_PATH": "/app/config/deployment.json",
    }

    feature = os.environ.get("FEATURE")
    if feature:
        environment["FEATURE"] = feature

    return environment


def _make_volumes_def(trace_dir: Path, deployment_output_path: Path) -> list:
    config_dir_mapping = f"{trace_dir}:{'/app/config'}"
    deployment_mapping = f"{deployment_output_path}:/app/config/deployment.json"
    return [config_dir_mapping, deployment_mapping]


def _make_load_generator_config_yaml(
    trace_dir: Path,
    deployment: Deployment,
) -> dict:
    frontend_info = deployment.services.get(FRONTEND_SERVICE_NAME)
    if frontend_info is None:
        raise RuntimeError(
            f"Frontend service not found in deployment: {FRONTEND_SERVICE_NAME}"
        )

    environment = {
        "PORT": str(frontend_info.port),
        "IP": frontend_info.ip,
    }

    for var in ("DURATION", "RPS", "SLO_MS"):
        value = os.environ.get(var)
        if value:
            environment[var] = value

    host_data_dir = os.environ.get("HOST_TRACE_DIR", str(trace_dir))
    volumes = [f"{host_data_dir}:{LOADGEN_OUTPUT_MOUNT}"]

    return {
        "image": "mssim_load_generator",
        "container_name": LOADGEN_SERVICE_NAME,
        "environment": environment,
        "volumes": volumes,
        "networks": ["microservice_net"],
    }
