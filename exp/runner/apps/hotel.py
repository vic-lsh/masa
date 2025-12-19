"""
Hotel application plugin.
"""

import json
import re
from pathlib import Path
from typing import Optional

from .base import AppPlugin, DockerConfig


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
            loadgen_image_name="hotel:latest",
            loadgen_binary_name="hotel_client_bench",
            app_config_filename="hotel.json",
            app_config_required=True,
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
