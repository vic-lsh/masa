"""
Base classes and interfaces for application plugins.
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


@dataclass
class DockerConfig:
    """Configuration for Docker operations."""
    
    compose_file: str  # Path to docker-compose file relative to app directory
    network_name: str  # Docker network name
    loadgen_container_name: str  # Name for load generator container
    loadgen_image_name: str  # Docker image for load generator
    loadgen_binary_name: str  # Binary name to run in load generator
    
    # Optional app-specific config
    app_config_filename: Optional[str] = None
    app_config_required: bool = False


class AppPlugin(ABC):
    """
    Abstract base class for application-specific experiment behavior.
    
    Each application (hotel, synthetic) implements this interface to provide
    custom configuration parsing, environment variable generation, and
    container management.
    """
    
    @abstractmethod
    def get_app_name(self) -> str:
        """Return the application name (e.g., 'hotel', 'synthetic')."""
        pass
    
    @abstractmethod
    def load_app_config(self, config_path: Path) -> dict:
        """
        Load application-specific configuration file.
        
        Args:
            config_path: Path to the app config file
            
        Returns:
            Dictionary containing parsed configuration
            
        Raises:
            FileNotFoundError: If config file doesn't exist (when required)
        """
        pass
    
    @abstractmethod
    def generate_env_vars(
        self, 
        gen_config: dict, 
        app_config: Optional[dict],
        app_dir: Path
    ) -> dict:
        """
        Generate environment variables needed for docker-compose.
        
        Args:
            gen_config: Load generator configuration from gen_config.json
            app_config: Application-specific config (or None if not present)
            app_dir: Path to application directory
            
        Returns:
            Dictionary of environment variable name -> value
        """
        pass
    
    @abstractmethod
    def get_docker_config(self) -> DockerConfig:
        """
        Return Docker configuration for this application.
        
        Returns:
            DockerConfig with paths and names for Docker operations
        """
        pass
    
    @abstractmethod
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names to collect logs from.
        
        Args:
            env_vars: Environment variables generated for this run
            
        Returns:
            List of container names that should have logs collected
        """
        pass
