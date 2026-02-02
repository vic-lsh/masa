"""
Docker operations manager for building, running, and managing containers.
"""

import logging
import os
import re
import subprocess
import threading
from pathlib import Path

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
    ansi_escape = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
    return ansi_escape.sub('', text)


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
    
    def start(
        self,
        app_dir: Path,
        compose_file: str,
        env_vars: dict,
        project_name: str | None = None,
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
        compose_file: str,
        env_vars: dict | None = None,
        project_name: str | None = None,
    ) -> None:
        """
        Stop Docker Compose services.
        
        Args:
            app_dir: Path to application directory
            compose_file: Path to compose file relative to app_dir
        """
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
    
    def get_container_names(
        self,
        compose_path: Path,
        project_name: str,
        env_vars: dict | None = None,
    ) -> list[str]:
        """
        Get list of container names from a docker compose project.
        
        Includes both running and stopped containers to ensure logs are
        gathered even for containers that crash quickly.
        
        Args:
            compose_path: Path to docker-compose.yml file (absolute or relative)
            project_name: Docker compose project name
            env_vars: Optional environment variables
            
        Returns:
            List of container names
        """
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
            container_names = [name.strip() for name in result.stdout.strip().split("\n") if name.strip()]
            return container_names
        except subprocess.CalledProcessError as e:
            logger.warning(f"Failed to get container names: {e}")
            return []
    
    def check_project_health(
        self,
        compose_path: Path,
        project_name: str,
        env_vars: dict | None = None,
    ) -> list[tuple[str, int]]:
        """
        Check if any containers in the project have failed (exited with non-zero code).
        
        Args:
            compose_path: Path to docker-compose.yml file
            project_name: Docker compose project name
            env_vars: Optional environment variables
            
        Returns:
            List of (container_name, exit_code) for failed containers.
            Returns empty list if all containers are healthy (running or exited with 0).
        """
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
            "-a",
            "--format",
            "json",
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
            
            import json
            containers = []
            try:
                # Try parsing as a single JSON array (standard format)
                containers = json.loads(result.stdout)
            except json.JSONDecodeError:
                # Fallback: Try parsing as newline-delimited JSON (NDJSON)
                # Some versions/configurations output one JSON object per line
                try:
                    containers = [json.loads(line) for line in result.stdout.strip().split('\n') if line.strip()]
                except json.JSONDecodeError as e:
                    logger.warning(f"Failed to parse docker compose ps output: {e}\nOutput was: {result.stdout}")
                    return []
            
            failed_containers = []
            if isinstance(containers, list):
                for c in containers:
                    # Check for non-zero exit code
                    exit_code = c.get("ExitCode", 0)
                    state = c.get("State", "").lower()
                    name = c.get("Name", "unknown")
                    
                    # If it's exited with non-zero code, it's a failure.
                    # Also consider "restarting" (crash loop) and "dead" as failures.
                    if (state == "exited" and exit_code != 0) or state in ("restarting", "dead"):
                        failed_containers.append((name, exit_code))
                        
            return failed_containers
            
        except subprocess.CalledProcessError as e:
            logger.warning(f"Failed to check project health: {e}")
            return []

    def _stream_container_log(
        self,
        container_name: str,
        log_file: Path,
        follow: bool
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
                        for line in iter(process.stdout.readline, ''):
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
