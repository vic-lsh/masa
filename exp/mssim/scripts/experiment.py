#!/usr/bin/env python3

"""Run Alibaba simulator experiments from a JSON plan."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional

REPO_ROOT = Path(__file__).parent.parent.parent.parent.resolve()
MSSIM_ROOT = REPO_ROOT / "apps" / "mssim"
GENERIC_SERVICE_IMAGE = "generic_service"

print("repo root is ", REPO_ROOT)
print("mssim root is ", MSSIM_ROOT)

@dataclass
class ExperimentConfig:
    name: str
    trace_dir: Path
    config_dir: Path
    output_root: Path
    duration_sec: int
    warmup_sec: int
    slo_ms: int
    policies: List[str]
    rps_values: List[float]
    repeats: int = 1
    orchestrator: str = "localhost:50051"
    replay_path: Optional[Path] = None
    max_in_flight: Optional[int] = None
    stats_interval_sec: Optional[int] = None
    extra_env: Dict[str, str] = field(default_factory=dict)


def load_config(path: Path) -> ExperimentConfig:
    data = json.loads(path.read_text())
    try:
        name = data["experiment_name"]
        trace_dir = Path(data["trace_dir"]).expanduser().resolve()
        
        config_dir = (Path(data["config_dir"]) if data.get("config_dir") else Path("."))
        config_dir = config_dir.expanduser().resolve()
        
        output_root = Path(data.get("output_root", f"apps/mssim/data/experiments")) / name
        
        duration_sec = int(data.get("duration_sec", 0))
        warmup_sec = int(data.get("warmup_sec", 0))
        
        policies = list(data.get("policies", []))
        rps_values = [float(v) for v in data.get("rps_values", [])]
        slo_ms = int(data["slo_ms"])
    except (KeyError, TypeError, ValueError) as err:
        raise ValueError(f"invalid experiment config: {err}") from err

    if not policies or not rps_values:
        raise ValueError("experiment config must list at least one policy and RPS value")
    repeats = int(data.get("repeats", 1))
    extra_env = {str(k): str(v) for k, v in data.get("extra_env", {}).items()}
    replay_path = data.get("replay_path")

    cfg = ExperimentConfig(
        name=name,
        trace_dir=trace_dir,
        config_dir=config_dir,
        output_root=output_root,
        duration_sec=duration_sec,
        warmup_sec=warmup_sec,
        slo_ms=slo_ms,
        policies=policies,
        rps_values=rps_values,
        repeats=repeats,
        orchestrator=str(data.get("orchestrator", "localhost:50051")),
        replay_path=Path(replay_path).expanduser().resolve() if replay_path else None,
        max_in_flight=(int(data["max_in_flight"]) if "max_in_flight" in data else None),
        stats_interval_sec=(int(data["stats_interval_sec"]) if "stats_interval_sec" in data else None),
        extra_env=extra_env,
    )
    return cfg


def run_once(
    cfg: ExperimentConfig,
    run_dir: Path,
    policy: str,
    rps: float,
    service_image: str,
) -> int:
    env = os.environ.copy()
    env.update(cfg.extra_env)
    env.setdefault("ORCHESTRATOR", cfg.orchestrator)
    env["FEATURE"] = policy
    env["RPS"] = f"{rps}"
    env["SLO_MS"] = str(cfg.slo_ms)
    if cfg.max_in_flight:
        env["MAX_IN_FLIGHT"] = str(cfg.max_in_flight)
    if cfg.stats_interval_sec:
        env["STATS_INTERVAL_SEC"] = str(cfg.stats_interval_sec)
    if cfg.replay_path:
        env["REPLAY_TRACE_PATH"] = str(cfg.replay_path)
    if cfg.duration_sec > 0:
        env["DURATION"] = str(cfg.duration_sec)
    if cfg.warmup_sec > 0:
        env["WARMUP_SEC"] = str(cfg.warmup_sec)

    env["GENERIC_SERVICE_IMAGE"] = service_image
    env["HOST_TRACE_DIR"] = run_dir.resolve()

    # Fail if required input paths do not exist
    missing_paths = [
        str(path)
        for path in (cfg.trace_dir, cfg.config_dir)
        if not path.exists()
    ]
    if cfg.replay_path and not cfg.replay_path.exists():
        missing_paths.append(str(cfg.replay_path))
    if missing_paths:
        joined = ", ".join(missing_paths)
        raise FileNotFoundError(f"Required input paths do not exist: {joined}")

    docker_compose_path = (run_dir / "docker-compose.yml").resolve()
    print("docker_compose_path is ", docker_compose_path)

    deployment_json_path = (run_dir / "deployment.json").resolve()
    print("deployment_json_path is ", deployment_json_path)

    trace_cmd = [
        sys.executable,
        "-m",
        "simulator_py.main",
        "-a",
        str(cfg.trace_dir),
        "-c",
        str(cfg.config_dir),
        "--docker-compose-output-path",
        str(docker_compose_path),
        "--deployment-output-path",
        str(deployment_json_path),
    ]
    if cfg.replay_path:
        trace_cmd.extend(["--replay-path", str(cfg.replay_path)])

    down_cmd = ["docker",
                "compose",
                "-f",
                str(docker_compose_path),
                "-p",
                "mssim",
                "down", 
                "--volumes"]

    up_cmd = [
        "docker", 
        "compose", 
        "-f",
        str(docker_compose_path),
        "-p",
        "mssim",
        "up",
        "--abort-on-container-exit",
    ]
    
    log_path = run_dir / "orchestrator.log"
    
    with log_path.open("wb") as log_file:
            print(f"Generating YAML with command: {' '.join(trace_cmd)}")
            proc = subprocess.run(
                trace_cmd,
                cwd=MSSIM_ROOT,
                env=env,
                stdout=log_file,
                stderr=subprocess.STDOUT,
            )

    if proc.returncode != 0:
        print(f"Trace command failed with return code {proc.returncode}. See log at {log_path}")
        return proc.returncode
    
    proc = None
    try:
        with log_path.open("ab") as log_file:
            print(f"Starting docker experiment with command: {' '.join(up_cmd)}")
            proc = subprocess.Popen(
                up_cmd,
                cwd=MSSIM_ROOT,
                env=env,
                stdout=log_file,
                stderr=subprocess.STDOUT,
            )

            grace_period = 300
            timeout = cfg.duration_sec + grace_period if cfg.duration_sec > 0 else None
            return_code = proc.wait(timeout=timeout)
            return return_code or 0
            
    except subprocess.TimeoutExpired:
        print(f"Timeout expired after {timeout} seconds. Forcing shutdown...")
        return 1
    except KeyboardInterrupt:
        print("\nKeyboard interrupt received. Shutting down...")
        return 1
    finally:
        print("Cleaning up docker-compose services...")
        if proc and proc.poll() is None:
            proc.send_signal(signal.SIGINT)
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                proc.kill()
        
        # Run docker-compose down to clean up all resources.
        subprocess.run(down_cmd, cwd=MSSIM_ROOT, check=False)
        print("Cleanup complete.")


def build_load_generator_image():
    print("Building mssim load generator image")
    build_cmd = [
        "docker",
        "buildx",
        "build",
        "--load",
        "-t",
        "mssim_load_generator",
        "--target",
        "loadgen",
        "-f",
        "apps/mssim/generic-service/Dockerfile",
        REPO_ROOT, # this should be the masa project root
    ]
    subprocess.run(build_cmd, cwd=REPO_ROOT, check=True)


def _canonicalize_features(feature: str) -> str:
    parts = [part.strip() for part in re.split(r"[\s,]+", feature) if part.strip()]
    if not parts:
        return ""
    return ",".join(sorted(set(parts)))


def _feature_tag(features: str) -> str:
    canonical = features or "default"
    slug = re.sub(r"[^a-z0-9_.-]+", "-", canonical.lower()).strip("-.")
    digest = hashlib.sha256(canonical.encode("utf-8")).hexdigest()[:12]
    if slug:
        tag = f"{slug}-{digest}"
    else:
        tag = digest
    if len(tag) > 120:
        trimmed = slug[:100].rstrip("-.")
        tag = f"{trimmed}-{digest}" if trimmed else digest
    return tag


def _docker_image_exists(image: str) -> bool:
    result = subprocess.run(
        ["docker", "image", "inspect", image],
        cwd=REPO_ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return result.returncode == 0


def build_generic_service_image(feature: str) -> str:
    canonical_features = _canonicalize_features(feature)
    features_for_build = canonical_features or feature.strip() or "default"
    tag_suffix = _feature_tag(canonical_features)
    feature_image = f"{GENERIC_SERVICE_IMAGE}:{tag_suffix}"

    if _docker_image_exists(feature_image):
        print(f"Reusing cached image {feature_image}")
    else:
        print(
            f"Building mssim generic service image for features '{features_for_build}'",
        )
        build_cmd = [
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
            "apps/mssim/generic-service/Dockerfile",
            REPO_ROOT,
        ]
        subprocess.run(build_cmd, cwd=REPO_ROOT, check=True)

    subprocess.run(
        ["docker", "tag", feature_image, GENERIC_SERVICE_IMAGE],
        cwd=REPO_ROOT,
        check=True,
    )

    return feature_image



def execute(cfg: ExperimentConfig, dry_run: bool) -> None:
    def _ensure_dir(path: Path) -> Path:
        path.mkdir(parents=True, exist_ok=True)
        return path
    
    def build_runs(cfg: ExperimentConfig) -> Iterable[Dict[str, object]]:
        for policy in cfg.policies:
            for rps in cfg.rps_values:
                for repeat_idx in range(cfg.repeats):
                    yield {
                        "policy": policy,
                        "rps": rps,
                        "repeat": repeat_idx,
                    }
    
    def write_metadata(run_dir: Path, metadata: Dict[str, object]) -> None:
        with (run_dir / "metadata.json").open("w", encoding="utf-8") as fh:
            json.dump(metadata, fh, indent=2, sort_keys=True)

    build_load_generator_image()
    feature_key_prev = None
    current_generic_service_image = GENERIC_SERVICE_IMAGE
    for run in build_runs(cfg):
        policy = str(run["policy"])
        rps = float(run["rps"])
        repeat = int(run["repeat"])
        run_dir = _ensure_dir(cfg.output_root / policy / f"rps_{rps:g}" / f"run_{repeat:01d}")
        metadata = {
            "policy": policy,
            "rps": rps,
            "repeat": repeat,
            "duration_sec": cfg.duration_sec,
        }

        feature_key = _canonicalize_features(policy) or policy.strip() or "default"
        if feature_key != feature_key_prev:
            current_generic_service_image = build_generic_service_image(policy)
            feature_key_prev = feature_key

        # Write metadata before running
        write_metadata(run_dir, metadata)
        if dry_run:
            print(f"[dry-run] would execute policy={policy} rps={rps} repeat={repeat}")
            continue
        rc = run_once(cfg, run_dir, policy, rps, current_generic_service_image)
        status = "ok" if rc == 0 else f"failed({rc})"
        print(f"run policy={policy} rps={rps} repeat={repeat}: {status}")


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run MSSIM experiments")
    parser.add_argument(
        "--config",
        required=True,
        type=Path,
        help="Path to experiment JSON file",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print planned runs without executing",
    )
    return parser.parse_args(argv)


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    try:
        cfg = load_config(args.config)
    except Exception as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    execute(cfg, args.dry_run)
    return 0


if __name__ == "__main__":
    sys.exit(main())
