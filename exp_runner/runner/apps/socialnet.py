"""
Socialnet application plugin.
"""

import hashlib
import json
import logging
import os
import re
import shlex
import shutil
import subprocess
import time
from pathlib import Path
from typing import TYPE_CHECKING, Optional

from ..cpu_monitor import CPUMonitor
from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import get_docker_progress_flag, normalize_features_to_tag

if TYPE_CHECKING:
    from exp_runner.runner.config import ExperimentConfig
    from exp_runner.runner.docker_manager import DockerManager

logger = logging.getLogger(__name__)


def _safe_project_name(*, experiment_name: str, iteration: int, policy: str) -> str:
    """Generate safe docker-compose project name for socialnet experiments.

    Format: socialnet-{slug}-{digest}
    - slug: sanitized experiment name (max 12 chars to keep total name under 63 char docker limit)
    - digest: 12-char hash for uniqueness
    """
    raw = f"{experiment_name}|{iteration}|{policy}"
    digest = hashlib.sha256(raw.encode("utf-8")).hexdigest()[:12]
    slug = re.sub(r"[^a-z0-9]+", "-", experiment_name.lower()).strip("-")[:12] or "exp"
    return f"socialnet-{slug}-{digest}"


def _generate_gen_config(
    *,
    template_config: dict,
    project_name: str,
    output_path: Path,
) -> None:
    """
    Generate project-specific gen_config.json with namespaced frontend address.

    Updates the "Addr" field to point to the project-prefixed frontend service.
    """
    import copy

    config = copy.deepcopy(template_config)

    # Use the service alias on the compose network so DNS returns all replicas.
    # The service name in docker-compose is "compose-post-service" and port is 8080.
    # Note: Inside the project network, "compose-post-service" resolves correctly.
    config["Addr"] = "http://compose-post-service:8080"

    # Write to file
    with output_path.open("w") as f:
        json.dump(config, f, indent=2)


class SocialnetLoadGenerator(LoadGenerator):
    """Load generator for the socialnet application."""

    def __init__(
        self, features: Optional[str] = None, project_name: Optional[str] = None
    ):
        """
        Initialize load generator with optional features for image tagging.

        Args:
            features: Cargo features used to build the image
            project_name: Docker compose project name for namespace isolation
        """
        self.features = features
        self.project_name = project_name

    def get_container_name(self) -> str:
        if self.project_name:
            return f"{self.project_name}_socialnet_client_bench"
        return "socialnet_client_bench"

    def get_network_name(self) -> str:
        if self.project_name:
            # Docker Compose creates network named {project_name}_{network_key}
            # Our network key in docker-compose.yaml is "socialnet-network"
            return f"{self.project_name}_socialnet-network"
        return "socialnet-network"

    def get_image_name(self) -> str:
        tag = normalize_features_to_tag(self.features)
        if tag and tag != "latest":
            return f"socialnet_client_bench:{tag}"
        else:
            return "socialnet_client_bench:latest"

    def get_binary_name(self) -> str:
        return "socialnet_client_bench"


