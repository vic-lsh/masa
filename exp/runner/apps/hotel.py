"""
Hotel application plugin.
"""

import json
import logging
import shlex
import subprocess
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import normalize_features_to_tag

logger = logging.getLogger(__name__)


class HotelLoadGenerator(LoadGenerator):
    """Load generator for the hotel reservation application."""
    
    def __init__(self, features: Optional[str] = None):
        """
        Initialize load generator with optional features for image tagging.
        
        Args:
            features: Cargo features used to build the image
        """
        self.features = features
    
    def get_container_name(self) -> str:
        return "hotel_client_bench"
    
    def get_network_name(self) -> str:
        return "local_hotel_network"
    
    def get_image_name(self) -> str:
        tag = normalize_features_to_tag(self.features)
        if tag and tag != "latest":
            return f"hotel_hotel_client_bench:{tag}"
        else:
            return "hotel_hotel_client_bench:latest"
    
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
    - hotel_hotel_client_bench:latest - load generator
    - hotel_hotel_frontend:latest - frontend service
    - hotel_hotel_geo:latest - geo service
    - hotel_hotel_rate:latest - rate service
    - hotel_hotel_review:latest - review service
    - hotel_hotel_search:latest - search service
    - hotel_hotel_profile:latest - profile service
    - hotel_hotel_reservation:latest - reservation service
    - hotel_hotel_user:latest - user service
    - hotel_hotel_recommendation:latest - recommendation service
    - hotel_loadgen:latest - loadgen service

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
    ) -> Optional[list[list[str]]]:
        app = "hotel"
        # List of binaries to build (each gets its own image)
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
            "loadgen",
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
        
        logger.info(f"Building {len(binaries_list)} docker images for hotel app using multi-stage build")
        if features:
            logger.info(f"Using features: {features}")

        # Collect commands if dry_run
        commands: list[list[str]] = []

        # Stage 1: Build all binaries once (shared across all images)
        logger.info("Stage 1: Building all binaries for hotel app")
        builder_build_args: list[str] = []
        if features:
            builder_build_args.extend(["--build-arg", f"FEATURES={features}"])
        builder_build_args.extend(["--build-arg", f"APP={app}"])
        
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
            "--progress=plain",
        ]
        
        if no_cache:
            builder_cmd.append("--no-cache")
        
        builder_cmd.extend(["-t", f"{app}_builder:{tag}", "."])
        
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
                logger.error(f"Failed to build builder stage. Command: {shlex.join(builder_cmd)}")
                raise
        logger.info("Stage 1 complete: All binaries built")

        # Stage 2: Build runtime-base (shared across all images)
        logger.info("Stage 2: Building runtime-base image")
        runtime_base_build_args: list[str] = []
        if features:
            runtime_base_build_args.extend(["--build-arg", f"FEATURES={features}"])
        runtime_base_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
        runtime_base_build_args.extend(["--build-arg", f"APP={app}"])
        runtime_base_build_args.extend(["--build-arg", f"APP_CONFIG_PATH={config_path_rel}"])
        runtime_base_build_args.extend(["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"])
        
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
            "--progress=plain",
        ]
        
        if no_cache:
            runtime_base_cmd.append("--no-cache")
        
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
                logger.error(f"Failed to build runtime-base stage. Command: {shlex.join(runtime_base_cmd)}")
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
            runtime_build_args.extend(["--build-arg", f"APP_CONFIG_PATH={config_path_rel}"])
            runtime_build_args.extend(["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"])
            runtime_build_args.extend(["--build-arg", f"BINARY_NAME={binary_name}"])

            # Generate image name: hotel_<binary>:<tag> or hotel_<binary>:latest if no features
            if tag:
                image_name = f"hotel_{binary_name}:{tag}"
            else:
                image_name = f"hotel_{binary_name}:latest"

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
                "--progress=plain",
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
                    logger.error(f"Failed to build runtime image for {binary_name}. Command: {shlex.join(runtime_cmd)}")
                    raise
            
            logger.info(f"Successfully built docker image: {image_name}")
        
        if dry_run:
            return commands
        
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
        self,
        gen_config: dict,
        app_config: Optional[dict],
        app_dir: Path
    ) -> dict:
        """
        Generate environment variables for hotel application.
        
        Extracts frontend port from gen_config and replica counts from hotel.json.
        """
        env_vars = {}
        
        # Extract frontend port from address (e.g., "http://[::1]:8659" -> "8659")
        addr = gen_config.get("Addr", "")
        match = re.search(r':(\d+)$', addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")
        
        # Set replica counts from hotel.json
        default_replicas = 1
        services = [
            "rate", "profile", "reservation", "geo",
            "search", "user", "recommendation", "review"
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
            env_vars["LOG_LEVEL"] = "warn"
        
        return env_vars
    
    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for hotel application."""
        return DockerConfig(
            compose_file="scripts/local/containers+svcs.yaml",
            network_name="local_hotel_network",
            loadgen_container_name="hotel_client_bench",
            loadgen_image_name="hotel_hotel_client_bench:<features>",  # Actual tag is dynamic based on features
            loadgen_binary_name="hotel_client_bench",
            app_config_filename="hotel.json",
        )
    
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for hotel application.
        
        Includes frontend and replicated service containers based on
        replica counts from environment variables.
        """
        container_names = ["hotel_frontend"]
        
        # Services that can be replicated
        replicated_services = {
            "rate": int(env_vars.get("RATE_REPLICAS", 1)),
            "profile": int(env_vars.get("PROFILE_REPLICAS", 1)),
            "reservation": int(env_vars.get("RESERVATION_REPLICAS", 1)),
            "geo": int(env_vars.get("GEO_REPLICAS", 1)),
            "search": int(env_vars.get("SEARCH_REPLICAS", 1)),
            "user": int(env_vars.get("USER_REPLICAS", 1)),
        }
        
        # Generate container names for each replica
        for service, count in replicated_services.items():
            for i in range(1, count + 1):
                container_names.append(f"local-{service}-service-{i}")
        
        return container_names
    
    def create_load_generator(self, features: Optional[str] = None) -> LoadGenerator:
        """
        Create a load generator instance for hotel application.
        
        Args:
            features: Optional cargo features used to build the image
        """
        return HotelLoadGenerator(features=features)

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
