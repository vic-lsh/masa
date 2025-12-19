"""
Synthetic application plugin.
"""

import json
import logging
import re
import subprocess
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator

logger = logging.getLogger(__name__)


class SyntheticLoadGenerator(LoadGenerator):
    """Load generator for the synthetic benchmark application."""
    
    def get_container_name(self) -> str:
        return "synthetic_client_bench"
    
    def get_network_name(self) -> str:
        return "local_synthetic_network"
    
    def get_image_name(self) -> str:
        return "synthetic_client_bench"
    
    def get_binary_name(self) -> str:
        return "synthetic_client_bench"


class SyntheticApp(AppPlugin):
    """
    Plugin for the synthetic benchmark application.
    
    The synthetic app has a simpler architecture with child services that can
    be configured with different replication strategies.
    """
    
    def get_app_name(self) -> str:
        return "synthetic"
    
    def load_app_config(self, config_path: Path) -> dict:
        """Load config.docker.json configuration file."""
        with open(config_path) as f:
            return json.load(f)
    
    def generate_env_vars(
        self,
        gen_config: dict,
        app_config: Optional[dict],
        app_dir: Path
    ) -> dict:
        """
        Generate environment variables for synthetic application.
        
        Calculates child replica counts from config and extracts frontend port.
        """
        env_vars = {}
        
        # Extract frontend port from address (e.g., "http://[::1]:8658" -> "8658")
        addr = gen_config.get("Addr", "")
        match = re.search(r':(\d+)$', addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")
        
        if app_config:
            # Calculate replica counts
            constant_replicas = app_config.get("child_constant_replicas", 1)
            env_vars["CONSTANT_REPLICAS"] = str(constant_replicas)
            
            # Calculate presampled replicas from child_presampled_services
            presampled_services = app_config.get("child_presampled_services", [])
            presampled_replicas = sum(
                service[0] for service in presampled_services if service
            ) if presampled_services else 0
            env_vars["PRESAMPLED_REPLICAS"] = str(presampled_replicas)
            
            # Random replicas is always 1
            random_replicas = 1
            env_vars["RANDOM_REPLICAS"] = str(random_replicas)
            
            # Total child replicas
            child_replicas = constant_replicas + random_replicas + presampled_replicas
            env_vars["CHILD_REPLICAS"] = str(child_replicas)
            
            # CPUs per replica
            cpus_per_replica = app_config.get("child_cpus_per_replica", 1)
            env_vars["CPUS_PER_REPLICA"] = str(cpus_per_replica)
        else:
            # Use defaults if no config provided
            env_vars["CONSTANT_REPLICAS"] = "1"
            env_vars["PRESAMPLED_REPLICAS"] = "0"
            env_vars["RANDOM_REPLICAS"] = "1"
            env_vars["CHILD_REPLICAS"] = "2"
            env_vars["CPUS_PER_REPLICA"] = "1"
        
        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "warn"
        
        return env_vars
    
    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for synthetic application."""
        return DockerConfig(
            compose_file="scripts/local/containers+svcs.yaml",
            network_name="local_synthetic_network",
            loadgen_container_name="synthetic_client_bench",
            loadgen_image_name="synthetic_client_bench",
            loadgen_binary_name="synthetic_client_bench",
            app_config_filename="config.docker.json",
            app_config_required=False,
        )
    
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for synthetic application.
        
        Includes frontend and child service containers based on replica count.
        """
        container_names = ["synthetic_frontend"]
        
        # Get child replica count
        child_replicas = int(env_vars.get("CHILD_REPLICAS", 1))
        
        # Generate container names for each child replica
        for i in range(1, child_replicas + 1):
            container_names.append(f"local-child-service-{i}")
        
        return container_names
    
    def create_load_generator(self, features: Optional[str] = None) -> LoadGenerator:
        """
        Create a load generator instance for synthetic application.
        
        Args:
            features: Optional cargo features (not used by synthetic app)
        """
        return SyntheticLoadGenerator()

    def create_builder(self) -> AppBuilder:
        """
        Create a builder instance for synthetic application.
        """
        return SyntheticBuilder()


class SyntheticBuilder(AppBuilder):
    """
    Build logic for the synthetic app docker images.
    
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
    ) -> None:
        app = "synthetic"
        
        # Services to build (each gets its own image)
        services = [
            ("synthetic_frontend", "synthetic_frontend:latest"),
            ("synthetic_child", "synthetic_child:latest"),
            ("synthetic_client_bench", "synthetic_client_bench:latest"),
        ]
        
        logger.info(f"Building {len(services)} docker images for synthetic app")
        if features:
            logger.info(f"Using features: {features}")
        
        for binary_name, image_name in services:
            logger.info(f"Building docker image: {image_name}")
            
            build_args: list[str] = []
            if features:
                build_args.extend(["--build-arg", f"FEATURES={features}"])
            build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
            build_args.extend(["--build-arg", f"APP={app}"])
            build_args.extend(["--build-arg", "APP_CONFIG_FILE=config.docker.json"])
            build_args.extend(["--build-arg", f"BINARIES={binary_name}"])
            
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
        
        logger.info("All synthetic app docker images built successfully")