class SocialnetBuilder(AppBuilder):
    """
    Build logic for the socialnet app docker images.

    Uses multi-stage multi-target build similar to hotel app.
    The socialnet app requires separate docker images for each binary.
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
        app = "socialnet"
        # List of binaries to build (each gets its own image)
        binaries_list = [
            "socialnet_client_bench",
            "compose_post_server",
            "home_timeline_server",
            "user_timeline_server",
            "post_storage_server",
            "social_graph_server",
            "write_home_timeline_server",
            "user_service",
            "media_service",
            "unique_id_service",
            "textservice_server",
            "usermention_server",
            "url_shorten_server",
        ]

        if gen_config_path is None:
            raise ValueError("gen_config_path is required for socialnet app")

        # Convert to path relative to repo_root
        gen_config_path_rel = gen_config_path.relative_to(repo_root)

        # Generate tag based on features for deterministic, feature-specific images
        tag = normalize_features_to_tag(features)

        logger.info(
            f"Building {len(binaries_list)} docker images for socialnet app using multi-stage build"
        )
        if features:
            logger.info(f"Using features: {features}")

        # Collect commands if dry_run
        commands: list[list[str]] = []

        # Start timing the docker build
        build_start_time = time.time()

        # Stage 1: Build all binaries once (shared across all images)
        logger.info("Stage 1: Building all binaries for socialnet app")
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
            *builder_build_args,
            "--ulimit",
            "nofile=4096:4096",
            get_docker_progress_flag(),
        ]

        if no_cache:
            builder_cmd.append("--no-cache")

        builder_cmd.extend(["-t", f"{app}_builder:{tag}", "."])

        if dry_run:
            commands.append(builder_cmd)
            logger.info(f"[DRY RUN] Would run: {shlex.join(builder_cmd)}")
        else:
            logger.info(f"Running: {shlex.join(builder_cmd)}")
            result = subprocess.run(builder_cmd, cwd=repo_root, check=True)
            logger.info("Stage 1 complete")

        # Stage 2: Build runtime-base image (shared dependencies)
        logger.info("Stage 2: Building runtime-base image")
        runtime_base_build_args: list[str] = []
        if features:
            runtime_base_build_args.extend(["--build-arg", f"FEATURES={features}"])
        runtime_base_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
        runtime_base_build_args.extend(["--build-arg", f"APP={app}"])
        runtime_base_build_args.extend(
            ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
        )
        runtime_base_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

        runtime_base_cmd: list[str] = [
            "docker",
            "buildx",
            "build",
            "-f",
            "./exp_runner/common/docker-build/Dockerfile",
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
            commands.append(runtime_base_cmd)
            logger.info(f"[DRY RUN] Would run: {shlex.join(runtime_base_cmd)}")
        else:
            logger.info(f"Running: {shlex.join(runtime_base_cmd)}")
            subprocess.run(runtime_base_cmd, cwd=repo_root, check=True)
            logger.info("Stage 2 complete")

        # Stage 3: Build individual runtime images for each binary
        logger.info("Stage 3: Building individual runtime images")
        for binary in binaries_list:
            binary_tag = f"{binary}:{tag}" if tag != "latest" else f"{binary}:latest"

            runtime_build_args: list[str] = []
            if features:
                runtime_build_args.extend(["--build-arg", f"FEATURES={features}"])
            runtime_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
            runtime_build_args.extend(["--build-arg", f"APP={app}"])
            runtime_build_args.extend(
                ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
            )
            runtime_build_args.extend(["--build-arg", f"BINARY_NAME={binary}"])
            runtime_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

            runtime_cmd: list[str] = [
                "docker",
                "buildx",
                "build",
                "-f",
                "./exp_runner/common/docker-build/Dockerfile",
                "--target",
                "runtime",
                *runtime_build_args,
                "--ulimit",
                "nofile=4096:4096",
                "-t",
                binary_tag,
                get_docker_progress_flag(),
            ]

            if no_cache:
                runtime_cmd.append("--no-cache")

            runtime_cmd.append(".")

            if dry_run:
                commands.append(runtime_cmd)
                logger.info(f"[DRY RUN] Would run: {shlex.join(runtime_cmd)}")
            else:
                logger.info(f"Building {binary_tag}...")
                subprocess.run(runtime_cmd, cwd=repo_root, check=True)
                logger.info(f"Built {binary_tag}")

        build_duration = time.time() - build_start_time
        logger.info(f"All docker images built in {build_duration:.2f} seconds")

        if dry_run:
            return commands
        return None


class SocialnetApp(AppPlugin):
    """
    Plugin for the socialnet microservices application.

    The socialnet app consists of multiple microservices (compose_post, home_timeline,
    user_timeline, post_storage, etc.) that can be replicated.
    """

    def get_app_name(self) -> str:
        return "socialnet"

    def load_app_config(self, config_path: Path) -> dict:
        """Load socialnet config file if provided."""
        if config_path.exists():
            with open(config_path) as f:
                return json.load(f)
        return {}

    def generate_env_vars(
        self, gen_config: dict, app_config: Optional[dict], app_dir: Path
    ) -> dict:
        """
        Generate environment variables for socialnet application.

        Extracts compose_post port from gen_config and sets replica counts.
        """
        env_vars = {}

        # We don't rely on extracting port from Addr here for internal service config
        # because we use hardcoded 8080 inside the container network.
        # But we still populate it just in case.
        env_vars["COMPOSE_POST_PORT"] = "8080"

        if "JWT_SECRET" not in env_vars:
            env_vars["JWT_SECRET"] = os.environ.get(
                "JWT_SECRET", "test-secret-key-for-ci"
            )

        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "info"

        return env_vars

    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for socialnet application."""
        return DockerConfig(
            compose_file="apps/socialnet/docker-compose.yaml",
            network_name="socialnet-network",
            loadgen_image_name="socialnet_client_bench:<features>",
            loadgen_binary_name="socialnet_client_bench",
            app_config_filename="socialnet.json",
        )

    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for socialnet application.
        """
        # This method is primarily used if run_workload is NOT overridden,
        # or if we need to predict names without docker client.
        # Since we override run_workload and use docker.get_container_names,
        # this might not be strictly needed, but good to keep consistent.
        # However, with project names, we can't easily predict names here without project name argument.
        # So we return empty or basic names.
        return []

    def create_load_generator(
        self, features: Optional[str] = None, project_name: Optional[str] = None
    ) -> LoadGenerator:
        """
        Create a load generator instance for socialnet application.

        Args:
            features: Optional cargo features used to build the image
            project_name: Optional docker compose project name
        """
        return SocialnetLoadGenerator(features=features, project_name=project_name)

    def create_builder(self) -> AppBuilder:
        """
        Create a builder instance for socialnet application.
        """
        return SocialnetBuilder()

    def get_image_tag(self, features: Optional[str] = None) -> str:
        """
        Get the docker image tag for the given features.

        Args:
            features: Optional cargo features

        Returns:
            Image tag string
        """
        return normalize_features_to_tag(features)

    def run_workload(
        self,
        *,
        repo_root: Path,
        config: "ExperimentConfig",
        docker: "DockerManager",
        policy: str,
        iteration: int,
        output_dir: Path,
        app_local_dir: Path,
        no_cache: bool,
        dry_run: bool = False,
        **kwargs,
    ) -> None:
        """Run socialnet experiment with namespace isolation."""

        # Generate project name for namespace isolation
        project_name = _safe_project_name(
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )

        # Setup paths - write directly to output_dir
        output_dir.mkdir(parents=True, exist_ok=True)

        # Generate project-specific gen_config.json
        project_gen_config_path = output_dir / "gen_config.json"
        _generate_gen_config(
            template_config=config.gen_config,
            project_name=project_name,
            output_path=project_gen_config_path,
        )

        # Generate environment variables
        env_vars = self.generate_env_vars(
            config.gen_config, config.app_config, config.app_dir
        )
        env_vars["PROJECT_NAME"] = project_name

        # Add image tag based on policy/features
        tag = self.get_image_tag(features=policy)
        env_vars["SOCIALNET_IMAGE_TAG"] = tag if tag else "latest"

        # Compute config paths
        docker_config = self.get_docker_config()
        app_config_path = None
        if docker_config.app_config_filename:
            candidate = config.in_dir / docker_config.app_config_filename
            if not candidate.exists():
                raise FileNotFoundError(f"App config not found at: {candidate}")
            app_config_path = candidate
            # Pass app config path to docker compose as env var for volume mounting
            env_vars["APP_CONFIG_PATH"] = str(app_config_path.resolve())

        gen_config_path = config.in_dir / "gen_config.json"
        if not gen_config_path.exists():
            raise FileNotFoundError(f"gen_config.json not found at: {gen_config_path}")

        # Build docker images (use ORIGINAL config for build)
        builder = self.create_builder()
        build_cmds = builder.build(
            repo_root=repo_root,
            app_dir=config.app_dir,
            features=policy,
            rust_log=env_vars.get("LOG_LEVEL", "info"),
            no_cache=no_cache,
            gen_config_path=gen_config_path,
            dry_run=dry_run,
        )

        if dry_run and build_cmds:
            print("\n".join(" ".join(cmd) for cmd in build_cmds))

        docker_compose_path = config.app_dir / "docker-compose.yaml"

        # Setup environment for docker compose
        env = os.environ.copy()
        env.update({k: str(v) for k, v in env_vars.items()})

        # Docker compose commands with project name
        up_cmd = [
            "docker",
            "compose",
            "-f",
            str(docker_compose_path),
            "-p",
            project_name,
            "up",
            "-d",
        ]
        down_cmd = [
            "docker",
            "compose",
            "-f",
            str(docker_compose_path),
            "-p",
            project_name,
            "down",
            "--volumes",
        ]

        if dry_run:
            print(
                f"[dry-run] would run socialnet policy={policy} iteration={iteration}"
            )
            print(f"[dry-run] project name: {project_name}")
            print("[dry-run] compose up:", " ".join(up_cmd))
            print("[dry-run] compose down:", " ".join(down_cmd))
            return

        # Save metadata
        metadata = {
            "app": "socialnet",
            "experiment": config.experiment_name,
            "iteration": iteration,
            "policy": policy,
            "docker_project": project_name,
        }
        with (output_dir / "metadata.json").open("w", encoding="utf-8") as fh:
            json.dump(metadata, fh, indent=2, sort_keys=True)

        # Initialize CPU monitor with project name filter
        cpu_stats_file = output_dir / "cpu_stats.csv"
        cpu_monitor = CPUMonitor(
            output_path=cpu_stats_file, poll_interval=2.0, container_prefix=project_name
        )

        log_threads = []
        try:
            # Start services
            logger.info(
                f"Starting socialnet services for policy={policy} iteration={iteration} project={project_name}"
            )
            subprocess.run(
                up_cmd,
                cwd=config.app_dir,
                env=env,
                check=True,
            )

            # Wait for services to be ready
            time.sleep(30)

            # Start CPU monitoring after services are up
            cpu_monitor.start()

            # Get container names for log streaming
            container_names = docker.get_container_names(
                compose_path=docker_compose_path,
                project_name=project_name,
                env_vars=env,
            )

            # Stream logs
            if container_names:
                logs_dir = output_dir / "logs"
                logger.info(
                    f"Streaming logs for {len(container_names)} containers to {logs_dir}"
                )
                log_threads = docker.stream_logs(
                    container_names=container_names,
                    output_dir=logs_dir,
                    follow=True,
                )
            else:
                logger.warning("No containers found for log streaming")

            # Run load generator
            # We pass the project-specific gen_config so the client connects to the correct service
            load_gen = self.create_load_generator(
                features=policy, project_name=project_name
            )
            load_gen.run(
                output_dir=output_dir,
                env_vars=env_vars,
                gen_config_path=project_gen_config_path,
            )

        finally:
            # Stop CPU monitor
            try:
                cpu_monitor.stop()
            except Exception as e:
                logger.warning(f"Error stopping CPU monitor: {e}")

            # Cleanup
            subprocess.run(down_cmd, cwd=config.app_dir, env=env, check=False)

            # Wait for log threads
            for thread in log_threads:
                thread.join(timeout=5)
