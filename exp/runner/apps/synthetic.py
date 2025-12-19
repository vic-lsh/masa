"""
Synthetic application plugin.
"""

import json
import re
from pathlib import Path
from typing import Optional

from .base import AppPlugin, DockerConfig


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
