"""
Synthetic application plugin.
"""

import json
import logging
import re
import shlex
import subprocess
import tempfile
import time
import hashlib
from pathlib import Path
from typing import Optional

import yaml

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import normalize_features_to_tag, get_docker_progress_flag

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
    slug = re.sub(r"[^a-z0-9]+", "-", experiment_name.lower()).strip("-")[:16] or "exp"
    return f"synthetic-{slug}-{digest}"


class SyntheticLoadGenerator(LoadGenerator):
    """Load generator for the synthetic benchmark application."""

    def __init__(
        self,
        features: Optional[str] = None,
        project_name: Optional[str] = None,
        network_name: Optional[str] = None,
    ):
        """
        Initialize load generator with optional features for image tagging.

        Args:
            features: Cargo features used to build the image
            project_name: Docker Compose project name (used to determine network name)
        """
        self.features = features
        self.project_name = project_name
        self.network_name = network_name

    def get_container_name(self) -> str:
        if self.project_name:
            return f"{self.project_name}_synthetic_client_bench"
        return "synthetic_client_bench"

    def get_network_name(self) -> str:
        """
        Return the Docker network name to connect to.

        Uses the docker-compose project's network: {project_name}_synthetic_network
        Docker Compose creates networks as {project_name}_{network_key} when networks
        are defined in the compose file.
        """
        if self.network_name:
            return self.network_name
        if self.project_name:
            return f"{self.project_name}_synthetic_network"
        # Fallback for the static compose which pins network name
        return "local_synthetic_network"

    def get_image_name(self) -> str:
        tag = normalize_features_to_tag(self.features)
        if tag and tag != "latest":
            return f"synthetic_client_bench:{tag}"
        else:
            return "synthetic_client_bench:latest"

    def get_binary_name(self) -> str:
        return "synthetic_client_bench"


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

    def get_app_name(self) -> str:
        return "synthetic"

    def load_app_config(self, config_path: Path) -> dict:
        """Load config.docker.json configuration file."""
        with open(config_path) as f:
            return json.load(f)

    def _generate_gen_config(
        self,
        template_config_path: Path,
        output_path: Path,
        project_name: str,
    ) -> None:
        """
        Generate project-specific gen_config.json with correct frontend service name.

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
            config["Addr"] = f"{protocol}://synthetic-frontend-service:{port}"

        # Write to file
        output_path.parent.mkdir(parents=True, exist_ok=True)
        with open(output_path, "w") as f:
            json.dump(config, f, indent=2)

        logger.debug(
            f"Generated gen_config.json at {output_path} with address {config.get('Addr', 'N/A')} for project {project_name}"
        )

    def _generate_call_graph_compose(
        self,
        app_dir: Path,
        app_config: dict,
        image_tag: str,
        app_config_path: Optional[Path],
        output_dir: Path,
    ) -> Path:
        """
        Generate a docker compose file for call graph configuration.

        Creates a compose file with:
        - Frontend service
        - One service per call graph service, each with its own SERVICE_ID

        Args:
            app_dir: Application directory
            app_config: Application configuration dict
            image_tag: Docker image tag
            app_config_path: Path to app config file
            output_dir: Output directory for generated compose file

        Returns:
            Path to generated compose file
        """
        call_graph = app_config.get("call_graph")
        if not call_graph:
            raise ValueError("call_graph not found in app_config")

        services = {}

        # Add frontend service
        # Build depends_on list for all call graph services
        # Use "local-{service-id}-service" naming to match service names
        depends_on = [
            f"local-{svc['id'].lower()}-service" for svc in call_graph["services"]
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
            service_name = f"local-{service_id.lower()}-service"
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

        # Write compose file to output directory
        output_dir.mkdir(parents=True, exist_ok=True)
        compose_path = output_dir / "docker-compose-callgraph.yaml"

        with open(compose_path, "w") as f:
            yaml.dump(compose_content, f, default_flow_style=False, sort_keys=False)

        logger.info(f"Generated call graph docker compose file: {compose_path}")
        logger.info(
            f"Services in call graph: {[s['id'] for s in call_graph['services']]}"
        )

        return compose_path

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
                service_name = f"local-{service_id.lower()}-service"
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

    def create_load_generator(
        self,
        features: Optional[str] = None,
        project_name: Optional[str] = None,
        network_name: Optional[str] = None,
    ) -> LoadGenerator:
        """
        Create a load generator instance for synthetic application.

        Args:
            features: Optional cargo features used to build the image
            project_name: Docker Compose project name (used to determine network name)
            network_name: Optional explicit docker network name to connect to
        """
        return SyntheticLoadGenerator(
            features=features,
            project_name=project_name,
            network_name=network_name,
        )

    def create_builder(self) -> AppBuilder:
        """
        Create a builder instance for synthetic application.
        """
        return SyntheticBuilder()

    def get_image_tag(self, features: Optional[str] = None) -> str:
        """
        Get the docker image tag for the given features.

        Args:
            features: Optional cargo features

        Returns:
            Docker image tag string
        """
        return normalize_features_to_tag(features)

    def run_workload(
        self,
        *,
        repo_root: Path,
        config,  # ExperimentConfig
        docker,  # DockerManager
        policy: str,
        iteration: int,
        output_dir: Path,
        app_local_dir: Path,
        no_cache: bool,
        dry_run: bool = False,
    ) -> None:
        """
        Run a single (iteration, policy) workload.

        Overrides base implementation to handle call graph compose file generation.
        """
        from .base import CPUMonitor

        docker_config = self.get_docker_config()

        # Generate environment variables
        env_vars = self.generate_env_vars(
            config.gen_config,
            config.app_config,
            config.app_dir,
        )

        # Add image tag if app supports it (feature-specific images)
        image_tag = self.get_image_tag(policy)
        env_vars[f"{config.app_name.upper()}_IMAGE_TAG"] = image_tag
        logger.debug(f"Set {config.app_name.upper()}_IMAGE_TAG={image_tag}")

        # Compute config paths for builds/loadgen
        app_config_path = None
        if docker_config.app_config_filename:
            candidate = config.in_dir / docker_config.app_config_filename
            if not candidate.exists():
                raise FileNotFoundError(f"App config not found at: {candidate}")
            app_config_path = candidate
            # Pass app config path to docker compose as env var for volume mounting
            env_vars["APP_CONFIG_PATH"] = str(app_config_path.resolve())

        # Use a safe docker-compose project name.
        # Policy strings may contain commas (e.g., "fifo,early") which Docker Compose
        # will normalize, causing mismatches if we try to build names from the raw policy.
        project_name = _safe_project_name(
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )

        # Pass project name to services via environment variable
        env_vars["DOCKER_COMPOSE_PROJECT_NAME"] = project_name

        # Use template gen_config.json for Docker build (like hotel app does)
        # The build happens before we generate the project-specific config
        template_gen_config_path = config.in_dir / "gen_config.json"
        if not template_gen_config_path.exists():
            raise FileNotFoundError(
                f"gen_config.json not found at: {template_gen_config_path}"
            )

        # Generate project-specific gen_config.json for load generator (with correct service name)
        # Docker Compose DNS resolution uses service names, not container names
        output_dir.mkdir(parents=True, exist_ok=True)
        gen_config_path = output_dir / "gen_config.json"
        self._generate_gen_config(
            template_config_path=template_gen_config_path,
            output_path=gen_config_path,
            project_name=project_name,
        )

        # Generate call graph compose file if needed
        compose_file = docker_config.compose_file
        # Default to experiment scripts dir for static compose
        compose_app_dir = repo_root / "exp/synthetic/scripts"

        # Check if call_graph is configured
        has_call_graph = (
            config.app_config is not None
            and "call_graph" in config.app_config
            and config.app_config.get("call_graph") is not None
        )

        if config.app_config is not None and "call_graph" not in config.app_config:
            logger.warning(
                f"config.docker.json exists but does not contain 'call_graph' key. "
                f"Available keys: {list(config.app_config.keys())}"
            )

        if has_call_graph:
            if config.app_config is None:
                raise ValueError("app_config is required for call_graph mode")

            logger.info("Call graph detected, generating docker compose file")
            generated_compose = self._generate_call_graph_compose(
                app_dir=config.app_dir,
                app_config=config.app_config,
                image_tag=image_tag,
                app_config_path=app_config_path,
                output_dir=output_dir,
            )
            # Use output_dir as app_dir and just the filename for compose_file
            compose_file = generated_compose.name
            compose_app_dir = output_dir
            self._generated_compose_path = generated_compose

        # Choose the network name for the load generator.
        # - Static compose (docker-compose.yaml) defines network key "synthetic_network".
        # - Generated call-graph compose also defines network key "synthetic_network".
        # In both cases, Docker Compose creates "{project_name}_synthetic_network".
        loadgen_network_name = f"{project_name}_synthetic_network"

        # Build images (use template gen_config.json for build, not project-specific one)
        # The project-specific gen_config.json is only used by the load generator at runtime
        builder = self.create_builder()
        if dry_run:
            commands = builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=no_cache,
                gen_config_path=template_gen_config_path,
                dry_run=True,
            )
            if commands:
                print("\n".join(shlex.join(cmd) for cmd in commands))
            print(
                f"[dry-run] would run {config.app_name} iteration={iteration} policy={policy}"
            )
            return

        # Write .env file expected by compose setups (only when actually running)
        env_file = output_dir / ".env"
        env_file.parent.mkdir(parents=True, exist_ok=True)
        with open(env_file, "w", encoding="utf-8") as f:
            for key, value in env_vars.items():
                f.write(f"{key}={value}\n")
        logger.debug(f"Wrote environment variables to {env_file}")

        builder.build(
            repo_root=repo_root,
            app_dir=config.app_dir,
            features=policy,
            rust_log="info",
            no_cache=no_cache,
            gen_config_path=template_gen_config_path,
            dry_run=False,
        )

        # Initialize CPU monitor
        cpu_stats_file = output_dir / "cpu_stats.csv"
        cpu_monitor = CPUMonitor(output_path=cpu_stats_file, poll_interval=2.0)

        try:
            docker.start(
                app_dir=compose_app_dir,
                compose_file=compose_file,
                env_vars=env_vars,
                project_name=project_name,
            )

            # Start CPU monitoring after services are up
            cpu_monitor.start()

            # Get container names for log streaming
            # Query Docker Compose for actual container names (includes project prefix)
            compose_path = compose_app_dir / compose_file
            container_names = docker.get_container_names(
                compose_path=compose_path,
                project_name=project_name,
                env_vars=env_vars,
            )
            if container_names:
                logs_dir = output_dir / "logs"
                logs_dir.mkdir(parents=True, exist_ok=True)
                logger.info(
                    f"Streaming logs for {len(container_names)} containers to {logs_dir}"
                )
                docker.stream_logs(
                    container_names=container_names,
                    output_dir=logs_dir,
                    follow=True,
                )

            # Run load generator
            loadgen = self.create_load_generator(
                features=policy,
                project_name=project_name,
                network_name=loadgen_network_name,
            )
            loadgen.run(
                output_dir=output_dir,
                env_vars=env_vars,
                gen_config_path=gen_config_path,
            )

            logger.info(f"Load generator completed for policy {policy}")

            # Wait a moment for logs to flush
            time.sleep(2)
        finally:
            # Stop CPU monitoring before stopping services
            try:
                cpu_monitor.stop()
            except Exception as e:
                logger.warning(f"Error stopping CPU monitor: {e}")

            # Stop Docker services
            docker.stop(
                app_dir=compose_app_dir,
                compose_file=compose_file,
                env_vars=env_vars,
                project_name=project_name,
            )

            # Keep generated compose file for debugging/inspection
            # (Previously cleaned up, but kept for troubleshooting)


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
    ) -> Optional[list[list[str]]]:
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

        # Collect commands if dry_run
        commands: list[list[str]] = []

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

            if dry_run:
                commands.append(builder_cmd.copy())
            else:
                try:
                    subprocess.run(
                        builder_cmd,
                        cwd=repo_root,
                        check=True,
                        capture_output=False,
                    )
                except subprocess.CalledProcessError as e:
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

            if dry_run:
                commands.append(runtime_base_cmd.copy())
            else:
                try:
                    subprocess.run(
                        runtime_base_cmd,
                        cwd=repo_root,
                        check=True,
                        capture_output=False,
                    )
                except subprocess.CalledProcessError as e:
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

                if dry_run:
                    commands.append(runtime_cmd.copy())
                else:
                    try:
                        subprocess.run(
                            runtime_cmd,
                            cwd=repo_root,
                            check=True,
                            capture_output=False,
                        )
                    except subprocess.CalledProcessError as e:
                        logger.error(
                            f"Failed to build runtime image for {binary_name}. Command: {shlex.join(runtime_cmd)}"
                        )
                        raise

                logger.info(f"Successfully built docker image: {image_name}")

        finally:
            pass

        if dry_run:
            return commands

        # Calculate and print build duration
        build_duration = time.time() - build_start_time
        logger.info(
            f"Docker image building took {build_duration:.2f} seconds ({build_duration / 60:.2f} minutes)"
        )
        logger.info("All synthetic app docker images built successfully")
        return None
