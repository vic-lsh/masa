"""
MSSIM configuration generation logic.
"""

import subprocess
import sys
from pathlib import Path
from typing import Optional

from ..executor import CommandExecutor, SubprocessExecutor


class MssimConfigGenerator:
    """Helper to generate MSSIM docker-compose and deployment configs."""

    def __init__(self, executor: Optional[CommandExecutor] = None):
        self.executor = executor or SubprocessExecutor()

    def generate(
        self,
        *,
        app_dir: Path,
        output_dir: Path,
        gen_config: dict,
        app_config: dict,
        replicas_path: Optional[Path],
        env_vars: dict,
        dry_run: bool = False,
    ) -> tuple[Path, Path]:
        """
        Generate docker-compose.yml and deployment.json.

        Args:
            app_dir: Directory containing MSSIM app code (where simulator module is)
            output_dir: Directory to write generated files
            gen_config: Content of gen_config.json
            app_config: Content of mssim.json
            replicas_path: Path to replicas.json (optional)
            env_vars: Environment variables to pass to the generator
            dry_run: If True, only log what would happen

        Returns:
            Tuple of (docker_compose_path, deployment_json_path)
        """
        # Handle callgraph_dirs
        callgraph_dirs_raw = app_config.get("callgraph_dirs")
        if not callgraph_dirs_raw or not isinstance(callgraph_dirs_raw, list):
            raise ValueError(
                "mssim.json must include 'callgraph_dirs' (list of call graph directory paths)"
            )

        callgraph_dirs = [Path(d).expanduser().resolve() for d in callgraph_dirs_raw]
        for callgraph_dir in callgraph_dirs:
            if not callgraph_dir.exists():
                raise FileNotFoundError(
                    f"MSSIM callgraph_dir does not exist: {callgraph_dir}"
                )

        output_dir.mkdir(parents=True, exist_ok=True)
        docker_compose_path = output_dir / "docker-compose.yml"
        deployment_json_path = output_dir / "deployment.json"
        log_path = output_dir / "orchestrator.log"

        trace_cmd = [
            sys.executable,
            "-m",
            "simulator.main",
            "--docker-compose-output-path",
            str(docker_compose_path),
            "--deployment-output-path",
            str(deployment_json_path),
        ]

        for callgraph_dir in callgraph_dirs:
            trace_cmd.extend(["-a", str(callgraph_dir)])

        if replicas_path is not None:
            trace_cmd.extend(["--replicas-path", str(replicas_path)])

        if app_config.get("replay_path"):
            trace_cmd.extend(
                [
                    "--replay-path",
                    str(Path(app_config["replay_path"]).expanduser().resolve()),
                ]
            )

        if dry_run:
            print("[dry-run] compose generation:", " ".join(trace_cmd))
            return docker_compose_path, deployment_json_path

        # Run generation
        with log_path.open("wb") as log_file:
            gen_proc = self.executor.run(
                trace_cmd,
                cwd=app_dir,
                env=env_vars,
                stdout=log_file,
                stderr=subprocess.STDOUT,
            )

        if gen_proc.returncode != 0:
            raise RuntimeError(
                f"MSSIM config generation failed ({gen_proc.returncode}). See log at {log_path}"
            )

        return docker_compose_path, deployment_json_path
