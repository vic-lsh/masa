"""
Hotel application plugin.
"""

import json
import logging
import re
import shlex
import subprocess
import time
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import (
    get_docker_progress_flag,
    normalize_features_to_tag,
    update_hotel_config_with_prefix,
)

logger = logging.getLogger(__name__)


class HotelLoadGenerator(LoadGenerator):
    """Load generator for the hotel reservation application."""

    def __init__(self, features: Optional[str] = None, project_prefix: str = ""):
        """
        Initialize load generator with optional features for image tagging.

        Args:
            features: Cargo features used to build the image
            project_prefix: PROJECT_PREFIX for parallel execution support
        """
        self.features = features
        self.project_prefix = project_prefix

    def get_container_name(self) -> str:
        return f"{self.project_prefix}hotel_client_bench"

    def get_network_name(self) -> str:
        # Network name uses the prefix (e.g., "exp1_0_fifo_hotel_network")
        # If no prefix, fall back to the original network name
        if self.project_prefix:
            return f"{self.project_prefix}hotel_network"
        return "local_hotel_network"

    def get_image_name(self) -> str:
        tag = normalize_features_to_tag(self.features)
        if tag and tag != "latest":
            return f"hotel_client_bench:{tag}"
        else:
            return "hotel_client_bench:latest"

    def get_binary_name(self) -> str:
        return "hotel_client_bench"


