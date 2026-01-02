"""
MSSIM (microservice simulator) application plugin for the experiment runner.

MSSIM differs from the default runner flow:
- It generates a docker-compose.yml per (policy, rps, repeat) via `python -m simulator.main`
- It runs the workload via `docker compose up --abort-on-container-exit`
- It always cleans up with `docker compose down --volumes`
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import subprocess
import sys
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import normalize_features_to_tag


GENERIC_SERVICE_IMAGE = "generic_service"
MSSIM_LOADGEN_IMAGE = "mssim_load_generator"


def _canonicalize_features_for_build(feature: str) -> str:
    """
    Canonicalize features for use in build args (FEATURE_ARG).
    Returns comma-separated, sorted, deduplicated feature string.
    """
    parts = [part.strip() for part in re.split(r"[\s,]+", feature) if part.strip()]
    if not parts:
        return "default"
    return ",".join(sorted(set(parts)))


def _generic_service_image_for_policy(policy: str) -> str:
    """
    Get the docker image name for a given policy/features.
    Uses normalized feature flags for tagging like other apps in exp.runner.
    """
    tag = normalize_features_to_tag(policy)
    return f"{GENERIC_SERVICE_IMAGE}:{tag}"


def _safe_project_name(*, experiment_name: str, iteration: int, policy: str, rps: float) -> str:
    # docker compose project names should be simple; use a digest for uniqueness.
    raw = f"{experiment_name}|{iteration}|{policy}|{rps}"
    digest = hashlib.sha256(raw.encode("utf-8")).hexdigest()[:12]
    slug = re.sub(r"[^a-z0-9]+", "-", experiment_name.lower()).strip("-")[:24] or "exp"
    return f"mssim-{slug}-{digest}"


class MssimLoadGenerator(LoadGenerator):
    """
    Placeholder load generator.

    MSSIM runs the load generator as part of the generated docker-compose stack,
    so the default LoadGenerator flow is not used.
    """

    def get_container_name(self) -> str:
        return "mssim_load_generator"

    def get_network_name(self) -> str:
        return "mssim_network"

    def get_image_name(self) -> str:
        return MSSIM_LOADGEN_IMAGE

    def get_binary_name(self) -> str:
        return "mssim_load_generator"


class MssimBuilder(AppBuilder):
    """
    Build logic for MSSIM images.

    - Always builds mssim_load_generator once per runner invocation.
    - Builds a policy-specific generic service image (with deterministic tag) and also tags it
      as `generic_service` for compatibility.
    """

    def __init__(self) -> None:
        self._loadgen_built = False
        self._built_feature_keys: set[str] = set()

    def build(
        self,
        *,
        repo_root: Path,
        app_dir: Path,
        features: Optional[str] = None,
        rust_log: str = "info",
        no_cache: bool = False,
        app_config_path: Optional[Path] = None,
        gen_config_path: Optional[Path] = None,
        dry_run: bool = False,
    ) -> Optional[list[list[str]]]:
        # NOTE: rust_log/app_config_path/gen_config_path are unused for MSSIM builds today.
        commands: list[list[str]] = []

        dockerfile = repo_root / "apps" / "mssim" / "generic-service" / "Dockerfile"
        if not dockerfile.exists():
            raise FileNotFoundError(f"MSSIM dockerfile not found: {dockerfile}")

        # Build load generator image once
        if not self._loadgen_built:
            loadgen_cmd = [
                "docker",
                "buildx",
                "build",
                "--load",
                "-t",
                MSSIM_LOADGEN_IMAGE,
                "--target",
                "loadgen",
                "-f",
                str(dockerfile),
                str(repo_root),
            ]
            if no_cache:
                loadgen_cmd.insert(-1, "--no-cache")

            if dry_run:
                commands.append(loadgen_cmd)
            else:
                subprocess.run(loadgen_cmd, cwd=repo_root, check=True)

            self._loadgen_built = True

        # Build generic service image for policy/features
        policy = (features or "").strip() or "default"
        # Use normalized tag for image tagging (consistent with other apps)
        tag = normalize_features_to_tag(policy)
        # Canonicalize features for build arg (sort, deduplicate)
        features_for_build = _canonicalize_features_for_build(policy)
        
        if tag not in self._built_feature_keys:
            feature_image = _generic_service_image_for_policy(policy)

            generic_cmd = [
                "docker",
                "buildx",
                "build",
                "--load",
                "-t",
                feature_image,
                "--build-arg",
                f"FEATURE_ARG={features_for_build}",
                "--target",
                "generic-service",
                "-f",
                str(dockerfile),
                str(repo_root),
            ]
            if no_cache:
                generic_cmd.insert(-1, "--no-cache")

            tag_cmd = ["docker", "tag", feature_image, GENERIC_SERVICE_IMAGE]

            if dry_run:
                commands.append(generic_cmd)
                commands.append(tag_cmd)
            else:
                subprocess.run(generic_cmd, cwd=repo_root, check=True)
                subprocess.run(tag_cmd, cwd=repo_root, check=True)

            self._built_feature_keys.add(tag)

        return commands if dry_run else None


class MssimApp(AppPlugin):
    def __init__(self) -> None:
        self._builder = MssimBuilder()

    def get_app_name(self) -> str:
        return "mssim"

    def get_required_gen_config_fields(self) -> list[str]:
        # MSSIM needs repeats and RPS sweep values.
        return ["Repeats", "Rps"]

    def load_app_config(self, config_path: Path) -> dict:
        with open(config_path, "r", encoding="utf-8") as f:
            return json.load(f)

    def generate_env_vars(
        self,
        gen_config: dict,
        app_config: Optional[dict],
        app_dir: Path,
    ) -> dict:
        # MSSIM env vars are interpreted by the generated docker-compose stack.
        cfg = app_config or {}
        env: dict[str, str] = {}

        env["SLO_MS"] = str(int(cfg["slo_ms"]))
        env["ORCHESTRATOR"] = str(cfg.get("orchestrator", "localhost:50051"))

        duration = int(gen_config.get("DurationSecs", 0) or 0)
        warmup = int(gen_config.get("WarmupSecs", 0) or 0)
        if duration > 0:
            env["DURATION"] = str(duration)
        if warmup > 0:
            env["WARMUP_SEC"] = str(warmup)

        max_in_flight = cfg.get("max_in_flight", gen_config.get("MaxInFlight"))
        if max_in_flight is not None and int(max_in_flight) > 0:
            env["MAX_IN_FLIGHT"] = str(int(max_in_flight))

        stats_interval = cfg.get("stats_interval_sec")
        if stats_interval is not None and int(stats_interval) > 0:
            env["STATS_INTERVAL_SEC"] = str(int(stats_interval))

        replay_path = cfg.get("replay_path")
        if replay_path:
            env["REPLAY_TRACE_PATH"] = str(Path(replay_path).expanduser().resolve())

        extra_env = cfg.get("extra_env") or {}
        for k, v in extra_env.items():
            env[str(k)] = str(v)

        return env

    def get_docker_config(self) -> DockerConfig:
        # compose_file is unused for MSSIM (it is generated per run).
        return DockerConfig(
            compose_file="generated",
            network_name="mssim_network",
            loadgen_container_name="mssim_load_generator",
            loadgen_image_name=MSSIM_LOADGEN_IMAGE,
            loadgen_binary_name="mssim_load_generator",
            app_config_filename="mssim.json",
        )

    def get_container_names(self, env_vars: dict) -> list[str]:
        # MSSIM logs are captured via orchestrator.log rather than docker logs streaming.
        return []

    def create_load_generator(self, features: Optional[str] = None) -> LoadGenerator:
        return MssimLoadGenerator()

    def create_builder(self) -> AppBuilder:
        return self._builder

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
        # MSSIM-specific orchestration:
        # - build images (cached across calls)
        # - for each rps: generate compose + run docker compose up/down
        mssim_cfg = config.app_config or {}

        trace_dir = Path(mssim_cfg["trace_dir"]).expanduser().resolve()
        config_dir_raw = mssim_cfg.get("config_dir")
        config_dir = Path(config_dir_raw).expanduser().resolve() if config_dir_raw else None

        if not trace_dir.exists():
            raise FileNotFoundError(f"MSSIM trace_dir does not exist: {trace_dir}")
        if config_dir is not None and not config_dir.exists():
            raise FileNotFoundError(f"MSSIM config_dir does not exist: {config_dir}")

        rps_values = [float(v) for v in (config.gen_config.get("Rps") or [])]
        if not rps_values:
            raise ValueError("gen_config.json must include non-empty 'Rps' for MSSIM")

        duration_sec = int(config.gen_config.get("DurationSecs", 0) or 0)

        # Build images (dry-run prints build commands)
        builder = self.create_builder()
        build_cmds = builder.build(
            repo_root=repo_root,
            app_dir=config.app_dir,
            features=policy,
            rust_log="info",
            no_cache=no_cache,
            app_config_path=(config.in_dir / "mssim.json"),
            gen_config_path=(config.in_dir / "gen_config.json"),
            dry_run=dry_run,
        )
        if dry_run and build_cmds:
            print("\n".join(" ".join(cmd) for cmd in build_cmds))

        # Base environment variables for MSSIM stack
        base_env_vars = self.generate_env_vars(config.gen_config, config.app_config, config.app_dir)

        for rps in rps_values:
            run_dir = output_dir / f"rps_{rps:g}" / f"run_{iteration:01d}"
            docker_compose_path = run_dir / "docker-compose.yml"
            deployment_json_path = run_dir / "deployment.json"
            log_path = run_dir / "orchestrator.log"

            feature_image = _generic_service_image_for_policy(policy)

            env = os.environ.copy()
            env.update({k: str(v) for k, v in base_env_vars.items()})
            env["FEATURE"] = str(policy)
            env["RPS"] = f"{rps}"
            env["SLO_MS"] = str(int(mssim_cfg["slo_ms"]))
            env["GENERIC_SERVICE_IMAGE"] = feature_image
            env["HOST_TRACE_DIR"] = str(run_dir.resolve())

            project_name = _safe_project_name(
                experiment_name=config.experiment_name,
                iteration=iteration,
                policy=policy,
                rps=rps,
            )
            env["DOCKER_COMPOSE_PROJECT_NAME"] = project_name

            trace_cmd = [
                sys.executable,
                "-m",
                "simulator.main",
                "-a",
                str(trace_dir),
                "--docker-compose-output-path",
                str(docker_compose_path),
                "--deployment-output-path",
                str(deployment_json_path),
            ]
            if config_dir is not None:
                trace_cmd.extend(["-c", str(config_dir)])
            if mssim_cfg.get("replay_path"):
                trace_cmd.extend(["--replay-path", str(Path(mssim_cfg["replay_path"]).expanduser().resolve())])

            up_cmd = [
                "docker",
                "compose",
                "-f",
                str(docker_compose_path),
                "-p",
                project_name,
                "up",
                "--abort-on-container-exit",
            ]
            down_cmd = [
                "docker",
                "compose",
                "-f",
                str(docker_compose_path),
                "-p",
                project_name,
                "down",
                "--volumes",
            ]

            if dry_run:
                print(f"[dry-run] would run mssim policy={policy} rps={rps} iteration={iteration}")
                print(f"[dry-run] would write outputs under: {run_dir}")
                print("[dry-run] compose generation:", " ".join(trace_cmd))
                print("[dry-run] compose up:", " ".join(up_cmd))
                print("[dry-run] compose down:", " ".join(down_cmd))
                continue

            run_dir.mkdir(parents=True, exist_ok=True)
            metadata = {
                "app": "mssim",
                "experiment": config.experiment_name,
                "iteration": iteration,
                "policy": policy,
                "rps": rps,
                "duration_sec": duration_sec,
                "trace_dir": str(trace_dir),
                "config_dir": str(config_dir) if config_dir is not None else None,
                "generic_service_image": feature_image,
                "docker_project": project_name,
            }
            with (run_dir / "metadata.json").open("w", encoding="utf-8") as fh:
                json.dump(metadata, fh, indent=2, sort_keys=True)

            proc: subprocess.Popen | None = None
            try:
                # Generate compose/deployment
                with log_path.open("wb") as log_file:
                    gen_proc = subprocess.run(
                        trace_cmd,
                        cwd=config.app_dir,
                        env=env,
                        stdout=log_file,
                        stderr=subprocess.STDOUT,
                    )
                if gen_proc.returncode != 0:
                    raise RuntimeError(
                        f"MSSIM compose generation failed ({gen_proc.returncode}). See log at {log_path}"
                    )

                # Run the experiment stack
                print(f"Starting services for policy={policy} rps={rps} iteration={iteration}")
                with log_path.open("ab") as log_file:
                    proc = subprocess.Popen(
                        up_cmd,
                        cwd=config.app_dir,
                        env=env,
                        stdout=log_file,
                        stderr=subprocess.STDOUT,
                    )

                    grace_period = 300
                    timeout = duration_sec + grace_period if duration_sec > 0 else None
                    proc.wait(timeout=timeout)
            except subprocess.TimeoutExpired as exc:
                raise RuntimeError(
                    f"MSSIM timeout expired after {duration_sec} (+{grace_period}) seconds"
                ) from exc
            finally:
                # Always tear down compose stack, matching old behavior.
                if proc is not None and proc.poll() is None:
                    proc.send_signal(signal.SIGINT)
                    try:
                        proc.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        proc.kill()

                subprocess.run(down_cmd, cwd=config.app_dir, env=env, check=False)

