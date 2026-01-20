"""
Abstract base class for deployment managers (Docker, K8s).
"""

from abc import ABC, abstractmethod
from pathlib import Path
from typing import Optional
import threading


class DeploymentManager(ABC):
    """
    Abstract base class for managing application deployments.
    """

    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    @abstractmethod
    def start(
        self,
        app_dir: Path,
        config: dict,  # Generic config dict (compose file path for docker, values dict for k8s)
        env_vars: dict,
        project_name: Optional[str] = None,
    ) -> None:
        """Start the deployment."""
        pass

    @abstractmethod
    def stop(
        self,
        app_dir: Path,
        config: dict,
        env_vars: Optional[dict] = None,
        project_name: Optional[str] = None,
    ) -> None:
        """Stop the deployment."""
        pass

    @abstractmethod
    def stream_logs(
        self, container_names: list[str], output_dir: Path, follow: bool = True
    ) -> list[threading.Thread]:
        """Stream logs from containers/pods."""
        pass

    @abstractmethod
    def get_container_names(
        self,
        config: dict,
        project_name: str,
        env_vars: Optional[dict] = None,
    ) -> list[str]:
        """Get list of container/pod names."""
        pass