class HotelBuilder(AppBuilder):
    """
    Build logic for the hotel app docker images.

    Uses multi-stage multi-target build:
    - Stage 1 (builder): Build all binaries once
    - Stage 2 (runtime-base): Base runtime image with dependencies
    - Stage 3 (runtime): Per-binary runtime images

    The hotel app requires separate docker images for each binary:
    - hotel_client_bench:latest - load generator
    - hotel_frontend:latest - frontend service
    - hotel_geo:latest - geo service
    - hotel_rate:latest - rate service
    - hotel_review:latest - review service
    - hotel_search:latest - search service
    - hotel_profile:latest - profile service
    - hotel_reservation:latest - reservation service
    - hotel_user:latest - user service
    - hotel_recommendation:latest - recommendation service

    Mirrors the behavior of exp/common/scripts/docker-build.sh, but lives in Python
    so the runner can select an app-specific build implementation.
    """

    def build(
        self,
        *,
        repo_root: Path,
        app_dir: Path,
        features: Optional[str] = None,
        rust_log: str = "info",
        no_cache: bool = False,
        app_config_path: Optional[Path] = None,
        gen_config_path: Optional[Path] = None,
        dry_run: bool = False,
        cpu_affinity: Optional[list[int]] = None,
        log_file: Optional[Path] = None,
    ) -> Optional[list[list[str]]]:
        app = "hotel"
        # List of binaries to build (each gets its own image)
        # Note: All binary names already include the hotel_ prefix
        binaries_list = [
            "hotel_client_bench",
            "hotel_frontend",
            "hotel_geo",
            "hotel_rate",
            "hotel_review",
            "hotel_search",
            "hotel_profile",
            "hotel_reservation",
            "hotel_user",
            "hotel_recommendation",
        ]

        if app_config_path is None:
            raise ValueError("app_config_path is required for hotel app")

        if gen_config_path is None:
            raise ValueError("gen_config_path is required for hotel app")

        # Convert to path relative to repo_root
        config_path_rel = app_config_path.relative_to(repo_root)
        gen_config_path_rel = gen_config_path.relative_to(repo_root)

        # Generate tag based on features for deterministic, feature-specific images
        tag = normalize_features_to_tag(features)

        logger.info(
            f"Building {len(binaries_list)} docker images for hotel app using multi-stage build"
        )
        if features:
            logger.info(f"Using features: {features}")

        # Collect commands if dry_run
        commands: list[list[str]] = []

        # Start timing the docker build
        build_start_time = time.time()

        # Helper function to execute commands with optional CPU affinity and log redirection
        def run_build_command(cmd: list[str], stage_name: str) -> None:
            """Execute a build command with optional CPU affinity and log redirection."""
            # Prepend taskset for CPU affinity if specified
            if cpu_affinity:
                cpu_list = ",".join(str(cpu) for cpu in cpu_affinity)
                cmd = ["taskset", "-c", cpu_list] + cmd
                logger.debug(f"Running {stage_name} with CPU affinity: {cpu_list}")

            # If redirecting to log file, replace --progress=tty with --progress=plain
            if log_file:
                # Replace progress flag for file output (tty doesn't work with redirection)
                cmd = [
                    arg if arg != "--progress=tty" else "--progress=plain"
                    for arg in cmd
                ]

            # Determine output redirection
            if log_file:
                # Redirect stdout and stderr to log file
                logger.info(f"{stage_name} - logs redirected to {log_file}")
                with open(log_file, "a", encoding="utf-8") as f:
                    f.write(f"\n{'=' * 80}\n")
                    f.write(f"{stage_name}\n")
                    f.write(f"{'=' * 80}\n")
                    f.write(f"Command: {shlex.join(cmd)}\n\n")
                    f.flush()
                    subprocess.run(
                        cmd,
                        cwd=repo_root,
                        check=True,
                        stdout=f,
                        stderr=subprocess.STDOUT,
                    )
            else:
                # Output to terminal (original behavior)
                subprocess.run(
                    cmd,
                    cwd=repo_root,
                    check=True,
                    capture_output=False,
                )

        # Stage 1: Build all binaries once (shared across all images)
        logger.info("Stage 1: Building all binaries for hotel app")
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
            "./exp/common/docker-build/Dockerfile",
            "--target",
            "builder",
            *builder_build_args,
            "--ulimit",
            "nofile=4096:4096",
            get_docker_progress_flag(),
        ]

        if no_cache:
            builder_cmd.append("--no-cache")

        builder_cmd.extend(["-t", f"{app}_builder:{tag}", "."])

        if dry_run:
            commands.append(builder_cmd.copy())
        else:
            try:
                run_build_command(builder_cmd, "Stage 1: Builder")
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
            ["--build-arg", f"APP_CONFIG_PATH={config_path_rel}"]
        )
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
            "./exp/common/docker-build/Dockerfile",
            "--target",
            "runtime-base",
            *runtime_base_build_args,
            "--ulimit",
            "nofile=4096:4096",
            get_docker_progress_flag(),
        ]

        if no_cache:
            runtime_base_cmd.append("--no-cache")

        runtime_base_cmd.extend(["-t", f"{app}_runtime-base:{tag}", "."])

        if dry_run:
            commands.append(runtime_base_cmd.copy())
        else:
            try:
                run_build_command(runtime_base_cmd, "Stage 2: Runtime-base")
            except subprocess.CalledProcessError as e:
                logger.error(
                    f"Failed to build runtime-base stage. Command: {shlex.join(runtime_base_cmd)}"
                )
                raise
        logger.info("Stage 2 complete: Runtime-base image built")

        # Stage 3: Build per-binary runtime images
        for binary_name in binaries_list:
            logger.info(f"Stage 3: Building runtime image for {binary_name}")

            runtime_build_args: list[str] = []
            if features:
                runtime_build_args.extend(["--build-arg", f"FEATURES={features}"])
            runtime_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
            runtime_build_args.extend(["--build-arg", f"APP={app}"])
            runtime_build_args.extend(
                ["--build-arg", f"APP_CONFIG_PATH={config_path_rel}"]
            )
            runtime_build_args.extend(
                ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
            )
            runtime_build_args.extend(["--build-arg", f"BINARY_NAME={binary_name}"])
            # Use consistent cache ID based on features across all stages
            runtime_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

            # Generate image name: <binary>:<tag> or <binary>:latest if no features
            # Note: binary_name already includes the hotel_ prefix
            if tag:
                image_name = f"{binary_name}:{tag}"
            else:
                image_name = f"{binary_name}:latest"

            runtime_cmd: list[str] = [
                "docker",
                "buildx",
                "build",
                "-f",
                "./exp/common/docker-build/Dockerfile",
                "--target",
                "runtime",
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
                    run_build_command(runtime_cmd, f"Stage 3: Runtime ({binary_name})")
                except subprocess.CalledProcessError as e:
                    logger.error(
                        f"Failed to build runtime image for {binary_name}. Command: {shlex.join(runtime_cmd)}"
                    )
                    raise

            logger.info(f"Successfully built docker image: {image_name}")

        if dry_run:
            return commands

        # Calculate and print build duration
        build_duration = time.time() - build_start_time
        logger.info(
            f"Docker image building took {build_duration:.2f} seconds ({build_duration / 60:.2f} minutes)"
        )
        logger.info("All hotel app docker images built successfully")
        return None


class HotelApp(AppPlugin):
    """
    Plugin for the hotel reservation microservices application.

    The hotel app consists of multiple microservices (rate, profile, reservation,
    geo, search, user, recommendation, review) that can be replicated.
    """

    def get_app_name(self) -> str:
        return "hotel"

    def load_app_config(self, config_path: Path) -> dict:
        """Load hotel.json configuration file."""
        with open(config_path) as f:
            return json.load(f)

    def generate_env_vars(
        self, gen_config: dict, app_config: Optional[dict], app_dir: Path
    ) -> dict:
        """
        Generate environment variables for hotel application.

        Extracts frontend port from gen_config and replica counts from hotel.json.
        """
        env_vars = {}

        # Extract frontend port from address (e.g., "http://[::1]:8659" -> "8659")
        addr = gen_config.get("Addr", "")
        match = re.search(r":(\d+)$", addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")

        # Set replica counts from hotel.json
        default_replicas = 1
        services = [
            "rate",
            "profile",
            "reservation",
            "geo",
            "search",
            "user",
            "recommendation",
            "review",
        ]

        if app_config:
            for service in services:
                env_key = f"{service.upper()}_REPLICAS"
                replicas = app_config.get(service, {}).get("replicas", default_replicas)
                env_vars[env_key] = str(replicas)
        else:
            # Use defaults if no config provided
            for service in services:
                env_key = f"{service.upper()}_REPLICAS"
                env_vars[env_key] = str(default_replicas)

        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "info"

        return env_vars

    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for hotel application."""
        return DockerConfig(
            compose_file="scripts/local/containers+svcs.yaml",
            network_name="local_hotel_network",
            loadgen_container_name="hotel_client_bench",
            loadgen_image_name="hotel_client_bench:<features>",  # Actual tag is dynamic based on features
            loadgen_binary_name="hotel_client_bench",
            app_config_filename="hotel.json",
        )

    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for hotel application.

        Includes frontend and replicated service containers based on
        replica counts from environment variables.

        Container names are prefixed with PROJECT_PREFIX if present in env_vars
        to support parallel execution.
        """
        prefix = env_vars.get("PROJECT_PREFIX", "")

        # Frontend has a fixed container_name
        container_names = [f"{prefix}hotel_frontend"]

        # MongoDB and Redis containers (with fixed container_name)
        mongo_redis_containers = [
            f"{prefix}rate_mongo",
            f"{prefix}rate_redis",
            f"{prefix}profile_mongo",
            f"{prefix}profile_redis",
            f"{prefix}reservation_mongo",
            f"{prefix}reservation_redis",
            f"{prefix}user_mongo",
        ]
        container_names.extend(mongo_redis_containers)

        # Services that can be replicated (scaled)
        # Docker Compose doesn't support both scale and container_name,
        # so these use auto-generated names, but we can still collect logs
        # by discovering them dynamically or using a pattern
        # For now, we'll skip these from the static list since docker.stream_logs
        # can discover them via docker compose ps

        return container_names

    def create_load_generator(
        self, features: Optional[str] = None, project_prefix: str = ""
    ) -> LoadGenerator:
        """
        Create a load generator instance for hotel application.

        Args:
            features: Optional cargo features used to build the image
            project_prefix: Optional PROJECT_PREFIX for parallel execution support
        """
        return HotelLoadGenerator(features=features, project_prefix=project_prefix)

    def create_builder(self) -> AppBuilder:
        """Create a builder instance for hotel application."""
        return HotelBuilder()

    def get_image_tag(self, features: Optional[str] = None) -> str:
        """
        Get the docker image tag for the given features.

        Args:
            features: Optional cargo features

        Returns:
            Docker image tag string
        """
        return normalize_features_to_tag(features)

    def _generate_project_prefix(
        self, experiment_name: str, iteration: int, policy: str
    ) -> str:
        """
        Generate a unique PROJECT_PREFIX for this experiment run.

        This allows multiple policies to run in parallel without container name collisions.

        Args:
            experiment_name: Name of the experiment
            iteration: Iteration number
            policy: Policy name

        Returns:
            Project prefix string (e.g., "exp1_0_fifo_")
        """
        # Sanitize names to be docker-safe (alphanumeric, dash, underscore)
        safe_exp = re.sub(r"[^a-zA-Z0-9_-]", "_", experiment_name)
        safe_policy = re.sub(r"[^a-zA-Z0-9_-]", "_", policy)

        # Create a unique prefix: {experiment}_{iteration}_{policy}_
        return f"{safe_exp}_{iteration}_{safe_policy}_"

    def run_workload(
        self,
        *,
        repo_root: "Path",
        config: "ExperimentConfig",  # type: ignore
        docker: "DockerManager",  # type: ignore
        policy: str,
        iteration: int,
        output_dir: "Path",
        app_local_dir: "Path",
        no_cache: bool,
        dry_run: bool = False,
        cpu_affinity: Optional[list[int]] = None,
        build_log_file: Optional["Path"] = None,
    ) -> None:
        """
        Run a single (iteration, policy) workload for hotel application.

        Overrides the default implementation to support parallel execution by:
        - Generating a unique PROJECT_PREFIX for container/network namespacing
        - Creating a modified hotel.json with prefixed service names
        - Updating container names to include the prefix
        """
        docker_config = self.get_docker_config()

        # Generate unique project prefix for parallel execution
        project_prefix = self._generate_project_prefix(
            config.experiment_name, iteration, policy
        )
        logger.info(f"Using PROJECT_PREFIX: {project_prefix}")

        # Generate environment variables
        env_vars = self.generate_env_vars(
            config.gen_config,
            config.app_config,
            config.app_dir,
        )

        # Add PROJECT_PREFIX for docker-compose variable substitution
        env_vars["PROJECT_PREFIX"] = project_prefix

        # Add image tag
        image_tag = self.get_image_tag(policy)
        env_vars["HOTEL_IMAGE_TAG"] = image_tag
        logger.debug(f"Set HOTEL_IMAGE_TAG={image_tag}")

        # Compute config paths
        app_config_path = None
        if docker_config.app_config_filename:
            candidate = config.in_dir / docker_config.app_config_filename
            if not candidate.exists():
                raise FileNotFoundError(f"App config not found at: {candidate}")
            app_config_path = candidate

        gen_config_path = config.in_dir / "gen_config.json"
        if not gen_config_path.exists():
            raise FileNotFoundError(f"gen_config.json not found at: {gen_config_path}")

        # Create a modified hotel.json with prefixed service names for parallel execution
        # This is written to a temporary location and used for the Docker build
        modified_hotel_config = update_hotel_config_with_prefix(
            config.app_config, project_prefix
        )

        # Write the modified config to the output directory
        modified_config_path = output_dir / "hotel.json"
        if not dry_run:
            output_dir.mkdir(parents=True, exist_ok=True)
            with open(modified_config_path, "w", encoding="utf-8") as f:
                json.dump(modified_hotel_config, f, indent=2)
            logger.debug(f"Wrote modified hotel.json to {modified_config_path}")

        # Build images (use the modified config)
        builder = self.create_builder()
        if dry_run:
            commands = builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=no_cache,
                app_config_path=modified_config_path
                if not dry_run
                else app_config_path,
                gen_config_path=gen_config_path,
                dry_run=True,
            )
            if commands:
                print("\n".join(shlex.join(cmd) for cmd in commands))
            print(
                f"[dry-run] would run hotel iteration={iteration} policy={policy} prefix={project_prefix}"
            )
            return

        # Write .env file expected by compose setups
        env_file = app_local_dir / ".env"
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
            app_config_path=modified_config_path,
            gen_config_path=gen_config_path,
            dry_run=False,
            cpu_affinity=cpu_affinity,
            log_file=build_log_file,
        )

        try:
            docker.start(
                app_dir=config.app_dir,
                compose_file=docker_config.compose_file,
                env_vars=env_vars,
            )

            # Start streaming logs in background
            container_names = self.get_container_names(env_vars)
            docker.stream_logs(
                container_names=container_names,
                output_dir=output_dir,
                follow=True,
            )

            # Run load generator (blocking)
            loadgen = self.create_load_generator(
                features=policy, project_prefix=project_prefix
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
            # Stop Docker services
            docker.stop(
                app_dir=config.app_dir,
                compose_file=docker_config.compose_file,
                env_vars=env_vars,
            )

            # Clean up .env file
            if env_file.exists():
                env_file.unlink()
