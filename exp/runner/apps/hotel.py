"""
Hotel application plugin.
"""

import json
import logging
import re
import subprocess
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator

logger = logging.getLogger(__name__)


def normalize_features_to_tag(features: Optional[str]) -> str:
    """
    Normalize cargo feature flags into a deterministic, valid docker tag.
    
    Cargo features are comma-separated (e.g., "feat-b,feat-a").
    Docker tags must be lowercase alphanumeric with periods, dashes, or underscores.
    
    Args:
        features: Comma-separated cargo features or None
        
    Returns:
        A normalized docker tag string (e.g., "feat-a-feat-b" or "latest")
    """
    if not features or features.strip() == "":
        return "latest"
    
    # Split by comma, strip whitespace, and sort for determinism
    feature_list = [f.strip() for f in features.split(",")]
    feature_list = [f for f in feature_list if f]  # Remove empty strings
    
    if not feature_list:
        return "latest"
    
    # Sort for determinism (case-insensitive for consistency)
    feature_list.sort(key=str.lower)
    
    # Join with dashes, ensuring valid docker tag characters
    # Replace any invalid characters with dashes
    tag = "-".join(feature_list)
    
    # Docker tags: lowercase alphanumeric, periods, dashes, underscores only
    # Also ensure it doesn't start with a period or dash
    tag = re.sub(r'[^a-zA-Z0-9._-]', '-', tag)
    tag = tag.lower()
    tag = re.sub(r'^[.-]+', '', tag)  # Remove leading periods or dashes
    tag = re.sub(r'-+', '-', tag)  # Collapse multiple dashes
    
    return tag if tag else "latest"


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
        return f"hotel:{tag}"
    
    def get_binary_name(self) -> str:
        return "hotel_client_bench"


class HotelBuilder(AppBuilder):
    """
    Build logic for the hotel app docker image.

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
    ) -> None:
        app = "hotel"
        binaries = (
            "hotel_client_bench hotel_frontend hotel_geo hotel_rate hotel_review "
            "hotel_search hotel_profile hotel_reservation hotel_user "
            "hotel_recommendation loadgen"
        )

        if app_config_path is None:
            raise ValueError("app_config_path is required for hotel app")

        # Convert to path relative to repo_root
        config_path_rel = app_config_path.relative_to(repo_root)

        build_args: list[str] = []
        if features:
            build_args.extend(["--build-arg", f"FEATURES={features}"])
        build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
        build_args.extend(["--build-arg", f"APP={app}"])
        build_args.extend(["--build-arg", f"APP_CONFIG_PATH={config_path_rel}"])
        build_args.extend(["--build-arg", f"BINARIES={binaries}"])

        # Generate tag based on features for deterministic, feature-specific images
        tag = normalize_features_to_tag(features)
        image_name = f"hotel:{tag}"
        
        logger.info(f"Building docker image: {image_name}")

        cmd: list[str] = [
            "docker",
            "build",
            "-f",
            "./exp/common/docker-build/Dockerfile",
            *build_args,
            "--ulimit",
            "nofile=4096:4096",
        ]

        if no_cache:
            cmd.append("--no-cache")

        cmd.extend(["-t", image_name, "."])

        subprocess.run(
            cmd,
            cwd=repo_root,
            check=True,
            capture_output=False,
        )
        
        logger.info(f"Successfully built docker image: {image_name}")


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
            loadgen_image_name="hotel:<features>",  # Actual tag is dynamic based on features
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
