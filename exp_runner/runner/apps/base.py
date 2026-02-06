"""
Base classes and interfaces for application plugins.
"""

import logging
import os
import shlex
import subprocess
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Optional, Tuple

from ..cpu_monitor import CPUMonitor
from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor, SubprocessExecutor
from .utils import verify_standard_workload

logger = logging.getLogger(__name__)

if TYPE_CHECKING:  # pragma: no cover
    from exp_runner.runner.config import ExperimentConfig
    from exp_runner.runner.deployment_manager import DeploymentManager


@dataclass
class DockerConfig:
    """Configuration for Docker operations."""

    compose_file: str  # Path to docker-compose file relative to app directory
    network_name: str  # Docker network name
    loadgen_image_name: str  # Docker image for load generator
    loadgen_binary_name: str  # Binary name to run in load generator

    # Optional app-specific config
    app_config_filename: Optional[str] = (
        None  # Filename for loading from exp/ directory
    )
    app_config_filename: Optional[str] = (
        None  # Filename for loading from exp/ directory
    )


class AppBuilder(ABC):
    """
    Abstract base class for application-specific build logic.

    We keep this separate from Docker compose start/stop so each app can decide
    how images are built (single image, multiple images, extra build args, etc.).
    """

    @abstractmethod
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
        executor: Optional[CommandExecutor] = None,
    ) -> Optional[list[list[str]]]:
        """
        Build the app's docker images.

        Args:
            repo_root: Path to repository root
            app_dir: Path to application directory (e.g. <repo>/apps/<app>)
            features: Cargo features to enable (e.g. scheduling policy)
            rust_log: Rust log level to pass into image
            no_cache: Whether to disable Docker cache
            gen_config_path: Path to gen_config.json file (relative to repo_root) to include in image
            dry_run: If True, return list of commands instead of executing them
            executor: Command executor to use (defaults to SubprocessExecutor)

        Returns:
            If dry_run is True, returns a list of commands (each command is a list of strings).
            If dry_run is False, returns None after executing the commands.
        """
        raise NotImplementedError


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
        self, gen_config: dict, app_config: Optional[dict], app_dir: Path
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

    @abstractmethod
    def create_builder(self) -> AppBuilder:
        """
        Create an app builder for this application.

        Returns:
            AppBuilder instance configured for this application
        """
        pass

    @abstractmethod
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

        Args:
            config: Experiment configuration
            policy: Scheduling policy
            iteration: Iteration number
            output_dir: Directory for output artifacts
            repo_root: Repository root
            use_k8s: Whether targeting Kubernetes
            executor: Command executor for any subprocesses
        """
        pass

    @abstractmethod
    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> Tuple[Path, str]:
        """
        Get deployment configuration location.

        Args:
            output_dir: Output directory (where generated configs might reside)
            use_k8s: Whether targeting Kubernetes
            repo_root: Repository root

        Returns:
            Tuple of (deploy_root, deploy_file)
            deploy_root: Base directory for deployment command
            deploy_file: Config filename (compose file or chart directory) relative to deploy_root
        """
        pass

    @abstractmethod
    def get_loadgen_spec(
        self,
        output_dir: Path,
        features: Optional[str],
        env_vars: dict,
        use_k8s: bool,
    ) -> TaskSpec:
        """
        Get specification for the load generator task.

        Args:
            output_dir: Output directory
            features: Cargo features (policy)
            env_vars: Environment variables
            use_k8s: Whether targeting Kubernetes

        Returns:
            TaskSpec for the load generator
        """
        pass

    def get_required_images(self, features: Optional[str] = None) -> list[str]:
        """
        Get list of docker images required for this application.

        These images need to be available (or loaded into Kind) for the application to run.

        Args:
            features: Optional cargo features used to build the image

        Returns:
            List of image names (including tags)
        """
        return []

    def get_required_gen_config_fields(self) -> list[str]:
        """
        Return the list of required fields in gen_config.json for this application.

        The default runner apps (hotel/synthetic) expect an address to parse the frontend port.
        Apps with different orchestration (e.g., MSSIM) can override this.
        """
        return ["Repeats", "Addr"]

    def verify_results(self, config: "ExperimentConfig") -> bool:
        """
        Verify the results of an experiment.

        Default implementation uses the standard workload verification (checking goodput/files).
        Subclasses can override this to implement custom verification logic.

        Args:
            config: Experiment configuration object

        Returns:
            True if verification passed, False otherwise
        """
        return verify_standard_workload(config)

    def run_workload(
        self,
        *,
        repo_root: Path,
        config: "ExperimentConfig",
        deployment: "DeploymentManager",
        policy: str,
        iteration: int,
        output_dir: Path,
        app_local_dir: Path,
        no_cache: bool,
        dry_run: bool = False,
        executor: Optional[CommandExecutor] = None,
        **kwargs,
    ) -> None:
        """
        Run a single (iteration, policy) workload.

        Delegates to ExpDriver to orchestrate the experiment.
        """
        # Local import to avoid circular dependency
        from ..experiment_driver import ExpDriver

        # Allow deployment to be anything compatible
        driver = ExpDriver(
            self, deployment, executor=executor or getattr(self, "executor", None)
        )
        driver.run_workload(
            config=config,
            policy=policy,
            iteration=iteration,
            output_dir=output_dir,
            repo_root=repo_root,
            no_cache=no_cache,
            dry_run=dry_run,
        )
