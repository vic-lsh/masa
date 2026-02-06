"""
Socialnet application plugin.
"""

import hashlib
import json
import logging
import os
import re
import shlex
import subprocess
import time
from pathlib import Path
from typing import TYPE_CHECKING, Optional

from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor, SubprocessExecutor
from .base import AppBuilder, AppPlugin, DockerConfig
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
        build_logs_dir: Optional[Path] = None,
        executor: Optional[CommandExecutor] = None,
    ) -> Optional[list[list[str]]]:
        executor = executor or SubprocessExecutor()
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

        executor.run(builder_cmd, cwd=repo_root, check=True)
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

        executor.run(runtime_base_cmd, cwd=repo_root, check=True)
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
                "--load",
            ]

            if no_cache:
                runtime_cmd.append("--no-cache")

            runtime_cmd.append(".")

            logger.info(f"Building {binary_tag}...")
            executor.run(runtime_cmd, cwd=repo_root, check=True)
            logger.info(f"Built {binary_tag}")

        build_duration = time.time() - build_start_time
        logger.info(f"All docker images built in {build_duration:.2f} seconds")

        if dry_run and hasattr(executor, "history"):
            return [cmd.args for cmd in executor.history]
        return None


class SocialnetApp(AppPlugin):
    """
    Plugin for the socialnet microservices application.

    The socialnet app consists of multiple microservices (compose_post, home_timeline,
    user_timeline, post_storage, etc.) that can be replicated.
    """

    def get_app_name(self) -> str:
        return "socialnet"

    # ===== NEW SIMPLIFIED INTERFACE (Phase 4) =====

    def get_binaries(self) -> list[str]:
        """Return list of binary names for socialnet app."""
        return [
            "textservice_server",
            "usermention_server",
            "url_shorten_server",
            "user_service",
            "user_timeline_server",
            "post_storage_server",
            "compose_post_server",
            "home_timeline_server",
            "social_graph_server",
            "write_home_timeline_server",
            "media_service",
            "unique_id_service",
            "socialnet_client_bench",
        ]

    def get_frontend_name(self) -> str:
        """Return the frontend service name."""
        return "compose-post-service"

    def get_cargo_package(self) -> str:
        """Return cargo package name."""
        return "socialnet"

    def get_build_parallelism(self) -> int:
        """Socialnet has 13 binaries - use parallelism of 4."""
        return 4

    def get_default_topology_path(self, repo_root: Path) -> Optional[Path]:
        """
        Socialnet topology is implicit in the application code.
        Return None to indicate no explicit topology file.
        """
        return None

    def customize_topology(self, topology):
        """
        Hook for socialnet-specific topology transformations.
        Currently no transformations needed.
        """
        return topology

    def customize_env_vars(self, topology, experiment, base_env):
        """
        Add socialnet-specific environment variables.

        Args:
            topology: Topology specification
            experiment: Experiment configuration
            base_env: Base environment variables from generator

        Returns:
            Updated environment variables
        """
        import os

        # Add JWT secret for authentication
        if "JWT_SECRET" not in base_env:
            base_env["JWT_SECRET"] = os.environ.get(
                "JWT_SECRET", "test-secret-key-for-ci"
            )

        # Add any socialnet-specific env vars if needed
        if "LOG_LEVEL" not in base_env:
            base_env["LOG_LEVEL"] = "info"

        # Compose post port
        if "COMPOSE_POST_PORT" not in base_env:
            base_env["COMPOSE_POST_PORT"] = "8080"

        return base_env

    # ===== LEGACY INTERFACE (backward compatibility) =====

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
            compose_file="docker-compose.yaml",
            network_name="socialnet-network",
            loadgen_image_name="socialnet_client_bench:<features>",
            loadgen_binary_name="socialnet_client_bench",
            app_config_filename="socialnet.json",
        )

    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for socialnet application.
        """
        project_name = env_vars.get("DOCKER_COMPOSE_PROJECT_NAME")
        if not project_name:
            return []

        # Single replica services
        services = [
            "compose-post-service",
            "home-timeline-service",
            "post-storage-service",
            "social-graph-service",
            "write-home-timeline-service",
            "user-service",
            "media-service",
            "unique-id-service",
            "text-service",
            "user-mention-service",
            "url-shorten-service",
        ]

        container_names = []
        for service in services:
            container_names.append(f"{project_name}-{service}-1")

        # Scaled services
        # user-timeline-service is hardcoded to scale: 4 in docker-compose.yaml
        for i in range(1, 5):
            container_names.append(f"{project_name}-user-timeline-service-{i}")

        return container_names

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

    def prepare_workload(
        self,
        config: "ExperimentConfig",
        policy: str,
        iteration: int,
        output_dir: Path,
        repo_root: Path,
        use_k8s: bool = False,
        executor: Optional[CommandExecutor] = None,
        use_new_generator: bool = False,
    ) -> dict:
        """
        Prepare workload configuration and environment variables.
        """
        # Store state for get_deployment_location
        self._last_use_k8s = use_k8s
        self._last_use_new_generator = use_new_generator

        if use_new_generator:
            from ..topology import TopologyResolver
            from ..generators.compose import ComposeGenerator
            from ..generators.helm import HelmValuesGenerator
            from ..experiment_config_v2 import (
                ExperimentConfigV2,
                ExecutionSpec,
                LoadGenSpec,
            )

            # 1. Resolve Topology
            resolver = TopologyResolver(repo_root)
            topology = resolver.resolve("socialnet", "default")

            # 2. Create ExperimentConfigV2 adapter
            exp_v2 = ExperimentConfigV2(
                name=config.experiment_name,
                app="socialnet",
                execution=ExecutionSpec(
                    repeats=1,
                    policies=[policy],
                ),
                replica_overrides={},
                loadgen=LoadGenSpec(rps=[]),
            )

            # Extract overrides
            if config.app_config:
                overrides = {}
                for svc, data in config.app_config.items():
                    if isinstance(data, dict) and "replicas" in data:
                        # Map legacy to topology names
                        # e.g. "user-timeline" -> "user-timeline-service"
                        # In legacy socialnet.json, keys match service names mostly
                        # But let's look at get_container_names in legacy:
                        # "user-timeline-service" is scaled.
                        # The legacy config key is "user-timeline" or "user-timeline-service"?
                        # socialnet.json usually has keys matching docker-compose service names.
                        overrides[svc] = int(data["replicas"])
                exp_v2.replica_overrides = overrides

            # 3. Generate Deployment
            project_name = _safe_project_name(
                experiment_name=config.experiment_name,
                iteration=iteration,
                policy=policy,
            )

            tag = self.get_image_tag(features=policy)
            image_tag = tag if tag else "latest"

            if use_k8s:
                generator = HelmValuesGenerator()
                deploy = generator.generate(
                    topology=topology,
                    experiment=exp_v2,
                    output_dir=output_dir,
                    project_name=project_name,
                    policy=policy,
                    image_tag=image_tag,
                )

                # Update gen_config Address for K8s
                # Service name is {project_name}-compose-post-service
                frontend_addr = f"http://{project_name}-compose-post-service:8080"

            else:
                generator = ComposeGenerator()
                deploy = generator.generate(
                    topology=topology,
                    experiment=exp_v2,
                    output_dir=output_dir,
                    project_name=project_name,
                    policy=policy,
                    image_tag=image_tag,
                )
                # Docker Compose uses service name aliases
                frontend_addr = "http://compose-post-service:8080"

            # 4. Generate gen_config.json
            import copy

            gen_config = copy.deepcopy(config.gen_config)
            gen_config["Addr"] = frontend_addr

            project_gen_config_path = output_dir / "gen_config.json"
            with project_gen_config_path.open("w") as f:
                json.dump(gen_config, f, indent=2)

            return deploy.env_vars

        if use_k8s:
            raise NotImplementedError(
                "Socialnet app does not support Kubernetes yet (legacy path)"
            )

        # Generate project name
        project_name = _safe_project_name(
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )

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
        env_vars["DOCKER_COMPOSE_PROJECT_NAME"] = project_name

        # Add image tag
        tag = self.get_image_tag(features=policy)
        env_vars["SOCIALNET_IMAGE_TAG"] = tag if tag else "latest"

        # Handle app config path
        docker_config = self.get_docker_config()
        if docker_config.app_config_filename:
            candidate = config.in_dir / docker_config.app_config_filename
            if candidate.exists():
                env_vars["APP_CONFIG_PATH"] = str(candidate.resolve())

        return env_vars

    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> tuple[Path, str]:
        # Check if we generated new files
        if getattr(self, "_last_use_new_generator", False):
            if use_k8s:
                return repo_root / "charts/socialnet", str(output_dir / "values.yaml")
            else:
                return output_dir, "docker-compose.yaml"

        if use_k8s:
            raise NotImplementedError("Socialnet app does not support Kubernetes yet")

        # Assume standard location
        return repo_root / "apps/socialnet", "docker-compose.yaml"

    def get_loadgen_spec(
        self,
        output_dir: Path,
        features: Optional[str],
        env_vars: dict,
        use_k8s: bool,
    ) -> TaskSpec:
        project_name = env_vars.get("DOCKER_COMPOSE_PROJECT_NAME")

        tag = self.get_image_tag(features)
        image = (
            f"socialnet_client_bench:{tag}" if tag else "socialnet_client_bench:latest"
        )
        binary = "socialnet_client_bench"

        network = (
            f"{project_name}_socialnet-network" if project_name else "socialnet-network"
        )

        task_env = {
            "BINARY_NAME": binary,
            "LOG_LEVEL": env_vars.get("LOG_LEVEL", "info"),
        }
        task_env.update(env_vars)

        gen_config_path = output_dir / "gen_config.json"
        volumes = {}
        if gen_config_path.exists():
            volumes[str(gen_config_path)] = "/usr/gen_config.json"

        return TaskSpec(
            name=f"{project_name}-loadgen" if project_name else "socialnet-loadgen",
            image=image,
            env_vars=task_env,
            network=network,
            volumes=volumes,
            cleanup=True,
            artifacts=[("/tmp/masa-load-gen/.", ".")],
        )
