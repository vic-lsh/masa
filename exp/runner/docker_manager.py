"""
Docker operations manager for building, running, and managing containers.
"""

import logging
import os
import re
import subprocess
import threading
from pathlib import Path
from typing import Optional

from .deployment_manager import DeploymentManager

logger = logging.getLogger(__name__)


def strip_ansi_codes(text: str) -> str:
    """
    Remove ANSI escape sequences from text.

    This function strips color codes and other ANSI escape sequences
    to make log files more legible while preserving colors in terminal output.

    Args:
        text: Text that may contain ANSI escape sequences

    Returns:
        Text with ANSI escape sequences removed
    """
    # Pattern to match ANSI escape sequences
    # Matches: ESC[ followed by optional parameters and a command character
    ansi_escape = re.compile(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])")
    return ansi_escape.sub("", text)


class DockerManager(DeploymentManager):
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
        super().__init__(repo_root)
        self.common_scripts_dir = repo_root / "exp" / "common" / "scripts"

    def start(
        self,
        app_dir: Path,
        config: dict,
        env_vars: dict,
        project_name: str | None = None,
    ) -> None:
        """
        Start Docker Compose services with environment variables.

        Args:
            app_dir: Path to application directory
            config: Dict containing 'compose_file' key
            env_vars: Environment variables for docker-compose
            project_name: Optional project name

        Raises:
            subprocess.CalledProcessError: If start fails
        """
        compose_file = config.get("compose_file")
        if not compose_file:
            raise ValueError("Docker config must contain 'compose_file'")

        compose_path = app_dir / compose_file
        logger.info(f"Starting Docker services from {compose_path}")

        env = os.environ.copy()
        env.update({k: str(v) for k, v in env_vars.items()})

        # First, ensure any existing services are stopped
        base_cmd = ["docker", "compose", "-f", str(compose_path)]
        if project_name:
            base_cmd.extend(["-p", project_name])

        subprocess.run(
            [*base_cmd, "down"],
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
            [*base_cmd, "up", "-d"],
            cwd=app_dir,
            check=True,
            env=env,
        )

        logger.info("Docker services started successfully")

    def stop(
        self,
        app_dir: Path,
        config: dict,
        env_vars: dict | None = None,
        project_name: str | None = None,
    ) -> None:
        """
        Stop Docker Compose services.

        Args:
            app_dir: Path to application directory
            config: Dict containing 'compose_file' key
            env_vars: Optional environment variables
            project_name: Optional project name
        """
        compose_file = config.get("compose_file")
        if not compose_file:
            raise ValueError("Docker config must contain 'compose_file'")

        compose_path = app_dir / compose_file
        logger.info(f"Stopping Docker services from {compose_path}")

        env = os.environ.copy()
        if env_vars:
            env.update({k: str(v) for k, v in env_vars.items()})

        base_cmd = ["docker", "compose", "-f", str(compose_path)]
        if project_name:
            base_cmd.extend(["-p", project_name])

        subprocess.run(
            [*base_cmd, "down"],
            cwd=app_dir,
            check=False,  # Don't fail if already stopped
            env=env,
            capture_output=True,
        )

        logger.info("Docker services stopped")

    def stream_logs(
        self, container_names: list[str], output_dir: Path, follow: bool = True
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

    def get_container_names(
        self,
        config: dict,
        project_name: str,
        env_vars: dict | None = None,
    ) -> list[str]:
        """
        Get list of container names from a docker compose project.

        Includes both running and stopped containers to ensure logs are
        gathered even for containers that crash quickly.

        Args:
            config: Dict containing 'compose_file' key or 'compose_path' (Path object)
            project_name: Docker compose project name
            env_vars: Optional environment variables

        Returns:
            List of container names
        """
        # Handle both dict config and direct Path object (legacy/internal use)
        if isinstance(config, dict):
            compose_file = config.get("compose_file")
            # If we have app_dir available we could construct full path, but here we might need
            # the caller to provide the full path or relative path.
            # In existing usage, get_container_names was passed a full Path object as 'compose_path'.
            # We need to bridge this.
            # For now, let's assume if it's a dict, we might need a 'compose_path' key if it's absolute,
            # or we rely on the caller to have resolved it.
            # However, the previous signature was (compose_path: Path, ...).
            # The base class says (config: dict, ...).

            # Let's check how it's called in experiment.py.
            # It constructs compose_path = compose_app_dir / compose_file and passes it.
            # So we should probably expect the caller to pass the path in the config dict.
            if "compose_path" in config:
                compose_path = Path(config["compose_path"])
            elif "compose_file" in config:
                # This might be relative, which is risky if we don't have app_dir.
                # But let's look at where this is called.
                # In experiment.py:
                # compose_path = compose_app_dir / compose_file
                # docker.get_container_names(compose_path=compose_path...)
                # So we need to support passing the path.
                pass

        # To make this compatible with the base class but also workable for the implementation:
        if isinstance(config, dict) and "compose_path" in config:
            compose_path = Path(config["compose_path"])
        else:
            raise ValueError(
                "Docker config for get_container_names must contain 'compose_path'"
            )

        env = os.environ.copy()
        if env_vars:
            env.update({k: str(v) for k, v in env_vars.items()})

        cmd = [
            "docker",
            "compose",
            "-f",
            str(compose_path),
            "-p",
            project_name,
            "ps",
            "-a",  # Include stopped containers
            "--format",
            "{{.Name}}",
        ]

        try:
            result = subprocess.run(
                cmd,
                cwd=compose_path.parent,
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            container_names = [
                name.strip()
                for name in result.stdout.strip().split("\n")
                if name.strip()
            ]
            return container_names
        except subprocess.CalledProcessError as e:
            logger.warning(f"Failed to get container names: {e}")
            return []

    def _stream_container_log(
        self, container_name: str, log_file: Path, follow: bool
    ) -> None:
        """
        Stream a single container's logs to a file.

        ANSI color codes are stripped from logs written to files to make them
        more legible. Colors are preserved when running manually in terminal.

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
            if follow:
                # For following logs, read line by line and strip ANSI codes
                with open(log_file, "w", encoding="utf-8") as f:
                    process = subprocess.Popen(
                        cmd,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        text=True,
                        bufsize=1,  # Line buffered
                    )

                    try:
                        # Read line by line until process exits
                        if process.stdout:
                            for line in iter(process.stdout.readline, ""):
                                if line:
                                    # Strip ANSI codes before writing to file
                                    cleaned_line = strip_ansi_codes(line)
                                    f.write(cleaned_line)
                                    f.flush()  # Ensure immediate write
                    finally:
                        process.wait()
            else:
                # For non-following logs, capture all output then strip ANSI codes
                result = subprocess.run(
                    cmd,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    check=False,  # Container may exit before we stop following
                )

                with open(log_file, "w", encoding="utf-8") as f:
                    # Strip ANSI codes before writing to file
                    cleaned_output = strip_ansi_codes(result.stdout)
                    f.write(cleaned_output)
        except Exception as e:
            logger.warning(f"Error streaming logs from {container_name}: {e}")
