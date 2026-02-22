"""
MSSIM (microservice simulator) application plugin for the experiment runner.

MSSIM differs from the default runner flow:
- It generates a docker-compose.yml per (policy, rps, repeat) via `python -m simulator.main`
- It runs the workload via `docker compose up --abort-on-container-exit`
- It always cleans up with `docker compose down --volumes`
"""

from __future__ import annotations

import csv
import hashlib
import json
import logging
import os
import re
from pathlib import Path
from typing import TYPE_CHECKING, Optional

if TYPE_CHECKING:
    from ..config import ExperimentConfig

import time
import yaml

from ..cpu_monitor import CPUMonitor  # noqa: F401
from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor, MockCommandExecutor, SubprocessExecutor
from ..naming import generate_project_name
from .base import AppBuilder, AppPlugin, DockerConfig
from .mssim_config import MssimConfigGenerator
from .utils import normalize_features_to_tag

logger = logging.getLogger(__name__)

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
    Uses normalized feature flags for tagging like other apps in exp_runner.runner.
    """
    tag = normalize_features_to_tag(policy)
    return f"{GENERIC_SERVICE_IMAGE}:{tag}"


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
        gen_config_path: Optional[Path] = None,
        dry_run: bool = False,
        executor: Optional[CommandExecutor] = None,
        **kwargs,
    ) -> Optional[list[list[str]]]:
        # NOTE: rust_log/app_config_path/gen_config_path are unused for MSSIM builds today.
        executor = executor or SubprocessExecutor()

        # Start timing the docker build
        build_start_time = time.time()

        dockerfile = repo_root / "apps" / "mssim" / "generic-service" / "Dockerfile"
        if not dockerfile.exists():
            raise FileNotFoundError(f"MSSIM dockerfile not found: {dockerfile}")

        # Build load generator image once
        if not self._loadgen_built:
            # Use a unique cache ID to avoid race conditions in parallel builds
            # Use "mssim-loadgen" as a consistent cache ID for the loadgen build
            loadgen_cache_id = "mssim-loadgen"
            loadgen_cmd = [
                "docker",
                "buildx",
                "build",
                "--load",
                "-t",
                MSSIM_LOADGEN_IMAGE,
                "--target",
                "loadgen",
                "--build-arg",
                f"CACHE_ID={loadgen_cache_id}",
                "-f",
                str(dockerfile),
                str(repo_root),
            ]
            if no_cache:
                loadgen_cmd.insert(-1, "--no-cache")

            executor.run(loadgen_cmd, cwd=repo_root, check=True)
            self._loadgen_built = True

        # Build generic service image for policy/features
        policy = (features or "").strip() or "default"
        # Use normalized tag for image tagging (consistent with other apps)
        tag = normalize_features_to_tag(policy)
        # Canonicalize features for build arg (sort, deduplicate)
        features_for_build = _canonicalize_features_for_build(policy)

        if tag not in self._built_feature_keys:
            feature_image = _generic_service_image_for_policy(policy)
            # Use a unique cache ID based on features to avoid race conditions in parallel builds
            # Format: mssim-{tag} to match the pattern used by other apps
            cache_id = f"mssim-{tag}"

            generic_cmd = [
                "docker",
                "buildx",
                "build",
                "--load",
                "-t",
                feature_image,
                "--build-arg",
                f"FEATURE_ARG={features_for_build}",
                "--build-arg",
                f"CACHE_ID={cache_id}",
                "--target",
                "generic-service",
                "-f",
                str(dockerfile),
                str(repo_root),
            ]
            if no_cache:
                generic_cmd.insert(-1, "--no-cache")

            tag_cmd = ["docker", "tag", feature_image, GENERIC_SERVICE_IMAGE]

            executor.run(generic_cmd, cwd=repo_root, check=True)
            executor.run(tag_cmd, cwd=repo_root, check=True)

            self._built_feature_keys.add(tag)

        if not dry_run:
            # Calculate and print build duration
            build_duration = time.time() - build_start_time
            logger.info(
                f"Docker image building took {build_duration:.2f} seconds ({build_duration / 60:.2f} minutes)"
            )

        if dry_run and isinstance(executor, MockCommandExecutor):
            return [cmd.args for cmd in executor.history]
        return None


class MssimApp(AppPlugin):
    def __init__(self, executor: Optional[CommandExecutor] = None) -> None:
        self._builder = MssimBuilder()
        self.executor = executor

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

        max_in_flight = gen_config.get("MaxInFlight")
        if max_in_flight is not None:
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
            compose_file="docker-compose.yml",  # Changed from 'generated' to generic, though prepare_workload sets it
            network_name="mssim_network",
            loadgen_image_name=MSSIM_LOADGEN_IMAGE,
            loadgen_binary_name="mssim_load_generator",
            app_config_filename="mssim.json",
        )

    def get_container_names(self, env_vars: dict) -> list[str]:
        # ExpDriver calls DeploymentManager.get_container_names which uses docker ps/inspect
        # So we usually don't need to return hardcoded names unless we want to filter.
        # But AppPlugin expects this to return something if monitoring needs hints?
        # Default implementation returns empty list, let's keep it empty as monitoring discovers containers.
        return []

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
        Prepare workload configuration using MssimConfigGenerator.
        """
        mssim_cfg = config.app_config or {}

        # Look for replicas.json
        replicas_path = config.in_dir / "replicas.json"
        if not replicas_path.exists():
            replicas_path = None

        rps_values = [float(v) for v in (config.gen_config.get("Rps") or [])]
        if not rps_values:
            raise ValueError("gen_config.json must include non-empty 'Rps' for MSSIM")

        # Base environment variables
        base_env_vars = self.generate_env_vars(
            config.gen_config, config.app_config, config.app_dir
        )

        feature_image = _generic_service_image_for_policy(policy)

        env = os.environ.copy()
        env.update({k: str(v) for k, v in base_env_vars.items()})
        env["FEATURE"] = str(policy)
        env["RPS_VALUES"] = json.dumps(rps_values)
        env["SLO_MS"] = str(int(mssim_cfg["slo_ms"]))
        env["GENERIC_SERVICE_IMAGE"] = feature_image
        env["HOST_TRACE_DIR"] = str(output_dir.resolve())

        # SKIP LOADGEN in compose generation, ExpDriver will run it as a task
        env["MSSIM_SKIP_LOADGEN"] = "1"

        project_name = generate_project_name(
            prefix="mssim",
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
            extra_suffix=str(0.0),  # rps placeholder for compatibility
        )
        env["DOCKER_COMPOSE_PROJECT_NAME"] = project_name

        # Generate Configs
        config_gen = MssimConfigGenerator(executor=executor or self.executor)

        # We need output_dir for run-specific artifacts
        # ExpDriver passes a run-specific output_dir (e.g. out/0/policy/run_0)
        # Verify output_dir structure in ExpDriver call.

        docker_compose_path, deployment_json_path = config_gen.generate(
            app_dir=config.app_dir,
            output_dir=output_dir,
            gen_config=config.gen_config,
            app_config=mssim_cfg,
            replicas_path=replicas_path,
            env_vars=env,
        )

        # Write metadata.json (Required for verification)
        # Note: callgraph_dirs logic is duplicated inside config gen, but we need it here for metadata
        callgraph_dirs_raw = mssim_cfg.get("callgraph_dirs", [])
        callgraph_dirs = [
            str(Path(d).expanduser().resolve()) for d in callgraph_dirs_raw
        ]

        metadata = {
            "app": "mssim",
            "experiment": config.experiment_name,
            "iteration": iteration,
            "policy": policy,
            "rps_values": rps_values,
            "duration_sec": int(config.gen_config.get("DurationSecs", 0) or 0),
            "callgraph_dirs": callgraph_dirs,
            "replicas_path": str(replicas_path) if replicas_path is not None else None,
            "generic_service_image": feature_image,
            "docker_project": project_name,
            "mode": "k8s" if use_k8s else "docker",
        }
        with (output_dir / "metadata.json").open("w", encoding="utf-8") as fh:
            json.dump(metadata, fh, indent=2, sort_keys=True)

        if use_k8s:
            return self._prepare_k8s_workload(
                output_dir,
                deployment_json_path,
                project_name,
                policy,
                mssim_cfg,
                env,
                repo_root,
            )

        return env

    def _prepare_k8s_workload(
        self,
        output_dir: Path,
        deployment_json_path: Path,
        project_name: str,
        policy: str,
        mssim_cfg: dict,
        env: dict,
        repo_root: Path,
    ) -> dict:
        """
        Prepare K8s-specific configuration (values.yaml with inline ConfigMaps).
        """

        def _sanitize_callgraph_name(raw: str, existing: set[str]) -> str:
            base = re.sub(r"[^a-z0-9]+", "-", raw.lower()).strip("-")
            if not base:
                base = "graph"
            base = base[:40]  # leave room for suffixes
            candidate = base
            idx = 1
            while candidate in existing:
                candidate = f"{base}-{idx}"
                idx += 1
            existing.add(candidate)
            return candidate

        # Modify deployment.json and frontend.json to set replicas=0 (K8s mode)
        with open(deployment_json_path) as f:
            deploy_data = json.load(f)

        services_values = []
        for svc_name, svc_info in deploy_data.get("services", {}).items():
            if svc_name == "USER" or svc_name.startswith("USER-"):
                env_svc_name = "USER"
            else:
                env_svc_name = svc_name

            svc_env = {
                "SERVICE_NAME": env_svc_name,
                "SERVICE_PORT": str(svc_info["port"]),
                "DEPLOYMENT_CONFIG_PATH": "/app/config/deployment.json",
                "FEATURE": str(policy),
            }

            services_values.append(
                {
                    "name": svc_name,
                    "replicas": svc_info["replicas"],
                    "env": svc_env,
                }
            )

        # Set replicas to 1 for client handling
        for svc_name in deploy_data.get("services", {}):
            deploy_data["services"][svc_name]["replicas"] = 1

        with open(deployment_json_path, "w") as f:
            json.dump(deploy_data, f, indent=2)

        frontend_json_path = output_dir / "frontend.json"
        if frontend_json_path.exists():
            with open(frontend_json_path) as f:
                frontend_data = json.load(f)
            for target in frontend_data:
                target["replicas"] = 1
            with open(frontend_json_path, "w") as f:
                json.dump(frontend_data, f, indent=2)

        # Prepare Inline ConfigMaps
        callgraph_dirs = [
            Path(d).expanduser().resolve() for d in mssim_cfg.get("callgraph_dirs", [])
        ]

        if not callgraph_dirs:
            raise ValueError("mssim.json must provide at least one callgraph directory")

        values = {
            "fullnameOverride": project_name,
            "image": {
                "repository": "",
                "genericServiceName": "generic_service",
                "tag": normalize_features_to_tag(policy),
                "pullPolicy": "Never",
            },
            # Map to the generated deployment config map name
            "deploymentConfigMap": f"{project_name}-deployment-config",
            "callgraphs": [],  # Populated below
            "services": services_values,
            "logLevel": "info",
            "configMaps": {
                "enabled": True,
                # Read deployment.json content
                "deploymentJson": deployment_json_path.read_text(),
                "callgraphs": [],
            },
        }

        sanitized_names: set[str] = set()
        for cg_dir in callgraph_dirs:
            if not cg_dir.exists() or not cg_dir.is_dir():
                raise FileNotFoundError(f"MSSIM call graph directory missing: {cg_dir}")

            files: dict[str, str] = {}
            for file_path in sorted(cg_dir.iterdir()):
                if file_path.is_file() and file_path.suffix.lower() in {
                    ".json",
                    ".csv",
                }:
                    files[file_path.name] = file_path.read_text(encoding="utf-8")

            if not files:
                raise ValueError(
                    f"No JSON/CSV files found in call graph directory {cg_dir}"
                )

            cg_name = _sanitize_callgraph_name(cg_dir.name, sanitized_names)
            values["configMaps"]["callgraphs"].append(
                {
                    "name": cg_name,
                    "files": files,
                }
            )
            values["callgraphs"].append(
                {
                    "name": cg_name,
                    "mountPath": cg_dir.name,
                    "configMapName": f"{project_name}-callgraph-{cg_name}",
                }
            )

        values_path = output_dir / "values.yaml"
        with open(values_path, "w") as f:
            yaml.dump(values, f)

        env["HELM_VALUES_FILE"] = str(values_path)
        return env

    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> tuple[Path, str]:
        if use_k8s:
            return repo_root / "charts" / "mssim", "."
        return output_dir, "docker-compose.yml"

    def get_loadgen_spec(
        self,
        output_dir: Path,
        features: Optional[str],
        env_vars: dict,
        use_k8s: bool,
    ) -> TaskSpec:
        # Construct network name if Docker
        network = None
        if not use_k8s:
            project_name = env_vars.get("DOCKER_COMPOSE_PROJECT_NAME", "")
            # Assuming standard compose network naming
            network = f"{project_name}_microservice_net"

        # For K8s, network is usually default or handled by CNI, we don't set it for TaskSpec usually?
        # DockerManager uses `docker run`.
        # If running on K8s, `ExpDriver` calls `deployment.run_task`.
        # K8sManager.run_task runs a Pod. Pods default to cluster network.

        # Loadgen Env
        loadgen_env = {
            "DURATION": env_vars.get("DURATION", "0"),
            "RPS_VALUES": env_vars.get("RPS_VALUES", "[]"),
            "STATS_INTERVAL_SEC": env_vars.get("STATS_INTERVAL_SEC", "1"),
            "HOST_TRACE_DIR": "/app/loadgen_output",
            "LOG_LEVEL": env_vars.get("LOG_LEVEL", "info"),
        }
        if "MAX_IN_FLIGHT" in env_vars:
            loadgen_env["MAX_IN_FLIGHT"] = env_vars["MAX_IN_FLIGHT"]

        frontend_json_path = output_dir / "frontend.json"

        # Artifacts
        artifacts = [("/app/loadgen_output/.", "")]  # Copy to output_dir

        cmd_str = "mssim-loadgen && echo 'MSSIM_LOADGEN_DONE'"
        if use_k8s:
            # In K8s, we need to keep the pod running to copy artifacts via exec
            cmd_str += " && sleep infinity"

        return TaskSpec(
            name="mssim-loadgen",
            image=MSSIM_LOADGEN_IMAGE,
            env_vars=loadgen_env,
            volumes={str(frontend_json_path): "/app/frontend.json"},
            artifacts=artifacts,
            network=network,
            command=[
                "/bin/sh",
                "-c",
                cmd_str,
            ],
            wait_for_log_pattern="MSSIM_LOADGEN_DONE",
            cleanup=True,
        )

    def get_required_images(self, features: Optional[str] = None) -> list[str]:
        policy = (features or "").strip() or "default"
        feature_image = _generic_service_image_for_policy(policy)
        images: list[str] = []
        for img in (MSSIM_LOADGEN_IMAGE, feature_image, GENERIC_SERVICE_IMAGE):
            if img and img not in images:
                images.append(img)
        return images

    def create_builder(self) -> AppBuilder:
        return self._builder

    def verify_results(self, config: "ExperimentConfig") -> bool:
        """
        Verification logic for MSSIM.
        Checks for presence of metadata and latency files, and validates goodput.
        """
        logger.info(f"Verifying MSSIM experiment: {config.experiment_name}")

        gen_config = config.gen_config
        try:
            rps_list = gen_config["Rps"]
            repeats = gen_config.get("Repeats", 1)
            duration = gen_config["DurationSecs"]
        except KeyError as e:
            logger.error(f"Missing key in gen_config.json: {e}")
            return False

        policies = config.policies

        # Check done marker
        done_file = config.out_dir / "done"
        if not done_file.exists():
            logger.error(f"Experiment not marked as complete: {done_file} missing")
            return False

        all_passed = True

        for i in range(repeats):
            for policy in policies:
                # MSSIM structure: {out_dir}/{iteration}/{policy}/run_0/
                # ExpDriver uses output_dir passed to prepare_workload.
                # Experiment.py sets output_dir = config.out_dir / iteration / policy.
                # But ExpDriver doesn't create "run_0" subdir automatically unless App does.
                # MssimApp.prepare_workload uses output_dir directly.
                # So we should look in {out_dir}/{iteration}/{policy}/ (no run_0)

                # Wait, MssimApp used to create run_0. Now it doesn't.
                # So verification logic needs to change to look in policy_run_dir directly?
                # ExpDriver: output_dir = self.config.out_dir / str(iteration) / policy

                policy_run_dir = config.out_dir / str(i) / policy
                if not policy_run_dir.exists():
                    logger.error(
                        f"MSSIM policy run directory missing: {policy_run_dir}"
                    )
                    all_passed = False
                    continue

                if not (policy_run_dir / "metadata.json").exists():
                    logger.error(
                        f"MSSIM metadata.json missing: {policy_run_dir / 'metadata.json'}"
                    )
                    all_passed = False

                for rps in rps_list:
                    # Look for standard format files first: r{rps}_{api}.csv
                    # Note: RPS in filename might be formatted (e.g. 100 or 100_5)
                    # We use glob to find matching files
                    latency_files = list(policy_run_dir.glob(f"r{int(rps)}_*.csv"))

                    # Fallback to legacy format
                    if not latency_files:
                        legacy_file = (
                            policy_run_dir / f"root_latencies_{int(rps)}rps.csv"
                        )
                        if legacy_file.exists():
                            latency_files = [legacy_file]

                    if not latency_files:
                        logger.error(
                            f"MSSIM latency files missing for {rps} RPS in {policy_run_dir}"
                        )
                        all_passed = False
                        continue

                    # Calculate goodput across all files for this RPS
                    current_rps_goodput = 0
                    for latency_file in latency_files:
                        try:
                            with open(latency_file, "r") as f:
                                reader = csv.reader(f)
                                header = next(reader, None)
                                if not header:
                                    continue

                                # Find error column index
                                try:
                                    error_idx = header.index("error")
                                except ValueError:
                                    # Fallback for legacy files without header or different names
                                    # Legacy format: error is at index 6
                                    error_idx = 6

                                for row in reader:
                                    if len(row) > error_idx:
                                        error = row[error_idx].strip()
                                        if error == "/None":
                                            current_rps_goodput += 1
                        except Exception as e:
                            logger.error(
                                f"Failed to read MSSIM CSV {latency_file}: {e}"
                            )
                            all_passed = False
                            continue

                    expected_total = rps * duration
                    lower = expected_total * 0.8
                    upper = expected_total * 1.2

                    if not (lower <= current_rps_goodput <= upper):
                        observed_rps = current_rps_goodput / duration
                        logger.error(
                            f"Goodput mismatch for {rps} RPS (Policy: {policy}, Iteration: {i})\n"
                            f"  Expected: ~{expected_total:.0f} (+/- 20%)\n"
                            f"  Got: {current_rps_goodput}\n"
                            f"  Observed RPS: {observed_rps:.2f} (Target: {rps:.2f})"
                        )
                        all_passed = False

        return all_passed
