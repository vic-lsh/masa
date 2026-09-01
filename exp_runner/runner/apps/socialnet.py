"""
Socialnet application plugin.
"""

import json
import logging
import os
import shlex
import subprocess
import time
from pathlib import Path
from typing import TYPE_CHECKING, Optional

import yaml

from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor, SubprocessExecutor
from ..naming import generate_project_name
from .base import AppBuilder, AppPlugin, DockerConfig
from .utils import (
    get_docker_progress_flag,
    normalize_features_to_tag,
)

if TYPE_CHECKING:
    from exp_runner.runner.config import ExperimentConfig

logger = logging.getLogger(__name__)


def _generate_gen_config(
    *,
    template_config: dict,
    project_name: str,
    output_path: Path,
    use_k8s: bool = False,
) -> None:
    """
    Generate project-specific gen_config.json with namespaced frontend address.

    Updates the "Addr" field to point to the unified frontend service. In k8s
    mode the hostname includes the project prefix and the `-1` suffix that
    matches the chart's per-service Service naming convention.
    """
    import copy

    config = copy.deepcopy(template_config)

    if use_k8s:
        config["Addr"] = f"http://{project_name}-frontend-service-1:8080"
    else:
        # Use the unified frontend service as the benchmark entrypoint
        # (compose service alias on the bridge network).
        config["Addr"] = "http://frontend-service:8080"

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
            "frontend_server",
            "register_user_server",
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
            try:
                executor.run(builder_cmd, cwd=repo_root, check=True)
            except subprocess.CalledProcessError:
                logger.error(
                    "Failed to build builder stage. Command: %s",
                    shlex.join(builder_cmd),
                )
                raise
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
            try:
                executor.run(runtime_base_cmd, cwd=repo_root, check=True)
            except subprocess.CalledProcessError:
                logger.error(
                    "Failed to build runtime-base stage. Command: %s",
                    shlex.join(runtime_base_cmd),
                )
                raise
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

            if dry_run:
                commands.append(runtime_cmd)
                logger.info(f"[DRY RUN] Would run: {shlex.join(runtime_cmd)}")
            else:
                logger.info(f"Building {binary_tag}...")
                try:
                    executor.run(runtime_cmd, cwd=repo_root, check=True)
                except subprocess.CalledProcessError:
                    logger.error(
                        "Failed to build runtime image %s. Command: %s",
                        binary_tag,
                        shlex.join(runtime_cmd),
                    )
                    raise
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

    @property
    def supports_k8s(self) -> bool:
        return True

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
            "frontend-service",
            "register-user-service",
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
    ) -> dict:
        """
        Prepare workload configuration and environment variables.
        """
        # Generate project name
        project_name = generate_project_name(
            prefix="sn",
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
            use_k8s=use_k8s,
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

        project_policy_params_path = self._write_policy_params(
            output_dir, config.policy_params, policy
        )
        env_vars["POLICY_PARAMS_PATH"] = str(project_policy_params_path)

        if use_k8s:
            self._prepare_k8s_workload(
                output_dir=output_dir,
                project_name=project_name,
                app_config=config.app_config or {},
                policy_params_path=project_policy_params_path,
                image_tag=tag if tag else "latest",
                log_level=env_vars.get("LOG_LEVEL", "info"),
                jwt_secret=env_vars.get("JWT_SECRET", "test-secret-key-for-ci"),
                env_vars=env_vars,
            )

        return env_vars

    def _prepare_k8s_workload(
        self,
        *,
        output_dir: Path,
        project_name: str,
        app_config: dict,
        policy_params_path: Path,
        image_tag: str,
        log_level: str,
        jwt_secret: str,
        env_vars: dict,
    ) -> None:
        """
        Build a Helm `values.yaml` for the socialnet chart and set HELM_VALUES_FILE.

        Each microservice's env vars are pre-resolved here with the project
        prefix baked into hostnames, so the addresses resolve via k8s DNS to
        the chart-generated Services. Service-to-service calls go through
        `{project}-{svc}-1` k8s Services (matching the Rust LoadBalancedChannel
        `{base}-1..{base}-N` convention); infra (mongo/redis/memcached/rabbitmq)
        is reached at `{project}-{name}` directly.

        We pin every `*_REPLICAS` env var to 1 in k8s mode (mirrors the tracebench
        pattern) so a single Service per microservice load-balances across the
        underlying Deployment pods.
        """
        p = project_name

        def svc_host(svc_name: str) -> str:
            # Hostname the Rust client passes to LoadBalancedChannel; the
            # client appends `-1..-N`, so this returns the unsuffixed base.
            return f"{p}-{svc_name}"

        def infra_host(name: str) -> str:
            # Plain hostname mode (mongo/redis/memcached/rabbitmq).
            return f"{p}-{name}"

        # Microservices. Ports default to 8080 (matches docker-compose).
        services = [
            {
                "name": "post-storage-service",
                "binary": "post_storage_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "POST_STORAGE_MONGODB_URI",
                        "value": f"mongodb://{infra_host('post-storage-mongo')}:27017",
                    },
                    {"name": "POST_STORAGE_MONGODB_DATABASE", "value": "post-storage"},
                    {
                        "name": "POST_STORAGE_MEMCACHED_ADDR",
                        "value": f"tcp://{infra_host('post-storage-memcached')}:11211",
                    },
                ],
            },
            {
                # Mirrors `scale: 4` in apps/socialnet/docker-compose.yaml —
                # the chart emits 4 Deployments + 4 Services, and the
                # compose-post client below sets USER_TIMELINE_REPLICAS=4.
                "name": "user-timeline-service",
                "binary": "user_timeline_server",
                "replicas": 4,
                "port": 8080,
                "env": [
                    {
                        "name": "USER_TIMELINE_MONGODB_URI",
                        "value": f"mongodb://{infra_host('user-timeline-mongo')}:27017",
                    },
                    {
                        "name": "USER_TIMELINE_REDIS_URL",
                        "value": f"redis://{infra_host('user-timeline-redis')}:6379",
                    },
                    {
                        "name": "POST_STORAGE_IP",
                        "value": svc_host("post-storage-service"),
                    },
                    {"name": "POST_STORAGE_PORT", "value": "8080"},
                    {"name": "POST_STORAGE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "home-timeline-service",
                "binary": "home_timeline_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "HOME_TIMELINE_REDIS_URL",
                        "value": f"redis://{infra_host('home-timeline-redis')}:6379",
                    },
                    {
                        "name": "POST_STORAGE_SERVICE_IP",
                        "value": svc_host("post-storage-service"),
                    },
                    {"name": "POST_STORAGE_SERVICE_PORT", "value": "8080"},
                    {"name": "POST_STORAGE_SERVICE_REPLICAS", "value": "1"},
                    {
                        "name": "SOCIAL_GRAPH_SERVICE_IP",
                        "value": svc_host("social-graph-service"),
                    },
                    {"name": "SOCIAL_GRAPH_SERVICE_PORT", "value": "8080"},
                    {"name": "SOCIAL_GRAPH_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                # write-home-timeline-service is a RabbitMQ queue consumer —
                # no inbound TCP listener, so it gets neither a Service nor a
                # readiness probe.
                "name": "write-home-timeline-service",
                "binary": "write_home_timeline_server",
                "replicas": 1,
                "port": 8080,
                "noListen": True,
                "env": [
                    {
                        "name": "RABBITMQ_URL",
                        "value": f"amqp://guest:guest@{infra_host('rabbitmq')}:5672",
                    },
                    {
                        "name": "REDIS_URL",
                        "value": f"redis://{infra_host('write-home-timeline-redis')}:6379",
                    },
                    {
                        "name": "SOCIAL_GRAPH_SERVICE_IP",
                        "value": svc_host("social-graph-service"),
                    },
                    {"name": "SOCIAL_GRAPH_SERVICE_PORT", "value": "8080"},
                    {"name": "SOCIAL_GRAPH_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "user-service",
                "binary": "user_service",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "MONGO_URL",
                        "value": f"mongodb://{infra_host('user-mongo')}:27017/user",
                    },
                    {
                        "name": "REDIS_URL",
                        "value": f"redis://{infra_host('user-redis')}:6379",
                    },
                    {"name": "JWT_SECRET", "value": jwt_secret},
                    {"name": "MACHINE_ID", "value": "02"},
                    {
                        "name": "SOCIAL_GRAPH_SERVICE_IP",
                        "value": svc_host("social-graph-service"),
                    },
                    {"name": "SOCIAL_GRAPH_SERVICE_PORT", "value": "8080"},
                    {"name": "SOCIAL_GRAPH_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "social-graph-service",
                "binary": "social_graph_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "MONGO_URL",
                        "value": f"mongodb://{infra_host('social-graph-mongo')}:27017/social-graph",
                    },
                    {
                        "name": "REDIS_URL",
                        "value": f"redis://{infra_host('social-graph-redis')}:6379",
                    },
                    {"name": "USER_SERVICE_IP", "value": svc_host("user-service")},
                    {"name": "USER_SERVICE_PORT", "value": "8080"},
                    {"name": "USER_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "unique-id-service",
                "binary": "unique_id_service",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {"name": "MACHINE_ID", "value": "01"},
                ],
            },
            {
                "name": "media-service",
                "binary": "media_service",
                "replicas": 1,
                "port": 8080,
                "env": [],
            },
            {
                "name": "text-service",
                "binary": "textservice_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "USER_MENTION_SERVICE_IP",
                        "value": svc_host("user-mention-service"),
                    },
                    {"name": "USER_MENTION_SERVICE_PORT", "value": "8080"},
                    {"name": "USER_MENTION_SERVICE_REPLICAS", "value": "1"},
                    {
                        "name": "URL_SHORTEN_SERVICE_IP",
                        "value": svc_host("url-shorten-service"),
                    },
                    {"name": "URL_SHORTEN_SERVICE_PORT", "value": "8080"},
                    {"name": "URL_SHORTEN_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "user-mention-service",
                "binary": "usermention_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "MONGO_URL",
                        "value": f"mongodb://{infra_host('user-mention-mongo')}:27017/user-mention",
                    },
                    {
                        "name": "MEMCACHED_URL",
                        "value": f"tcp://{infra_host('user-mention-memcached')}:11211",
                    },
                ],
            },
            {
                "name": "url-shorten-service",
                "binary": "url_shorten_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "MONGO_URL",
                        "value": f"mongodb://{infra_host('url-shorten-mongo')}:27017/url-shorten",
                    },
                    {"name": "RUST_LOG", "value": "tonic=debug,info"},
                ],
            },
            {
                "name": "register-user-service",
                "binary": "register_user_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {"name": "USER_SERVICE_IP", "value": svc_host("user-service")},
                    {"name": "USER_SERVICE_PORT", "value": "8080"},
                    {"name": "USER_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "compose-post-service",
                "binary": "compose_post_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "POST_STORAGE_IP",
                        "value": svc_host("post-storage-service"),
                    },
                    {"name": "POST_STORAGE_PORT", "value": "8080"},
                    {"name": "POST_STORAGE_REPLICAS", "value": "1"},
                    {
                        "name": "USER_TIMELINE_IP",
                        "value": svc_host("user-timeline-service"),
                    },
                    {"name": "USER_TIMELINE_PORT", "value": "8080"},
                    {"name": "USER_TIMELINE_REPLICAS", "value": "4"},
                    {
                        "name": "HOME_TIMELINE_IP",
                        "value": svc_host("home-timeline-service"),
                    },
                    {"name": "HOME_TIMELINE_PORT", "value": "8080"},
                    {"name": "HOME_TIMELINE_REPLICAS", "value": "1"},
                    {"name": "USER_SERVICE_IP", "value": svc_host("user-service")},
                    {"name": "USER_SERVICE_PORT", "value": "8080"},
                    {"name": "USER_SERVICE_REPLICAS", "value": "1"},
                    {"name": "MEDIA_SERVICE_IP", "value": svc_host("media-service")},
                    {"name": "MEDIA_SERVICE_PORT", "value": "8080"},
                    {"name": "MEDIA_SERVICE_REPLICAS", "value": "1"},
                    {"name": "TEXT_SERVICE_IP", "value": svc_host("text-service")},
                    {"name": "TEXT_SERVICE_PORT", "value": "8080"},
                    {"name": "TEXT_SERVICE_REPLICAS", "value": "1"},
                    {
                        "name": "USER_MENTION_SERVICE_IP",
                        "value": svc_host("user-mention-service"),
                    },
                    {"name": "USER_MENTION_SERVICE_PORT", "value": "8080"},
                    {"name": "USER_MENTION_SERVICE_REPLICAS", "value": "1"},
                    {
                        "name": "URL_SHORTEN_SERVICE_IP",
                        "value": svc_host("url-shorten-service"),
                    },
                    {"name": "URL_SHORTEN_SERVICE_PORT", "value": "8080"},
                    {"name": "URL_SHORTEN_SERVICE_REPLICAS", "value": "1"},
                    {
                        "name": "UNIQUE_ID_SERVICE_IP",
                        "value": svc_host("unique-id-service"),
                    },
                    {"name": "UNIQUE_ID_SERVICE_PORT", "value": "8080"},
                    {"name": "UNIQUE_ID_SERVICE_REPLICAS", "value": "1"},
                ],
            },
            {
                "name": "frontend-service",
                "binary": "frontend_server",
                "replicas": 1,
                "port": 8080,
                "env": [
                    {
                        "name": "COMPOSE_POST_SERVICE_IP",
                        "value": svc_host("compose-post-service"),
                    },
                    {"name": "COMPOSE_POST_SERVICE_PORT", "value": "8080"},
                    {"name": "COMPOSE_POST_SERVICE_REPLICAS", "value": "1"},
                    {
                        "name": "REGISTER_USER_SERVICE_IP",
                        "value": svc_host("register-user-service"),
                    },
                    {"name": "REGISTER_USER_SERVICE_PORT", "value": "8080"},
                    {"name": "REGISTER_USER_SERVICE_REPLICAS", "value": "1"},
                ],
            },
        ]

        # Stateful infra. Names match the docker-compose service names so
        # the env-var hostnames above resolve to the right Service.
        infra = [
            {"name": "post-storage-mongo", "image": "mongo:7.0", "port": 27017},
            {"name": "post-storage-memcached", "image": "memcached:1.6", "port": 11211},
            {"name": "user-timeline-mongo", "image": "mongo:7.0", "port": 27017},
            {"name": "user-timeline-redis", "image": "redis:7.2", "port": 6379},
            {"name": "home-timeline-redis", "image": "redis:7.2", "port": 6379},
            {"name": "write-home-timeline-redis", "image": "redis:7.2", "port": 6379},
            {"name": "user-mongo", "image": "mongo:7.0", "port": 27017},
            {"name": "user-redis", "image": "redis:7.2", "port": 6379},
            {"name": "social-graph-mongo", "image": "mongo:7.0", "port": 27017},
            {"name": "social-graph-redis", "image": "redis:7.2", "port": 6379},
            {"name": "user-mention-mongo", "image": "mongo:7.0", "port": 27017},
            {"name": "user-mention-memcached", "image": "memcached:1.6", "port": 11211},
            {"name": "url-shorten-mongo", "image": "mongo:7.0", "port": 27017},
            {"name": "rabbitmq", "image": "rabbitmq:3.13-management", "port": 5672},
        ]

        with policy_params_path.open() as f:
            policy_params_text = f.read()

        values = {
            "fullnameOverride": project_name,
            "image": {
                "repository": "",
                "tag": image_tag,
                "pullPolicy": "Never",
            },
            "logLevel": log_level,
            "service": {"type": "ClusterIP"},
            "services": services,
            "infra": infra,
            "configMaps": {
                "enabled": True,
                "appConfigJson": json.dumps(app_config, indent=2),
                "policyParamsJson": policy_params_text,
            },
        }

        values_path = output_dir / "values.yaml"
        with values_path.open("w") as f:
            yaml.dump(values, f, sort_keys=False)

        env_vars["HELM_VALUES_FILE"] = str(values_path.resolve())

    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> tuple[Path, str]:
        if use_k8s:
            return repo_root / "charts" / "socialnet", "."

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

        # In k8s mode pods attach to the cluster network — no docker network.
        if use_k8s:
            network = None
        else:
            network = (
                f"{project_name}_socialnet-network"
                if project_name
                else "socialnet-network"
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
        volumes.update(self._policy_params_loadgen_mount(output_dir))

        # In k8s, the pod must stay alive after the loadgen exits so ExpDriver
        # can `kubectl cp` the trace artifacts out — same trick as hotel.
        command = None
        if use_k8s:
            command = [
                "/bin/sh",
                "-c",
                "/usr/entrypoint.sh || exit $?; "
                "while true; do echo SOCIALNET_LOADGEN_DONE; sleep 10; done",
            ]

        return TaskSpec(
            name=f"{project_name}-loadgen" if project_name else "socialnet-loadgen",
            image=image,
            env_vars=task_env,
            network=network,
            volumes=volumes,
            cleanup=True,
            artifacts=[("/tmp/masa-load-gen/.", ".")],
            command=command,
            wait_for_log_pattern="SOCIALNET_LOADGEN_DONE" if use_k8s else None,
        )

    def get_required_images(self, features: Optional[str] = None) -> list[str]:
        tag = self.get_image_tag(features) or "latest"
        binaries = [
            "socialnet_client_bench",
            "frontend_server",
            "register_user_server",
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
        images = [f"{b}:{tag}" for b in binaries]
        # Stateful infra images — loaded into kind so tests work offline.
        images.extend(
            [
                "mongo:7.0",
                "redis:7.2",
                "memcached:1.6",
                "rabbitmq:3.13-management",
            ]
        )
        return images
