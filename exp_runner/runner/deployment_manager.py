"""
Abstract base class for deployment managers (Docker vs Kubernetes).
"""

import logging
import subprocess
import threading
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple

logger = logging.getLogger(__name__)


@dataclass
class TaskSpec:
    """Specification for a one-off task (e.g., load generator)."""

    name: str
    image: str
    env_vars: Dict[str, str] = field(default_factory=dict)
    network: Optional[str] = None
    command: Optional[List[str]] = None
    volumes: Dict[str, str] = field(default_factory=dict)  # host_path -> container_path
    cleanup: bool = False  # Whether to remove container after run
    artifacts: List[Tuple[str, str]] = field(
        default_factory=list
    )  # (src_in_container, dst_filename)


class DeploymentManager(ABC):
    """
    Abstract interface for managing application deployments.

    Standardizes operations across Docker Compose and Kubernetes.
    """

    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    @abstractmethod
    def run_task(self, task_spec: TaskSpec, log_file: Optional[Path] = None) -> None:
        """
        Run a one-off task (blocking).

        Args:
            task_spec: Specification of the task to run
            log_file: Optional path to save task logs/output
        """
        pass

    @abstractmethod
    def cleanup_task(self, task_spec: TaskSpec) -> None:
        """
        Cleanup resources associated with a task.

        Args:
            task_spec: Specification of the task to cleanup
        """
        pass

    @abstractmethod
    def start(
        self,
        app_dir: Path,
        deployment_config: str,
        env_vars: Dict[str, str],
        project_name: str,
    ) -> None:
        """
        Start services.

        Args:
            app_dir: Base directory for the application
            deployment_config: Configuration file/path relative to app_dir
                             (e.g., docker-compose.yml or helm chart dir)
            env_vars: Environment variables to inject
            project_name: Unique name for this deployment (required)
        """
        pass

    @abstractmethod
    def stop(
        self,
        app_dir: Path,
        deployment_config: str,
        env_vars: Optional[Dict[str, str]] = None,
        project_name: Optional[str] = None,
    ) -> None:
        """
        Stop services.

        Args:
            app_dir: Base directory for the application
            deployment_config: Configuration file/path relative to app_dir
            env_vars: Environment variables
            project_name: Unique name for this deployment
        """
        pass

    @abstractmethod
    def get_container_names(
        self,
        config_path: Path,
        project_name: str,
        env_vars: Optional[Dict[str, str]] = None,
    ) -> List[str]:
        """
        Get list of container/pod names.

        Args:
            config_path: Full path to configuration file
            project_name: Unique name for this deployment
            env_vars: Environment variables

        Returns:
            List of container/pod names
        """
        pass

    @abstractmethod
    def stream_logs(
        self,
        container_names: List[str],
        output_dir: Path,
        follow: bool = True,
    ) -> List[threading.Thread]:
        """
        Stream logs to files.

        Args:
            container_names: List of container/pod names
            output_dir: Directory to store log files
            follow: Whether to follow logs continuously

        Returns:
            List of threads handling the streaming
        """
        pass

    @abstractmethod
    def check_project_health(
        self,
        config_path: Path,
        project_name: str,
        env_vars: Optional[Dict[str, str]] = None,
    ) -> List[Tuple[str, int]]:
        """
        Check for failed containers/pods.

        Args:
            config_path: Full path to configuration file
            project_name: Unique name for this deployment
            env_vars: Environment variables

        Returns:
            List of (name, exit_code) for failed units
        """
        pass

    # Optional methods with default no-op or error implementation

    def load_image_to_cluster(self, cluster_name: str, image_names: List[str]) -> None:
        """
        Load images to cluster (only relevant for Kind/K8s).
        """
        pass

    def copy_from_container(
        self, container_name: str, src_path: str, dest_path: Path
    ) -> None:
        """
        Copy file from container/pod to local path.
        """
        raise NotImplementedError(
            "copy_from_container not implemented for this manager"
        )

    def port_forward(
        self, service_name: str, local_port: int, remote_port: int
    ) -> Optional[subprocess.Popen]:
        """
        Port forward service (only relevant for K8s).
        """
        return None
