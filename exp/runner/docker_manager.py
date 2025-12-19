"""
Docker operations manager for building, running, and managing containers.
"""

import logging
import os
import subprocess
import threading
from pathlib import Path
from typing import Optional

logger = logging.getLogger(__name__)


class DockerManager:
    """
    Manages all Docker operations for experiments.
    
    Handles building images, starting/stopping services, and collecting container logs.
    
    Note: Load generator execution is now handled by app-specific LoadGenerator classes.
    """
    
    def __init__(self, repo_root: Path):
        """
        Initialize DockerManager.
        
        Args:
            repo_root: Path to repository root
        """
        self.repo_root = repo_root
        self.common_scripts_dir = repo_root / "exp" / "common" / "scripts"
    
    def build(
        self,
        app_name: str,
        app_dir: Path,
        features: Optional[str] = None,
        rust_log: str = "info",
        no_cache: bool = False
    ) -> None:
        """
        Build Docker image for the application with specified features.
        
        Args:
            app_name: Name of the application
            app_dir: Path to application directory
            features: Cargo features to enable (e.g., scheduling policy)
            rust_log: Rust log level
            no_cache: Whether to disable Docker cache
            
        Raises:
            subprocess.CalledProcessError: If build fails
        """
        logger.info(f"Building Docker image for {app_name} with features: {features}")
        
        cmd = [
            str(self.common_scripts_dir / "docker-build.sh"),
            "--rust-log", rust_log,
        ]
        
        if features:
            cmd.extend(["--features", features])
        
        if no_cache:
            cmd.append("--no-cache")
        
        env = os.environ.copy()
        
        try:
            subprocess.run(
                cmd,
                cwd=app_dir,
                check=True,
                env=env,
                capture_output=False,
            )
            logger.info(f"Successfully built Docker image for {app_name}")
        except subprocess.CalledProcessError as e:
            logger.error(f"Docker build failed with exit code {e.returncode}")
            raise
    
    def start(
        self,
        app_dir: Path,
        compose_file: str,
        env_vars: dict
    ) -> None:
        """
        Start Docker Compose services with environment variables.
        
        Args:
            app_dir: Path to application directory
            compose_file: Path to compose file relative to app_dir
            env_vars: Environment variables for docker-compose
            
        Raises:
            subprocess.CalledProcessError: If start fails
        """
        compose_path = app_dir / compose_file
        logger.info(f"Starting Docker services from {compose_path}")
        
        env = os.environ.copy()
        env.update({k: str(v) for k, v in env_vars.items()})
        
        # First, ensure any existing services are stopped
        subprocess.run(
            ["docker", "compose", "-f", str(compose_path), "down"],
            cwd=app_dir,
            check=False,  # Don't fail if nothing to stop
            env=env,
            capture_output=True,
        )
        
        # Prune volumes
        subprocess.run(
            ["docker", "volume", "prune", "-a", "-f"],
            check=False,
            capture_output=True,
        )
        
        # Start services
        subprocess.run(
            ["docker", "compose", "-f", str(compose_path), "up", "-d"],
            cwd=app_dir,
            check=True,
            env=env,
        )
        
        logger.info("Docker services started successfully")
    
    def stop(
        self,
        app_dir: Path,
        compose_file: str
    ) -> None:
        """
        Stop Docker Compose services.
        
        Args:
            app_dir: Path to application directory
            compose_file: Path to compose file relative to app_dir
        """
        compose_path = app_dir / compose_file
        logger.info(f"Stopping Docker services from {compose_path}")
        
        subprocess.run(
            ["docker", "compose", "-f", str(compose_path), "down"],
            cwd=app_dir,
            check=False,  # Don't fail if already stopped
            capture_output=True,
        )
        
        logger.info("Docker services stopped")
    
    def stream_logs(
        self,
        container_names: list[str],
        output_dir: Path,
        follow: bool = True
    ) -> list[threading.Thread]:
        """
        Stream logs from containers to files.
        
        Args:
            container_names: List of container names to stream logs from
            output_dir: Directory to save log files
            follow: Whether to follow logs (blocking until container stops)
            
        Returns:
            List of threads streaming logs (if follow=True)
        """
        output_dir.mkdir(parents=True, exist_ok=True)
        threads = []
        
        for container_name in container_names:
            log_file = output_dir / f"{container_name}.log"
            
            if follow:
                # Start log streaming in background thread
                thread = threading.Thread(
                    target=self._stream_container_log,
                    args=(container_name, log_file, True),
                    daemon=True,
                )
                thread.start()
                threads.append(thread)
                logger.debug(f"Started log streaming for {container_name}")
            else:
                # Capture logs synchronously
                self._stream_container_log(container_name, log_file, False)
        
        return threads
    
    def _stream_container_log(
        self,
        container_name: str,
        log_file: Path,
        follow: bool
    ) -> None:
        """
        Stream a single container's logs to a file.
        
        Args:
            container_name: Name of container
            log_file: Path to output log file
            follow: Whether to follow logs
        """
        cmd = ["docker", "logs"]
        
        if follow:
            cmd.append("-f")
        
        cmd.append(container_name)
        
        try:
            with open(log_file, "w") as f:
                subprocess.run(
                    cmd,
                    stdout=f,
                    stderr=subprocess.STDOUT,
                    check=False,  # Container may exit before we stop following
                )
        except Exception as e:
            logger.warning(f"Error streaming logs from {container_name}: {e}")
