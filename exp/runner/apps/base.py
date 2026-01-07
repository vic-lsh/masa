"""
Base classes and interfaces for application plugins.
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Optional
import logging
import os
import shlex
import subprocess
import time


logger = logging.getLogger(__name__)

if TYPE_CHECKING:  # pragma: no cover
    from exp.runner.config import ExperimentConfig
    from exp.runner.docker_manager import DockerManager


@dataclass
class DockerConfig:
    """Configuration for Docker operations."""
    
    compose_file: str  # Path to docker-compose file relative to app directory
    network_name: str  # Docker network name
    loadgen_image_name: str  # Docker image for load generator
    loadgen_binary_name: str  # Binary name to run in load generator
    
    # Optional app-specific config
    app_config_filename: Optional[str] = None  # Filename for loading from exp/ directory


class LoadGenerator(ABC):
    """
    Abstract base class for application-specific load generator execution.
    
    Each application implements this interface to define how its load generator
    should be run, including container configuration, environment variables,
    and trace collection.
    """
    
    @abstractmethod
    def get_container_name(self) -> str:
        """Return the container name for the load generator."""
        pass
    
    @abstractmethod
    def get_network_name(self) -> str:
        """Return the Docker network name to connect to."""
        pass
    
    @abstractmethod
    def get_image_name(self) -> str:
        """Return the Docker image name to use for the load generator."""
        pass
    
    @abstractmethod
    def get_binary_name(self) -> str:
        """Return the binary name to execute inside the container."""
        pass
    
    def get_container_trace_path(self) -> str:
        """
        Return the path inside the container where traces are saved.
        
        Default is /tmp/masa-load-gen, can be overridden by subclasses.
        """
        return "/tmp/masa-load-gen"
    
    def get_env_vars(self, env_vars: Optional[dict] = None) -> dict:
        """
        Get environment variables to pass to the load generator container.
        
        Args:
            env_vars: Additional environment variables from experiment config
            
        Returns:
            Dictionary of environment variables
        """
        base_env = {
            "BINARY_NAME": self.get_binary_name(),
            "LOG_LEVEL": os.environ.get("LOG_LEVEL", "info"),
        }
        
        if env_vars:
            base_env.update(env_vars)
        
        return base_env
    
    def run(self, output_dir: Path, env_vars: Optional[dict] = None, gen_config_path: Optional[Path] = None) -> None:
        """
        Run the load generator and collect results.
        
        This method implements the core logic from loadgen-run.sh:
        1. Remove existing container if present
        2. Create output directory
        3. Run docker container with proper configuration
        4. Copy traces from container to output directory
        
        Args:
            output_dir: Directory to save output and traces
            env_vars: Additional environment variables for the container
            gen_config_path: Path to gen_config.json file to mount in container
            
        Raises:
            subprocess.CalledProcessError: If load generator execution fails
        """
        container_name = self.get_container_name()
        network_name = self.get_network_name()
        image_name = self.get_image_name()
        
        logger.info(f"Running load generator: {container_name}")
        
        # Remove existing container if present
        subprocess.run(
            ["docker", "rm", "-f", container_name],
            capture_output=True,
            check=False,
        )
        
        # Ensure output directory exists
        output_dir.mkdir(parents=True, exist_ok=True)
        
        # Build docker run command
        cmd = [
            "docker", "run",
            "--name", container_name,
            "--network", network_name,
        ]
        
        # Mount gen_config.json if provided (required for client bench binaries)
        if gen_config_path and gen_config_path.exists():
            cmd.extend(["-v", f"{gen_config_path}:/usr/gen_config.json:ro"])
            logger.debug(f"Mounting gen_config.json from {gen_config_path}")
        
        # Add environment variables
        load_env_vars = self.get_env_vars(env_vars)
        for key, value in load_env_vars.items():
            cmd.extend(["-e", f"{key}={value}"])
        
        cmd.append(image_name)
        
        # Run load generator and save logs
        loadgen_log = output_dir / "loadgen.log"
        logger.info(f"Load generator output will be saved to {loadgen_log}")
        
        with open(loadgen_log, "w") as f:
            try:
                subprocess.run(
                    cmd,
                    stdout=f,
                    stderr=subprocess.STDOUT,
                    check=True,
                )
            except subprocess.CalledProcessError as e:
                logger.error(f"Load generator failed. Command: {shlex.join(cmd)}")
                raise
        
        logger.info("Load generator container finished")
        
        # Copy traces from container
        self._copy_traces(container_name, output_dir)
        
        logger.info("Load generator completed successfully")
    
    def _copy_traces(self, container_name: str, output_dir: Path) -> None:
        """
        Copy traces from container to output directory.
        
        Implements the trace copying and flattening logic from loadgen-run.sh.
        
        Args:
            container_name: Name of the container to copy from
            output_dir: Directory to copy traces to
        """
        container_trace_path = self.get_container_trace_path()
        temp_subdir = output_dir / "masa-load-gen"
        
        try:
            # Copy traces from container
            copy_cmd = ["docker", "cp", f"{container_name}:{container_trace_path}", str(output_dir)]
            try:
                subprocess.run(
                    copy_cmd,
                    check=True,
                    capture_output=True,
                )
            except subprocess.CalledProcessError as e:
                logger.error(f"Failed to copy traces from container. Command: {shlex.join(copy_cmd)}")
                raise
            
            # Flatten the directory structure if needed
            if temp_subdir.exists() and temp_subdir.is_dir():
                logger.debug(f"Flattening traces from {temp_subdir}")
                for trace_file in temp_subdir.iterdir():
                    dest = output_dir / trace_file.name
                    trace_file.rename(dest)
                    logger.debug(f"Moved {trace_file.name} to {output_dir}")
                
                # Remove the now-empty subdirectory
                temp_subdir.rmdir()
            
            logger.info(f"Copied traces from container to {output_dir}")
            
        except subprocess.CalledProcessError as e:
            logger.warning(f"Failed to copy traces from container: {e}")
            logger.warning("Traces may not have been generated")


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
    
    @abstractmethod
    def create_load_generator(self, features: Optional[str] = None) -> LoadGenerator:
        """
        Create a load generator instance for this application.
        
        Args:
            features: Optional cargo features used to build the image
        
        Returns:
            LoadGenerator instance configured for this application
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

    def get_required_gen_config_fields(self) -> list[str]:
        """
        Return the list of required fields in gen_config.json for this application.

        The default runner apps (hotel/synthetic) expect an address to parse the frontend port.
        Apps with different orchestration (e.g., MSSIM) can override this.
        """
        return ["Repeats", "Addr"]

    def run_workload(
        self,
        *,
        repo_root: Path,
        config: "ExperimentConfig",
        docker: "DockerManager",
        policy: str,
        iteration: int,
        output_dir: Path,
        app_local_dir: Path,
        no_cache: bool,
        dry_run: bool = False,
    ) -> None:
        """
        Run a single (iteration, policy) workload.

        Default implementation matches the existing runner behavior:
        - generate env vars
        - build images
        - start docker compose services
        - stream logs
        - run load generator
        - stop docker compose services

        Apps with non-standard orchestration can override this method.
        """
        docker_config = self.get_docker_config()

        # Generate environment variables
        env_vars = self.generate_env_vars(
            config.gen_config,
            config.app_config,
            config.app_dir,
        )

        # Add image tag if app supports it (feature-specific images)
        if hasattr(self, "get_image_tag"):
            image_tag = getattr(self, "get_image_tag")(policy)
            env_vars[f"{config.app_name.upper()}_IMAGE_TAG"] = image_tag
            logger.debug(f"Set {config.app_name.upper()}_IMAGE_TAG={image_tag}")

        # Compute config paths for builds/loadgen
        app_config_path = None
        if docker_config.app_config_filename:
            candidate = config.in_dir / docker_config.app_config_filename
            if not candidate.exists():
                raise FileNotFoundError(f"App config not found at: {candidate}")
            app_config_path = candidate
            # Pass app config path to docker compose as env var for volume mounting
            env_vars["APP_CONFIG_PATH"] = str(app_config_path.resolve())

        gen_config_path = config.in_dir / "gen_config.json"
        if not gen_config_path.exists():
            raise FileNotFoundError(f"gen_config.json not found at: {gen_config_path}")

        # Build images
        builder = self.create_builder()
        if dry_run:
            commands = builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log="info",
                no_cache=no_cache,
                gen_config_path=gen_config_path,
                dry_run=True,
            )
            if commands:
                print("\n".join(shlex.join(cmd) for cmd in commands))
            print(f"[dry-run] would run {config.app_name} iteration={iteration} policy={policy}")
            return

        # Write .env file expected by compose setups (only when actually running)
        env_file = app_local_dir / ".env"
        env_file.parent.mkdir(parents=True, exist_ok=True)
        with open(env_file, "w", encoding="utf-8") as f:
            for key, value in env_vars.items():
                f.write(f"{key}={value}\n")
        logger.debug(f"Wrote environment variables to {env_file}")

        builder.build(
            repo_root=repo_root,
            app_dir=config.app_dir,
            features=policy,
            rust_log="info",
            no_cache=no_cache,
            gen_config_path=gen_config_path,
            dry_run=False,
        )

        try:
            docker.start(
                app_dir=config.app_dir,
                compose_file=docker_config.compose_file,
                env_vars=env_vars,
            )

            # Start streaming logs in background
            container_names = self.get_container_names(env_vars)
            docker.stream_logs(
                container_names=container_names,
                output_dir=output_dir,
                follow=True,
            )

            # Run load generator (blocking)
            loadgen = self.create_load_generator(features=policy)
            loadgen.run(
                output_dir=output_dir,
                env_vars=env_vars,
                gen_config_path=gen_config_path,
            )

            logger.info(f"Load generator completed for policy {policy}")

            # Wait a moment for logs to flush
            time.sleep(2)
        finally:
            # Stop Docker services
            docker.stop(
                app_dir=config.app_dir,
                compose_file=docker_config.compose_file,
                env_vars=env_vars,
            )

            # Clean up .env file
            if env_file.exists():
                env_file.unlink()
