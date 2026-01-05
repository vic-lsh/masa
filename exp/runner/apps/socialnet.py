"""
Socialnet application plugin.
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
from .utils import normalize_features_to_tag, get_docker_progress_flag

logger = logging.getLogger(__name__)


class SocialnetLoadGenerator(LoadGenerator):
    """Load generator for the socialnet application."""
    
    def __init__(self, features: Optional[str] = None):
        """
        Initialize load generator with optional features for image tagging.
        
        Args:
            features: Cargo features used to build the image
        """
        self.features = features
    
    def get_container_name(self) -> str:
        return "socialnet_client_bench"
    
    def get_network_name(self) -> str:
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
        app_config_path: Optional[Path] = None,
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
        
        logger.info(f"Building {len(binaries_list)} docker images for socialnet app using multi-stage build")
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
        
        builder_cmd.append(str(repo_root))
        
        if dry_run:
            commands.append(builder_cmd)
            logger.info(f"[DRY RUN] Would run: {shlex.join(builder_cmd)}")
        else:
            logger.info(f"Running: {shlex.join(builder_cmd)}")
            result = subprocess.run(builder_cmd, cwd=repo_root, check=True)
            logger.info("Stage 1 complete")

        # Stage 2: Build runtime-base image (shared dependencies)
        logger.info("Stage 2: Building runtime-base image")
        runtime_base_cmd: list[str] = [
            "docker",
            "buildx",
            "build",
            "-f",
            "./exp/common/docker-build/Dockerfile",
            "--target",
            "runtime-base",
            *builder_build_args,
            get_docker_progress_flag(),
        ]
        
        if no_cache:
            runtime_base_cmd.append("--no-cache")
        
        runtime_base_cmd.append(str(repo_root))
        
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
            
            runtime_cmd: list[str] = [
                "docker",
                "buildx",
                "build",
                "-f",
                "./exp/common/docker-build/Dockerfile",
                "--target",
                "runtime",
                *builder_build_args,
                "--build-arg",
                f"BINARY={binary}",
                "-t",
                binary_tag,
                get_docker_progress_flag(),
            ]
            
            if no_cache:
                runtime_cmd.append("--no-cache")
            
            runtime_cmd.append(str(repo_root))
            
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
        self,
        gen_config: dict,
        app_config: Optional[dict],
        app_dir: Path
    ) -> dict:
        """
        Generate environment variables for socialnet application.
        
        Extracts compose_post port from gen_config and sets replica counts.
        """
        env_vars = {}
        
        # Extract port from address (e.g., "http://compose-post-service:8080" -> "8080")
        addr = gen_config.get("Addr", "")
        match = re.search(r':(\d+)$', addr)
        if match:
            env_vars["COMPOSE_POST_PORT"] = match.group(1)
        else:
            # Default port if not specified
            env_vars["COMPOSE_POST_PORT"] = "8080"
        
        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "info"
        
        return env_vars
    
    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for socialnet application."""
        return DockerConfig(
            compose_file="docker-compose.yaml",
            network_name="socialnet-network",
            loadgen_container_name="socialnet_client_bench",
            loadgen_image_name="socialnet_client_bench:<features>",
            loadgen_binary_name="socialnet_client_bench",
            app_config_filename=None,  # Socialnet doesn't use a separate app config file
        )
    
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for socialnet application.
        
        Includes compose_post and other service containers.
        """
        container_names = ["compose-post-service"]
        
        # Add other service containers (these match the docker-compose service names)
        # Note: Actual container names may have prefixes/suffixes based on docker-compose scaling
        services = [
            "post-storage-service",
            "user-timeline-service",
            "home-timeline-service",
            "user-service",
            "unique-id-service",
            "media-service",
            "text-service",
            "user-mention-service",
            "url-shorten-service",
            "social-graph-service",
            "write-home-timeline-service",
        ]
        
        container_names.extend(services)
        
        return container_names
    
    def create_load_generator(self, features: Optional[str] = None) -> LoadGenerator:
        """
        Create a load generator instance for socialnet application.
        
        Args:
            features: Optional cargo features used to build the image
        """
        return SocialnetLoadGenerator(features=features) if features is not None else SocialnetLoadGenerator()

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
        tag = normalize_features_to_tag(features)
        return f"socialnet_client_bench:{tag}" if tag != "latest" else "socialnet_client_bench:latest"
