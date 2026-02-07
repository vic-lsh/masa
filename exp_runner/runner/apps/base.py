"""
Base classes and interfaces for application plugins.
"""

import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Optional, Tuple

from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor
from .utils import verify_standard_workload

logger = logging.getLogger(__name__)

if TYPE_CHECKING:  # pragma: no cover
    from exp_runner.runner.config import ExperimentConfig
    from exp_runner.runner.deployment_manager import DeploymentManager
    from exp_runner.runner.topology import TopologySpec
    from exp_runner.runner.experiment_config_v2 import ExperimentConfigV2


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

    NEW SIMPLIFIED INTERFACE (Phase 4):
    - Essential methods: get_app_name(), get_binaries(), get_frontend_name()
    - New hooks: get_default_topology_path(), customize_topology(), customize_env_vars()
    - Build integration: get_cargo_package(), get_build_parallelism()

    OLD INTERFACE (deprecated but maintained for backward compatibility):
    - prepare_workload(), get_deployment_location(), get_loadgen_spec()
    - load_app_config(), generate_env_vars(), get_docker_config(), get_container_names()
    """

    # ===== NEW SIMPLIFIED INTERFACE (Phase 4) =====

    @abstractmethod
    def get_app_name(self) -> str:
        """Return the application name (e.g., 'hotel', 'synthetic')."""
        pass

    @abstractmethod
    def get_binaries(self) -> list[str]:
        """
        Return list of binary names needed for this application.

        Used by BuildOrchestrator to build per-binary Docker images.

        Returns:
            List of binary names (e.g., ['frontend', 'geo_service', 'rate_service'])
        """
        pass

    @abstractmethod
    def get_frontend_name(self) -> str:
        """
        Return the name of the frontend service.

        This is the service that receives load from the load generator.

        Returns:
            Frontend service name (e.g., 'frontend', 'compose-post-service')
        """
        pass

    def get_cargo_package(self) -> str:
        """
        Return the cargo package name for this application.

        Defaults to the app name, override if different.

        Returns:
            Cargo package name (e.g., 'hotel', 'synthetic')
        """
        return self.get_app_name()

    def get_build_parallelism(self) -> int:
        """
        Return the degree of parallelism for building images.

        Defaults to 4 for parallel builds. Override for apps with few binaries.

        Returns:
            Number of parallel builds (1 = sequential, >1 = parallel)
        """
        return 4

    def get_default_topology_path(self, repo_root: Path) -> Optional[Path]:
        """
        Return path to default topology file, or None if topology is implicit.

        For real apps (hotel, socialnet): Usually None (topology is implicit in app code)
        For synthetic/mssim: Returns path to default topology YAML

        Args:
            repo_root: Repository root path

        Returns:
            Path to topology file, or None if implicit
        """
        # Default: check for apps/<app>/topology.yaml
        topology_path = repo_root / "apps" / self.get_app_name() / "topology.yaml"
        if topology_path.exists():
            return topology_path
        return None

    def customize_topology(self, topology: "TopologySpec") -> "TopologySpec":
        """
        Hook for app-specific topology transformations.

        Override this to modify topology before deployment generation.
        Default implementation returns topology unchanged.

        Args:
            topology: Loaded topology specification

        Returns:
            Modified topology specification
        """
        # Import here to avoid circular dependency
        if TYPE_CHECKING:
            pass
        return topology

    def customize_env_vars(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
        base_env: dict[str, str],
    ) -> dict[str, str]:
        """
        Hook for app-specific environment variable customization.

        Override this to add or modify environment variables beyond what
        the generators provide by default.

        Args:
            topology: Topology specification
            experiment: Experiment configuration
            base_env: Base environment variables from generator

        Returns:
            Modified environment variables dictionary
        """
        # Import here to avoid circular dependency
        if TYPE_CHECKING:
            pass
        return base_env

    def validate_experiment(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
    ) -> None:
        """
        Validate experiment configuration against topology.

        Override to add app-specific validation logic.
        Default implementation does nothing.

        Args:
            topology: Topology specification
            experiment: Experiment configuration

        Raises:
            ValueError: If validation fails
        """
        # Import here to avoid circular dependency
        if TYPE_CHECKING:
            pass
        pass

    # ===== LEGACY INTERFACE (backward compatibility) =====

    @abstractmethod
    def load_app_config(self, config_path: Path) -> dict:
        """
        Load application-specific configuration file.

        DEPRECATED: This method is part of the old interface and maintained for backward compatibility.

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

        DEPRECATED: This method is part of the old interface and maintained for backward compatibility.
        New code should use customize_env_vars() hook with generators.

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

        DEPRECATED: This method is part of the old interface and maintained for backward compatibility.

        Returns:
            DockerConfig with paths and names for Docker operations
        """
        pass

    @abstractmethod
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names to collect logs from.

        DEPRECATED: This method is part of the old interface and maintained for backward compatibility.

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

        DEPRECATED: This method is part of the old interface and maintained for backward compatibility.
        New code should use BuildOrchestrator directly with get_cargo_package() and get_binaries().

        Returns:
            AppBuilder instance configured for this application
        """
        pass

    @property
    def supports_k8s(self) -> bool:
        """
        Whether this application supports Kubernetes deployment.
        """
        return False

    def generate_k8s_values(
        self,
        config: "ExperimentConfig",
        policy: str,
        iteration: int,
        image_tag: str,
        app_config_path: Optional[Path],
        env_vars: dict,
    ) -> dict:
        """
        Generate Helm chart values for Kubernetes deployment.

        Args:
            config: Experiment configuration
            policy: Scheduling policy
            iteration: Iteration number
            image_tag: Docker image tag
            app_config_path: Path to app config file (optional)
            env_vars: Environment variables

        Returns:
            Dictionary of Helm values
        """
        return {}

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
        use_new_generator: bool = False,
    ) -> dict:
        """
        Prepare workload configuration and environment variables.

        DEPRECATED: This method is part of the old interface and will be removed in the future.
        New code should use the generator-based approach with customize_env_vars() hook.

        This method should:
        1. Calculate configuration values (pure logic)
        2. Generate configuration files (I/O)
        3. Return environment variables

        Args:
            config: Experiment configuration
            policy: Scheduling policy
            iteration: Iteration number
            output_dir: Directory for output artifacts
            repo_root: Repository root
            use_k8s: Whether targeting Kubernetes
            executor: Command executor for any subprocesses
            use_new_generator: Whether to use new topology-based generators
        """
        pass

    @abstractmethod
    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> Tuple[Path, str]:
        """
        Get deployment configuration location.

        DEPRECATED: This method is part of the old interface and will be removed in the future.
        New code should use generators which return GeneratedDeployment with these paths.

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

        DEPRECATED: This method is part of the old interface and will be removed in the future.
        New code should use DeploymentManager's built-in load generator task creation.

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
        use_new_generator: bool = False,
        **kwargs,
    ) -> None:
        """
        Run a single (iteration, policy) workload.

        Delegates to ExpDriver to orchestrate the experiment.
        """
        # Local import to avoid circular dependency
        from ..experiment_driver import ExpDriver

        # Track whether this run should use the new topology/generator pathway.
        self._using_new_generator = use_new_generator

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
            use_new_generator=use_new_generator,
        )

        # Clear flag after run to avoid leaking state across iterations.
        self._using_new_generator = False
