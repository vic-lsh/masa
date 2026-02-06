"""
Synthetic application plugin.
"""

import hashlib
import json
import logging
import re
import shlex
import subprocess
import time
from pathlib import Path
from typing import TYPE_CHECKING, Any, Optional, Tuple

import yaml

from ..build_orchestrator import BuildOrchestrator
from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor, MockCommandExecutor, SubprocessExecutor
from ..generators import ComposeGenerator
from ..generators.base import GeneratedDeployment
from ..legacy import convert_legacy_to_experiment_config
from ..naming import generate_project_name
from ..topology import TopologyResolver, TopologySpec
from .base import AppBuilder, AppPlugin, DockerConfig
from .utils import get_docker_progress_flag, normalize_features_to_tag

if TYPE_CHECKING:  # pragma: no cover
    from ..config import ExperimentConfig
    from ..deployment_manager import DeploymentManager
    from ..experiment_config_v2 import ExperimentConfigV2

logger = logging.getLogger(__name__)


def _safe_project_name(*, experiment_name: str, iteration: int, policy: str) -> str:
    """
    Generate a docker-compose project name that is safe and deterministic.

    Policy strings may contain characters like commas (e.g., "fifo,early") that
    Docker Compose will normalize. We avoid any mismatch by using a digest-based
    project name that contains only safe characters.
    """
    raw = f"{experiment_name}|{iteration}|{policy}"
    digest = hashlib.sha256(raw.encode("utf-8")).hexdigest()[:12]
    slug = re.sub(r"[^a-z0-9]+", "-", experiment_name.lower()).strip("-")[:12] or "exp"
    return f"syn-{slug}-{digest}"


class SyntheticApp(AppPlugin):
    """
    Plugin for the synthetic benchmark application.

    The synthetic app has a simpler architecture with child services that can
    be configured with different replication strategies.

    When call_graph is configured, it dynamically generates a docker compose file
    with separate services for each service in the call graph.
    """

    def __init__(self):
        """Initialize synthetic app plugin."""
        self._generated_compose_path: Optional[Path] = None
        self._generated_deployment: Optional[GeneratedDeployment] = None
        self._experiment_config_v2 = None

    def get_app_name(self) -> str:
        return "synthetic"

    def _is_new_generator_enabled(self) -> bool:
        return getattr(self, "_using_new_generator", False)

    # ===== NEW SIMPLIFIED INTERFACE (Phase 4) =====

    def get_binaries(self) -> list[str]:
        """Return list of binary names for synthetic app."""
        return [
            "synthetic_frontend",
            "synthetic_child",
            "synthetic_client_bench",
        ]

    def get_frontend_name(self) -> str:
        """Return the frontend service name."""
        return "synthetic-frontend-service"

    def get_cargo_package(self) -> str:
        """Return cargo package name."""
        return "synthetic"

    def get_build_parallelism(self) -> int:
        """Synthetic has 3 binaries - use parallelism of 3."""
        return 3

    def get_default_topology_path(self, repo_root: Path) -> Optional[Path]:
        """
        Synthetic uses explicit topologies in exp/synthetic/topologies/.
        Return None to indicate topology must be specified per experiment.
        """
        return None

    def run_workload(
        self,
        *,
        repo_root: Path,
        config: "ExperimentConfig",
        deployment: "DeploymentManager",
        policy: str,
        iteration: int,
        output_dir: Path,
        app_local_dir: Path,
        no_cache: bool,
        dry_run: bool = False,
        executor: Optional[CommandExecutor] = None,
        use_new_generator: bool = False,
        **kwargs,
    ) -> None:
        """
        Override run_workload to reset generator state after each iteration.
        """
        try:
            super().run_workload(
                repo_root=repo_root,
                config=config,
                deployment=deployment,
                policy=policy,
                iteration=iteration,
                output_dir=output_dir,
                app_local_dir=app_local_dir,
                no_cache=no_cache,
                dry_run=dry_run,
                executor=executor,
                use_new_generator=use_new_generator,
                **kwargs,
            )
        finally:
            if use_new_generator:
                self._generated_deployment = None

    def customize_topology(self, topology):
        """
        Hook for synthetic-specific topology transformations.
        Currently no transformations needed.
        """
        return topology

    def customize_env_vars(self, topology, experiment, base_env):
        """
        Add synthetic-specific environment variables.

        Args:
            topology: Topology specification
            experiment: Experiment configuration
            base_env: Base environment variables from generator

        Returns:
            Updated environment variables
        """
        # Add any synthetic-specific env vars if needed
        if "LOG_LEVEL" not in base_env:
            base_env["LOG_LEVEL"] = "info"

        # Add CPUS_PER_REPLICA if not set (used for resource limits)
        if "CPUS_PER_REPLICA" not in base_env:
            base_env["CPUS_PER_REPLICA"] = "1"

        return base_env

    # ===== LEGACY INTERFACE (backward compatibility) =====

    @property
    def supports_k8s(self) -> bool:
        return True

    def create_builder(self) -> AppBuilder:
        if self._is_new_generator_enabled():
            app = self

            class _Adapter(AppBuilder):
                def build(
                    self,
                    *,
                    repo_root: Path,
                    app_dir: Path,
                    features: Optional[str] = None,
                    rust_log: str = "info",
                    no_cache: bool = False,
                    gen_config_path: Optional[Path] = None,
                    dry_run: bool = False,
                    executor: Optional[CommandExecutor] = None,
                ) -> Optional[list[list[str]]]:
                    if gen_config_path is None:
                        raise ValueError(
                            "gen_config_path is required when using BuildOrchestrator"
                        )

                    orchestrator = BuildOrchestrator(repo_root, executor=executor)
                    orchestrator.build(
                        app=app,
                        features=features,
                        gen_config_path=gen_config_path,
                        rust_log=rust_log,
                        no_cache=no_cache,
                        dry_run=dry_run,
                    )
                    return None

            return _Adapter()

        return SyntheticBuilder()

    def get_image_tag(self, features: Optional[str] = None) -> str:
        return normalize_features_to_tag(features)

    def load_app_config(self, config_path: Path) -> dict:
        """Load config.docker.json configuration file."""
        with open(config_path) as f:
            return json.load(f)

    def create_gen_config_dict(
        self,
        template_config_path: Path,
        project_name: str,
        service_name_override: Optional[str] = None,
    ) -> dict:
        """
        Generate project-specific gen_config.json content with correct frontend service name.

        Updates the "Addr" field to point to the Docker Compose service name
        (synthetic-frontend-service). Docker Compose DNS resolution uses service names,
        not container names. The container name will be automatically prefixed with
        the project name by Docker Compose.
        """
        with open(template_config_path) as f:
            config = json.load(f)

        # Parse the original address
        if "Addr" in config:
            addr = config["Addr"]
            # Extract protocol and port from original address
            if "://" in addr:
                protocol, rest = addr.split("://", 1)
                if ":" in rest:
                    _, port = rest.rsplit(":", 1)
                else:
                    port = "8000"  # default
            else:
                protocol = "http"
                port = "8000"

            # Use the service name from Docker Compose (synthetic-frontend-service)
            # Docker Compose DNS resolution uses service names, not container names
            # The container will be named {project_name}-synthetic-frontend-service-1
            # but DNS resolution uses the service name

            service_name = service_name_override or "synthetic-frontend-service"
            config["Addr"] = f"{protocol}://{service_name}:{port}"

        return config

    def create_call_graph_compose_dict(
        self,
        app_config: dict,
        image_tag: str,
    ) -> dict:
        """
        Generate a docker compose dictionary for call graph configuration.

        Creates a compose file content with:
        - Frontend service
        - One service per call graph service, each with its own SERVICE_ID

        Args:
            app_config: Application configuration dict
            image_tag: Docker image tag

        Returns:
            Dictionary representing docker-compose file content
        """
        call_graph = app_config.get("call_graph")
        if not call_graph:
            raise ValueError("call_graph not found in app_config")

        services = {}

        # Add frontend service
        # Build depends_on list for all call graph services
        # Use "local-{service-id}-service" naming to match service names
        depends_on = [
            f"local-{svc['id'].lower().replace('_', '-')}-service"
            for svc in call_graph["services"]
        ]
        services["synthetic-frontend-service"] = {
            "image": f"synthetic_frontend:{image_tag}",
            "restart": "always",
            "networks": ["synthetic_network"],
            # "ports": ["${FRONTEND_PORT}:8000"],
            "depends_on": depends_on,
            "environment": [
                "BINARY_NAME=synthetic_frontend",
                "LOG_LEVEL=${LOG_LEVEL:-info}",
                "DOCKER_COMPOSE_PROJECT_NAME=${DOCKER_COMPOSE_PROJECT_NAME:-}",
            ],
            "volumes": ["${APP_CONFIG_PATH}:/usr/config.json:ro"],
            "deploy": {"resources": {"limits": {"cpus": "4"}}},
        }

        # Add one service per call graph service
        # Use "local-{service-id}-service" naming to match frontend expectations
        for service_def in call_graph["services"]:
            service_id = service_def["id"]
            service_name = f"local-{service_id.lower().replace('_', '-')}-service"
            replicas = service_def.get("replicas", 1)

            services[service_name] = {
                "image": f"synthetic_child:{image_tag}",
                "scale": replicas,
                "networks": ["synthetic_network"],
                "environment": [
                    "BINARY_NAME=synthetic_child",
                    "LOG_LEVEL=${LOG_LEVEL:-info}",
                    f"SERVICE_ID={service_id}",
                    "DOCKER_COMPOSE_PROJECT_NAME=${DOCKER_COMPOSE_PROJECT_NAME:-}",
                ],
                "volumes": ["${APP_CONFIG_PATH}:/usr/config.json:ro"],
                "deploy": {"resources": {"limits": {"cpus": "${CPUS_PER_REPLICA}"}}},
            }

        compose_content = {
            "services": services,
            "networks": {"synthetic_network": {"driver": "bridge"}},
        }
        return compose_content

    def generate_env_vars(
        self, gen_config: dict, app_config: Optional[dict], app_dir: Path
    ) -> dict:
        """
        Generate environment variables for synthetic application.

        Calculates child replica counts from config and extracts frontend port.
        When call_graph is present, handles call graph services instead.
        """
        env_vars = {}

        # Extract frontend port from address (e.g., "http://[::1]:8658" -> "8658")
        addr = gen_config.get("Addr", "")
        match = re.search(r":(\d+)$", addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")

        if app_config:
            # Check if call_graph is configured
            call_graph = app_config.get("call_graph")
            if call_graph:
                # Call graph mode: calculate total replicas from call graph services
                total_replicas = sum(
                    service.get("replicas", 1)
                    for service in call_graph.get("services", [])
                )
                env_vars["CHILD_REPLICAS"] = str(total_replicas)

                # CPUs per replica (use default if not specified)
                cpus_per_replica = app_config.get("child_cpus_per_replica", 1)
                env_vars["CPUS_PER_REPLICA"] = str(cpus_per_replica)
            else:
                # Traditional mode: calculate child replicas from child_services
                child_services = app_config.get("child_services", [])
                if child_services:
                    child_replicas = sum(
                        service.get("replicas", 1) for service in child_services
                    )
                else:
                    child_replicas = 1

                env_vars["CHILD_REPLICAS"] = str(child_replicas)

                # CPUs per replica
                cpus_per_replica = app_config.get("child_cpus_per_replica", 1)
                env_vars["CPUS_PER_REPLICA"] = str(cpus_per_replica)
        else:
            # Use defaults if no config provided
            env_vars["CHILD_REPLICAS"] = "1"
            env_vars["CPUS_PER_REPLICA"] = "1"

        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "info"

        return env_vars

    def get_docker_config(self) -> DockerConfig:
        """
        Return Docker configuration for synthetic application.

        Note: When call_graph is configured, this is called first with default values.
        The actual compose file generation happens in run_workload override.
        """
        return DockerConfig(
            compose_file="docker-compose.yaml",
            network_name="local_synthetic_network",
            loadgen_image_name="synthetic_client_bench:<features>",
            loadgen_binary_name="synthetic_client_bench",
            app_config_filename="config.docker.json",
        )

    def get_required_images(self, features: Optional[str] = None) -> list[str]:
        tag = self.get_image_tag(features)
        suffix = f":{tag}" if tag else ":latest"
        return [
            f"synthetic_frontend{suffix}",
            f"synthetic_child{suffix}",
            f"synthetic_client_bench{suffix}",
        ]

    def get_container_names(
        self, env_vars: dict, app_config: Optional[dict] = None
    ) -> list[str]:
        """
        Get list of container names for synthetic application.

        Includes frontend and child service containers based on replica count.
        When call_graph is configured, includes containers for each call graph service.

        Note: This method returns container name patterns without project prefix.
        The actual container names will include the project prefix when Docker Compose
        creates them. For actual container names, use docker.get_container_names() which
        queries Docker Compose directly.
        """
        # Frontend container name pattern (without project prefix)
        # Docker Compose will create: {project}-synthetic-frontend-service-1
        container_names = []

        # Check if call_graph is configured
        if app_config and app_config.get("call_graph"):
            # Call graph mode: generate container names for each service
            call_graph = app_config["call_graph"]
            for service_def in call_graph.get("services", []):
                service_id = service_def["id"]
                service_name = f"local-{service_id.lower().replace('_', '-')}-service"
                replicas = service_def.get("replicas", 1)

                # Generate container names for each replica
                # Docker compose naming: {project}-{service}-{replica_number}
                # Service names use "local-{service_id}-service" to match frontend expectations
                for i in range(1, replicas + 1):
                    # Docker compose creates containers like: {project}-{service}-{i}
                    # We use a pattern that matches what docker compose generates
                    container_names.append(f"{service_name}-{i}")
        else:
            # Traditional mode: get child replica count
            child_replicas = int(env_vars.get("CHILD_REPLICAS", 1))

            # Generate container names for each child replica
            for i in range(1, child_replicas + 1):
                container_names.append(f"local-child-service-{i}")

        return container_names

    def _validate_config(
        self, repo_root: Path, config_path: Path, executor: CommandExecutor
    ) -> None:
        """
        Validate the experiment configuration using the rust validation tool.
        """
        logger.info(f"Validating config: {config_path}")

        cmd = [
            "cargo",
            "run",
            "-q",
            "--release",
            "-p",
            "synthetic",
            "--bin",
            "validate_config",
            "--",
            "--config",
            str(config_path),
        ]

        try:
            executor.run(
                cmd,
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )
            logger.info("Config validation passed")
        except subprocess.CalledProcessError as e:
            logger.error(f"Config validation failed:\n{e.stderr}")
            raise RuntimeError(f"Config validation failed for {config_path}")

    def generate_k8s_values(
        self,
        project_name: str,
        image_tag: str,
        app_config_path: Optional[Path],
        env_vars: dict,
    ) -> dict:
        """
        Generate Helm chart values for Kubernetes deployment.
        """
        values = {}
        values["fullnameOverride"] = project_name

        if app_config_path:
            try:
                with open(app_config_path) as f:
                    values["appConfig"] = json.load(f)
            except Exception as e:
                logger.warning(f"Failed to load app config: {e}")

        if "LOG_LEVEL" in env_vars:
            values["logLevel"] = env_vars["LOG_LEVEL"]

        # Image tag
        if "image" not in values:
            values["image"] = {}
        values["image"]["tag"] = image_tag
        return values

    def prepare_workload(
        self,
        config: "ExperimentConfig",
        policy: str,
        iteration: int,
        output_dir: Path,
        repo_root: Path,
        use_k8s: bool = False,
        executor: Optional[CommandExecutor] = None,
    ) -> dict:
        """
        Prepare workload configuration and environment variables.
        """
        if self._is_new_generator_enabled():
            return self._prepare_workload_new(
                config=config,
                policy=policy,
                iteration=iteration,
                output_dir=output_dir,
                repo_root=repo_root,
                use_k8s=use_k8s,
                executor=executor,
            )

        return self._prepare_workload_legacy(
            config=config,
            policy=policy,
            iteration=iteration,
            output_dir=output_dir,
            repo_root=repo_root,
            use_k8s=use_k8s,
            executor=executor,
        )

    def _prepare_workload_legacy(
        self,
        config: "ExperimentConfig",
        policy: str,
        iteration: int,
        output_dir: Path,
        repo_root: Path,
        use_k8s: bool = False,
        executor: Optional[CommandExecutor] = None,
    ) -> dict:
        """
        Legacy workload preparation path (pre-generator).
        """
        executor = executor or SubprocessExecutor()
        docker_config = self.get_docker_config()

        # Generate project name
        project_name = _safe_project_name(
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )

        # Generate base env vars
        env_vars = self.generate_env_vars(
            config.gen_config,
            config.app_config,
            config.app_dir,
        )

        # Add image tag
        image_tag = self.get_image_tag(policy)
        env_vars[f"{self.get_app_name().upper()}_IMAGE_TAG"] = image_tag
        env_vars["DOCKER_COMPOSE_PROJECT_NAME"] = project_name
        logger.debug(f"Set {self.get_app_name().upper()}_IMAGE_TAG={image_tag}")

        # Prepare config files
        output_dir.mkdir(parents=True, exist_ok=True)
        gen_config_path = output_dir / "gen_config.json"
        template_gen_config_path = config.in_dir / "gen_config.json"

        # Check for app config (hotel.json / config.docker.json)
        app_config_path = None
        if docker_config.app_config_filename:
            candidate = config.in_dir / docker_config.app_config_filename
            if candidate.exists():
                app_config_path = candidate
                # Pass app config path to docker compose as env var for volume mounting
                env_vars["APP_CONFIG_PATH"] = str(app_config_path.resolve())

                # Validate config
                self._validate_config(repo_root, app_config_path, executor)

        if use_k8s:
            # K8s Preparation
            # Service name override for K8s (typically {project}-frontend)
            service_name = f"{project_name}-frontend"

            # 1. Calculate values (Pure)
            values = self.generate_k8s_values(
                project_name=project_name,
                image_tag=image_tag,
                app_config_path=app_config_path,
                env_vars=env_vars,
            )

            # 2. Write values (Effects)
            values_file = output_dir / "values.yaml"
            with open(values_file, "w", encoding="utf-8") as f:
                yaml.dump(values, f)
            env_vars["HELM_VALUES_FILE"] = str(values_file.resolve())

        else:
            # Docker Preparation
            service_name = None  # Docker Compose handles DNS via service aliases

            # Handle call_graph mode which generates a dynamic compose file
            if config.app_config and config.app_config.get("call_graph"):
                # 1. Calculate compose content (Pure)
                compose_content = self.create_call_graph_compose_dict(
                    app_config=config.app_config,
                    image_tag=image_tag,
                )

                # 2. Write compose file (Effects)
                compose_path = output_dir / "docker-compose-callgraph.yaml"
                with open(compose_path, "w") as f:
                    yaml.dump(
                        compose_content, f, default_flow_style=False, sort_keys=False
                    )

                self._generated_compose_path = compose_path
                logger.info(f"Generated call graph docker compose file: {compose_path}")
                logger.info(
                    f"Services in call graph: {[s['id'] for s in config.app_config['call_graph']['services']]}"
                )

        # Generate gen_config.json
        # 1. Calculate content (Pure)
        gen_config_content = self.create_gen_config_dict(
            template_config_path=template_gen_config_path,
            project_name=project_name,
            service_name_override=service_name,
        )

        # 2. Write file (Effects)
        with open(gen_config_path, "w") as f:
            json.dump(gen_config_content, f, indent=2)

        logger.debug(
            f"Generated gen_config.json at {gen_config_path} with address {gen_config_content.get('Addr', 'N/A')} for project {project_name}"
        )

        return env_vars

    def _prepare_workload_new(
        self,
        config: "ExperimentConfig",
        policy: str,
        iteration: int,
        output_dir: Path,
        repo_root: Path,
        use_k8s: bool = False,
        executor: Optional[CommandExecutor] = None,
    ) -> dict:
        """
        New generator-based workload preparation.
        """
        if use_k8s:
            raise NotImplementedError(
                "New generator pipeline does not support Kubernetes yet"
            )

        executor = executor or SubprocessExecutor()
        output_dir.mkdir(parents=True, exist_ok=True)

        project_name = generate_project_name(
            app=self.get_app_name(),
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )
        image_tag = self.get_image_tag(policy)

        experiment_v2 = convert_legacy_to_experiment_config(
            config.gen_config,
            config.policies,
            config.experiment_name,
            config.app_name,
        )
        self._experiment_config_v2 = experiment_v2

        topology = self._resolve_topology_for_new_stack(
            repo_root=repo_root,
            experiment=experiment_v2,
            config=config,
        )
        topology = self.customize_topology(topology)

        compose_generator = ComposeGenerator()
        generated = compose_generator.generate(
            topology=topology,
            experiment=experiment_v2,
            output_dir=output_dir,
            project_name=project_name,
            policy=policy,
            image_tag=image_tag,
        )
        self._generated_deployment = generated
        self._generated_compose_path = None

        env_vars = generated.env_vars.copy()
        env_vars["DOCKER_COMPOSE_PROJECT_NAME"] = project_name
        env_vars[f"{self.get_app_name().upper()}_IMAGE_TAG"] = image_tag

        app_config_payload = self._build_app_config_payload(config, topology)
        app_config_path = output_dir / "config.generated.json"
        with open(app_config_path, "w", encoding="utf-8") as f:
            json.dump(app_config_payload, f, indent=2)
        self._validate_config(repo_root, app_config_path, executor)
        env_vars["APP_CONFIG_PATH"] = str(app_config_path)

        template_gen_config_path = config.in_dir / "gen_config.json"
        gen_config_path = output_dir / "gen_config.json"
        gen_config_content = self.create_gen_config_dict(
            template_config_path=template_gen_config_path,
            project_name=project_name,
            service_name_override=None,
        )
        with open(gen_config_path, "w", encoding="utf-8") as f:
            json.dump(gen_config_content, f, indent=2)

        env_vars = self.customize_env_vars(topology, experiment_v2, env_vars)
        logger.info(
            "Prepared synthetic workload via new generator "
            f"(project={project_name}, policy={policy})"
        )

        return env_vars

    def _resolve_topology_for_new_stack(
        self,
        repo_root: Path,
        experiment: "ExperimentConfigV2",
        config: "ExperimentConfig",
    ) -> TopologySpec:
        resolver = TopologyResolver(repo_root)

        if experiment.topology_ref:
            base_topology = resolver.resolve(
                app_name=self.get_app_name(),
                topology_ref=experiment.topology_ref,
            )
        elif config.app_config and config.app_config.get("call_graph"):
            base_topology = TopologySpec(
                app=self.get_app_name(),
                description="Generated from legacy synthetic config",
                call_graph=json.loads(json.dumps(config.app_config["call_graph"])),
            )
        else:
            base_topology = resolver.resolve(
                app_name=self.get_app_name(),
                topology_ref="default",
            )

        return resolver.apply_overrides(
            base_topology,
            experiment.replica_overrides,
        )

    def _build_app_config_payload(
        self,
        config: "ExperimentConfig",
        topology: TopologySpec,
    ) -> dict[str, Any]:
        if config.app_config:
            return config.app_config

        if topology.call_graph:
            return {"call_graph": topology.call_graph}

        raise ValueError("Synthetic app requires either app_config or call_graph topology")

    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> Tuple[Path, str]:
        if use_k8s:
            return repo_root / "charts/synthetic", "."

        if self._is_new_generator_enabled() and self._generated_deployment:
            return (
                self._generated_deployment.deploy_root,
                self._generated_deployment.deploy_file,
            )

        # Docker
        if self._generated_compose_path:
            return output_dir, self._generated_compose_path.name

        docker_config = self.get_docker_config()
        # Default scripts dir
        return repo_root / "exp/synthetic/scripts", docker_config.compose_file

    def get_loadgen_spec(
        self,
        output_dir: Path,
        features: Optional[str],
        env_vars: dict,
        use_k8s: bool,
    ) -> TaskSpec:

        project_name = env_vars.get("DOCKER_COMPOSE_PROJECT_NAME")

        # Create the legacy load generator helper to reuse logic if possible,
        # or just define properties here.
        # The legacy classes (SyntheticLoadGenerator) are still useful for names/images

        # Determine image and binary
        tag = self.get_image_tag(features)
        image = (
            f"synthetic_client_bench:{tag}" if tag else "synthetic_client_bench:latest"
        )
        binary = "synthetic_client_bench"

        # Network
        network = (
            f"{project_name}_synthetic_network"
            if project_name
            else "local_synthetic_network"
        )
        if use_k8s:
            network = None  # Not used in K8s TaskSpec usually

        # Env Vars
        task_env = {
            "BINARY_NAME": binary,
            "LOG_LEVEL": env_vars.get("LOG_LEVEL", "info"),
        }
        # Merge other relevant env vars
        # For synthetic, we might need some from the main env_vars?
        # The legacy code calls get_env_vars(env_vars) which updates base_env with env_vars.
        task_env.update(env_vars)

        # Config mounting
        gen_config_path = output_dir / "gen_config.json"

        volumes = {}
        if gen_config_path.exists():
            # Host path -> Container path
            volumes[str(gen_config_path)] = "/usr/gen_config.json"

        return TaskSpec(
            name=f"{project_name}-loadgen" if project_name else "synthetic-loadgen",
            image=image,
            env_vars=task_env,
            network=network,
            volumes=volumes,
            cleanup=True,
            artifacts=[
                ("/tmp/masa-load-gen/.", ".")
            ],  # Source in container, dest dir in host
        )


class SyntheticBuilder(AppBuilder):
    """
    Build logic for the synthetic app docker images.

    Uses multi-stage multi-target build:
    - Stage 1 (builder): Build all binaries once
    - Stage 2 (runtime-base): Base runtime image with dependencies
    - Stage 3 (runtime): Per-binary runtime images

    The synthetic app requires three separate docker images:
    - synthetic_frontend:latest - frontend service
    - synthetic_child:latest - child service (can be scaled)
    - synthetic_client_bench:latest - load generator
    """

    def build(
        self,
        *,
        repo_root: Path,
        app_dir: Path,
        features: Optional[str] = None,
        rust_log: str = "info",
        no_cache: bool = False,
        gen_config_path: Optional[Path] = None,
        dry_run: bool = False,
        build_logs_dir: Optional[Path] = None,
        executor: Optional[CommandExecutor] = None,
    ) -> Optional[list[list[str]]]:
        executor = executor or SubprocessExecutor()
        app = "synthetic"

        # Services to build (each gets its own image)
        binaries = [
            "synthetic_frontend",
            "synthetic_child",
            "synthetic_client_bench",
        ]

        # Generate tag based on features for deterministic, feature-specific images
        tag = normalize_features_to_tag(features)

        logger.info(
            f"Building {len(binaries)} docker images for synthetic app using multi-stage build"
        )
        if features:
            logger.info(f"Using features: {features}")

        # Start timing the docker build
        build_start_time = time.time()

        if gen_config_path is None:
            raise ValueError("gen_config_path is required for synthetic app")
        gen_config_path_rel = gen_config_path.relative_to(repo_root)

        try:
            # Stage 1: Build all binaries once (shared across all images)
            # Note: We don't use --load for intermediate stages (builder, runtime-base)
            # to avoid slow layer export. BuildKit handles COPY --from and FROM internally.
            # Only final runtime images use --load since they're used by docker-compose.
            # logger.info("Stage 1: Building all binaries for synthetic app")
            logger.info("Stage 1: Building all binaries for synthetic app")
            builder_build_args: list[str] = []
            if features:
                builder_build_args.extend(["--build-arg", f"FEATURES={features}"])
            builder_build_args.extend(["--build-arg", f"APP={app}"])
            # Use a unique cache ID to avoid race conditions in parallel builds
            cache_id = f"{app}-{tag}"
            builder_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

            builder_cmd: list[str] = [
                "docker",
                "buildx",
                "build",
                "-f",
                "./exp_runner/common/docker-build/Dockerfile",
                "--target",
                "builder",
                # Don't use --load for builder stage - it's an intermediate stage
                # BuildKit handles COPY --from=builder internally without loading.
                # Removing --load avoids slow layer export (~300s -> ~10s on some systems).
                *builder_build_args,
                "--ulimit",
                "nofile=4096:4096",
                get_docker_progress_flag(),
            ]

            if no_cache:
                builder_cmd.append("--no-cache")

            # Don't tag builder stage - it's only used as intermediate stage
            # The stage is still available for COPY --from=builder in subsequent stages.
            builder_cmd.append(".")

            try:
                executor.run(
                    builder_cmd,
                    cwd=repo_root,
                    check=True,
                    capture_output=False,
                )
            except subprocess.CalledProcessError:
                logger.error(
                    f"Failed to build builder stage. Command: {shlex.join(builder_cmd)}"
                )
                raise
            logger.info("Stage 1 complete: All binaries built")

            # Stage 2: Build runtime-base (shared across all images)
            logger.info("Stage 2: Building runtime-base image")
            runtime_base_build_args: list[str] = []
            if features:
                runtime_base_build_args.extend(["--build-arg", f"FEATURES={features}"])
            runtime_base_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
            runtime_base_build_args.extend(["--build-arg", f"APP={app}"])
            runtime_base_build_args.extend(
                ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
            )
            # Use consistent cache ID based on features across all stages
            runtime_base_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

            runtime_base_cmd: list[str] = [
                "docker",
                "buildx",
                "build",
                "-f",
                "./exp_runner/common/docker-build/Dockerfile",
                "--target",
                "runtime-base",
                # Don't use --load for runtime-base - it's an intermediate stage.
                # BuildKit handles FROM runtime-base internally without loading.
                # Removing --load avoids slow layer export (~300s -> ~10s on some systems).
                *runtime_base_build_args,
                "--ulimit",
                "nofile=4096:4096",
                get_docker_progress_flag(),
            ]

            if no_cache:
                runtime_base_cmd.append("--no-cache")

            # Tag runtime-base for potential inspection/debugging, but don't load it
            runtime_base_cmd.extend(["-t", f"{app}_runtime-base:{tag}", "."])

            try:
                executor.run(
                    runtime_base_cmd,
                    cwd=repo_root,
                    check=True,
                    capture_output=False,
                )
            except subprocess.CalledProcessError:
                logger.error(
                    f"Failed to build runtime-base stage. Command: {shlex.join(runtime_base_cmd)}"
                )
                raise
            logger.info("Stage 2 complete: Runtime-base image built")

            # Stage 3: Build per-binary runtime images
            for binary_name in binaries:
                logger.info(f"Stage 3: Building runtime image for {binary_name}")

                runtime_build_args: list[str] = []
                if features:
                    runtime_build_args.extend(["--build-arg", f"FEATURES={features}"])
                runtime_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
                runtime_build_args.extend(["--build-arg", f"APP={app}"])
                runtime_build_args.extend(
                    ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
                )
                runtime_build_args.extend(["--build-arg", f"BINARY_NAME={binary_name}"])
                # Use consistent cache ID based on features across all stages
                runtime_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

                # Image name: synthetic_<binary>:<tag> or synthetic_<binary>:latest if no features
                if tag:
                    image_name = f"{binary_name}:{tag}"
                else:
                    image_name = f"{binary_name}:latest"

                runtime_cmd: list[str] = [
                    "docker",
                    "buildx",
                    "build",
                    "-f",
                    "./exp_runner/common/docker-build/Dockerfile",
                    "--target",
                    "runtime",
                    "--load",
                    *runtime_build_args,
                    "--ulimit",
                    "nofile=4096:4096",
                    get_docker_progress_flag(),
                ]

                if no_cache:
                    runtime_cmd.append("--no-cache")

                runtime_cmd.extend(["-t", image_name, "."])

                try:
                    executor.run(
                        runtime_cmd,
                        cwd=repo_root,
                        check=True,
                        capture_output=False,
                    )
                except subprocess.CalledProcessError:
                    logger.error(
                        f"Failed to build runtime image for {binary_name}. Command: {shlex.join(runtime_cmd)}"
                    )
                    raise

                logger.info(f"Successfully built docker image: {image_name}")

        finally:
            pass


        if dry_run and isinstance(executor, MockCommandExecutor):
            return [cmd.args for cmd in executor.history]

        # Calculate and print build duration
        build_duration = time.time() - build_start_time
        logger.info(
            f"Docker image building took {build_duration:.2f} seconds ({build_duration / 60:.2f} minutes)"
        )
        logger.info("All synthetic app docker images built successfully")
        return None
