"""
Hotel application plugin.
"""

import hashlib
import json
import logging
import os
import re
import shlex
import shutil
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import normalize_features_to_tag, get_docker_progress_flag
from ..cpu_monitor import CPUMonitor

logger = logging.getLogger(__name__)


def _safe_project_name(*, experiment_name: str, iteration: int, policy: str) -> str:
    """Generate safe docker-compose project name for hotel experiments.

    Format: hotel-{slug}-{digest}
    - slug: sanitized experiment name (max 12 chars to keep total name under 63 char docker limit)
    - digest: 12-char hash for uniqueness
    """
    raw = f"{experiment_name}|{iteration}|{policy}"
    digest = hashlib.sha256(raw.encode("utf-8")).hexdigest()[:12]
    slug = re.sub(r"[^a-z0-9]+", "-", experiment_name.lower()).strip("-")[:12] or "exp"
    return f"hotel-{slug}-{digest}"


def _generate_gen_config(
    *,
    template_config: dict,
    project_name: str,
    output_path: Path,
) -> None:
    """
    Generate project-specific gen_config.json with namespaced frontend address.

    Updates the "Addr" field to point to the project-prefixed frontend service.
    """
    import copy

    config = copy.deepcopy(template_config)

    # Parse the original address
    if "Addr" in config:
        addr = config["Addr"]
        # Extract protocol and port from original address
        if "://" in addr:
            protocol, rest = addr.split("://", 1)
            if ":" in rest:
                _, port = rest.rsplit(":", 1)
            else:
                port = "8660"  # default
        else:
            protocol = "http"
            port = "8660"

    # Use the service alias on the compose network so DNS returns all replicas.
    config["Addr"] = f"{protocol}://hotel_frontend:{port}"

    # Write to file
    with output_path.open("w") as f:
        json.dump(config, f, indent=2)


def _generate_hotel_config(
    *,
    template_config: dict,
    project_name: str,
    output_path: Path,
) -> None:
    """
    Generate project-specific hotel.json with namespaced service names.

    Transforms service IPs from static names to project-prefixed container names:
    - "local-rate-service" -> "{project_name}-rate-service"
    - "rate_mongo" -> "{project_name}-rate-mongo-1"
    """
    import copy

    config = copy.deepcopy(template_config)

    # Service name mappings: config key -> (compose service name, is_scaled)
    # Scaled services use service name for DNS load balancing
    # Non-scaled services get explicit -1 suffix
    service_mappings = {
        "geo": ("geo-service", True),
        "profile": ("profile-service", True),
        "rate": ("rate-service", True),
        "recommendation": ("recommendation-service", True),
        "reservation": ("reservation-service", True),
        "review": ("review-service", True),
        "search": ("search-service", True),
        "user": ("user-service", True),
        # Frontend is not scaled, but we reference it by service name
        "frontend": ("hotel-frontend-service", False),
    }

    # Update service IPs
    for key, (service_name, is_scaled) in service_mappings.items():
        if key in config:
            if is_scaled:
                # Scaled services: use service name (docker-compose load balances)
                config[key]["ip"] = f"{project_name}-{service_name}"
            else:
                # Non-scaled services: use container name with -1 suffix
                config[key]["ip"] = f"{project_name}-{service_name}-1"

    # Update infrastructure addresses (mongo, redis)
    # These are always single containers with -1 suffix
    infra_mappings = {
        "profile": {"mongodbAddr": "profile-mongo", "redisAddr": "profile-redis"},
        "rate": {"mongodbAddr": "rate-mongo", "redisAddr": "rate-redis"},
        "reservation": {"mongodbAddr": "reservation-mongo", "redisAddr": "reservation-redis"},
        "user": {"mongodbAddr": "user-mongo"},
    }

    for service, addrs in infra_mappings.items():
        if service in config:
            for addr_key, infra_service in addrs.items():
                if addr_key in config[service]:
                    # Parse and update the address
                    old_addr = config[service][addr_key]
                    if "://" in old_addr:
                        protocol, rest = old_addr.split("://", 1)
                        hostname, *port_parts = rest.split(":", 1)
                        new_hostname = f"{project_name}-{infra_service}-1"
                        if port_parts:
                            new_addr = f"{protocol}://{new_hostname}:{port_parts[0]}"
                        else:
                            new_addr = f"{protocol}://{new_hostname}"
                        config[service][addr_key] = new_addr

    # Write to file
    with output_path.open("w") as f:
        json.dump(config, f, indent=2)


class HotelLoadGenerator(LoadGenerator):
    """Load generator for the hotel reservation application."""

    def __init__(self, features: Optional[str] = None, project_name: Optional[str] = None):
        """
        Initialize load generator with optional features for image tagging.

        Args:
            features: Cargo features used to build the image
            project_name: Docker compose project name for namespace isolation
        """
        self.features = features
        self.project_name = project_name

    def get_container_name(self) -> str:
        if self.project_name:
            return f"{self.project_name}_hotel_client_bench"
        return "hotel_client_bench"

    def get_network_name(self) -> str:
        if self.project_name:
            return f"{self.project_name}_hotel_network"
        return "local_hotel_network"

    def get_image_name(self) -> str:
        tag = normalize_features_to_tag(self.features)
        if tag and tag != "latest":
            return f"hotel_client_bench:{tag}"
        else:
            return "hotel_client_bench:latest"

    def get_binary_name(self) -> str:
        return "hotel_client_bench"


class HotelBuilder(AppBuilder):
    """
    Build logic for the hotel app docker images.

    Uses multi-stage multi-target build:
    - Stage 1 (builder): Build all binaries once
    - Stage 2 (runtime-base): Base runtime image with dependencies
    - Stage 3 (runtime): Per-binary runtime images

    The hotel app requires separate docker images for each binary:
    - hotel_client_bench:latest - load generator
    - hotel_frontend:latest - frontend service
    - hotel_geo:latest - geo service
    - hotel_rate:latest - rate service
    - hotel_review:latest - review service
    - hotel_search:latest - search service
    - hotel_profile:latest - profile service
    - hotel_reservation:latest - reservation service
    - hotel_user:latest - user service
    - hotel_recommendation:latest - recommendation service

    Mirrors the behavior of exp/common/scripts/docker-build.sh, but lives in Python
    so the runner can select an app-specific build implementation.
    """

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
        build_logs_dir: Optional[Path] = None,
    ) -> Optional[list[list[str]]]:
        app = "hotel"
        # List of binaries to build (each gets its own image)
        # Note: All binary names already include the hotel_ prefix
        binaries_list = [
            "hotel_client_bench",
            "hotel_frontend",
            "hotel_geo",
            "hotel_rate",
            "hotel_review",
            "hotel_search",
            "hotel_profile",
            "hotel_reservation",
            "hotel_user",
            "hotel_recommendation",
        ]

        if gen_config_path is None:
            raise ValueError("gen_config_path is required for hotel app")

        # Convert to path relative to repo_root
        gen_config_path_rel = gen_config_path.relative_to(repo_root)

        # Generate tag based on features for deterministic, feature-specific images
        tag = normalize_features_to_tag(features)
        
        logger.info(f"Building {len(binaries_list)} docker images for hotel app using multi-stage build")
        if features:
            logger.info(f"Using features: {features}")

        # Collect commands if dry_run
        commands: list[list[str]] = []

        # Start timing the docker build
        build_start_time = time.time()

        # Stage 1: Build all binaries once (shared across all images)
        logger.info("Stage 1: Building all binaries for hotel app")
        stage1_start_time = time.time()
        builder_build_args: list[str] = []
        if features:
            builder_build_args.extend(["--build-arg", f"FEATURES={features}"])
        builder_build_args.extend(["--build-arg", f"APP={app}"])
        # Use a unique cache ID to avoid race conditions in parallel builds
        cache_id = f"{app}-{tag}"
        builder_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])
        
        builder_cmd: list[str] = [
            "docker",
            "buildx",
            "build",
            "-f",
            "./exp/common/docker-build/Dockerfile",
            "--target",
            "builder",
            *builder_build_args,
            "--ulimit",
            "nofile=4096:4096",
            get_docker_progress_flag(),
        ]
        
        if no_cache:
            builder_cmd.append("--no-cache")
        
        builder_cmd.extend(["-t", f"{app}_builder:{tag}", "."])
        
        if dry_run:
            commands.append(builder_cmd.copy())
        else:
            try:
                subprocess.run(
                    builder_cmd,
                    cwd=repo_root,
                    check=True,
                    capture_output=False,
                )
            except subprocess.CalledProcessError as e:
                logger.error(f"Failed to build builder stage. Command: {shlex.join(builder_cmd)}")
                raise
        stage1_duration = time.time() - stage1_start_time
        logger.info(f"Stage 1 complete: All binaries built ({stage1_duration:.2f}s)")

        # Stage 2: Build runtime-base (shared across all images)
        logger.info("Stage 2: Building runtime-base image")
        stage2_start_time = time.time()
        runtime_base_build_args: list[str] = []
        if features:
            runtime_base_build_args.extend(["--build-arg", f"FEATURES={features}"])
        runtime_base_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
        runtime_base_build_args.extend(["--build-arg", f"APP={app}"])
        runtime_base_build_args.extend(["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"])
        # Use consistent cache ID based on features across all stages
        runtime_base_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])
        
        runtime_base_cmd: list[str] = [
            "docker",
            "buildx",
            "build",
            "-f",
            "./exp/common/docker-build/Dockerfile",
            "--target",
            "runtime-base",
            *runtime_base_build_args,
            "--ulimit",
            "nofile=4096:4096",
            get_docker_progress_flag(),
        ]
        
        if no_cache:
            runtime_base_cmd.append("--no-cache")
        
        runtime_base_cmd.extend(["-t", f"{app}_runtime-base:{tag}", "."])
        
        if dry_run:
            commands.append(runtime_base_cmd.copy())
        else:
            try:
                subprocess.run(
                    runtime_base_cmd,
                    cwd=repo_root,
                    check=True,
                    capture_output=False,
                )
            except subprocess.CalledProcessError as e:
                logger.error(f"Failed to build runtime-base stage. Command: {shlex.join(runtime_base_cmd)}")
                raise
        stage2_duration = time.time() - stage2_start_time
        logger.info(f"Stage 2 complete: Runtime-base image built ({stage2_duration:.2f}s)")

        # Stage 3: Build per-binary runtime images IN PARALLEL
        def build_runtime_image(binary_name: str) -> tuple[str, bool, Optional[str], Optional[Path]]:
            """Build a single runtime image. Returns (binary_name, success, error_msg, log_file)."""
            log_file = None
            try:
                runtime_build_args: list[str] = []
                if features:
                    runtime_build_args.extend(["--build-arg", f"FEATURES={features}"])
                runtime_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
                runtime_build_args.extend(["--build-arg", f"APP={app}"])
                runtime_build_args.extend(["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"])
                runtime_build_args.extend(["--build-arg", f"BINARY_NAME={binary_name}"])
                # Use consistent cache ID based on features across all stages
                runtime_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

                # Generate image name: <binary>:<tag> or <binary>:latest if no features
                # Note: binary_name already includes the hotel_ prefix
                if tag:
                    image_name = f"{binary_name}:{tag}"
                else:
                    image_name = f"{binary_name}:latest"

                runtime_cmd: list[str] = [
                    "docker",
                    "buildx",
                    "build",
                    "-f",
                    "./exp/common/docker-build/Dockerfile",
                    "--target",
                    "runtime",
                    *runtime_build_args,
                    "--ulimit",
                    "nofile=4096:4096",
                ]

                if no_cache:
                    runtime_cmd.append("--no-cache")

                runtime_cmd.extend(["-t", image_name, "."])

                if dry_run:
                    # Add progress flag for dry-run display
                    runtime_cmd.insert(-1, get_docker_progress_flag())
                    commands.append(runtime_cmd.copy())
                    return (binary_name, True, None, None)

                # Write output to log file instead of terminal (for parallel builds)
                if build_logs_dir:
                    build_logs_dir.mkdir(parents=True, exist_ok=True)
                    log_file = build_logs_dir / f"{binary_name}.log"
                    logger.info(f"Building {image_name} (log: {log_file})")

                    # Use --progress=plain for file output (not tty)
                    build_cmd_with_progress = runtime_cmd.copy()
                    build_cmd_with_progress.insert(-1, "--progress=plain")

                    with open(log_file, "w") as f:
                        result = subprocess.run(
                            build_cmd_with_progress,
                            cwd=repo_root,
                            check=True,
                            stdout=f,
                            stderr=subprocess.STDOUT,
                            text=True,
                        )
                else:
                    logger.info(f"Building {image_name}")
                    # Use tty progress for terminal output
                    build_cmd_with_progress = runtime_cmd.copy()
                    build_cmd_with_progress.insert(-1, get_docker_progress_flag())
                    subprocess.run(
                        build_cmd_with_progress,
                        cwd=repo_root,
                        check=True,
                        capture_output=False,
                    )

                logger.info(f"Successfully built docker image: {image_name}")
                return (binary_name, True, None, log_file)

            except subprocess.CalledProcessError as e:
                error_msg = f"Failed to build runtime image for {binary_name}"
                return (binary_name, False, error_msg, log_file)

        # Build all runtime images in parallel
        if not dry_run:
            logger.info("Stage 3: Building all runtime images in parallel")
            stage3_start_time = time.time()
            build_errors = []
            completed_binaries = []

            with ThreadPoolExecutor(max_workers=10) as executor:
                # Submit all build tasks
                futures = {executor.submit(build_runtime_image, binary): binary
                           for binary in binaries_list}

                # Collect results as they complete
                for future in as_completed(futures):
                    binary_name, success, error_msg, log_file = future.result()
                    if success:
                        completed_binaries.append(binary_name)
                    else:
                        build_errors.append((binary_name, error_msg, log_file))

            # Check if any builds failed
            if build_errors:
                logger.error(f"Failed to build {len(build_errors)} images:")
                for binary_name, error_msg, log_file in build_errors:
                    msg = f"  - {binary_name}: {error_msg}"
                    if log_file:
                        msg += f" (see {log_file})"
                    logger.error(msg)
                raise RuntimeError(f"Failed to build {len(build_errors)} runtime images")

            stage3_duration = time.time() - stage3_start_time
            logger.info(f"Stage 3 complete: Built all {len(completed_binaries)} runtime images ({stage3_duration:.2f}s)")
            if build_logs_dir:
                logger.info(f"Build logs written to: {build_logs_dir}")
        else:
            # In dry-run mode, still call build_runtime_image to collect commands
            for binary_name in binaries_list:
                build_runtime_image(binary_name)
        
        if dry_run:
            return commands
        
        # Calculate and print build duration
        build_duration = time.time() - build_start_time
        logger.info(f"Docker image building took {build_duration:.2f} seconds ({build_duration/60:.2f} minutes)")
        logger.info(f"  Stage 1 (build binaries): {stage1_duration:.2f}s")
        logger.info(f"  Stage 2 (runtime-base): {stage2_duration:.2f}s")
        logger.info(f"  Stage 3 (parallel copying): {stage3_duration:.2f}s")
        logger.info("All hotel app docker images built successfully")
        return None


class HotelApp(AppPlugin):
    """
    Plugin for the hotel reservation microservices application.
    
    The hotel app consists of multiple microservices (rate, profile, reservation,
    geo, search, user, recommendation, review) that can be replicated.
    """
    
    def get_app_name(self) -> str:
        return "hotel"
    
    def load_app_config(self, config_path: Path) -> dict:
        """Load hotel.json configuration file."""
        with open(config_path) as f:
            return json.load(f)
    
    def generate_env_vars(
        self,
        gen_config: dict,
        app_config: Optional[dict],
        app_dir: Path
    ) -> dict:
        """
        Generate environment variables for hotel application.
        
        Extracts frontend port from gen_config and replica counts from hotel.json.
        """
        env_vars = {}
        
        # Extract frontend port from address (e.g., "http://[::1]:8659" -> "8659")
        addr = gen_config.get("Addr", "")
        match = re.search(r':(\d+)$', addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")
        
        # Set replica counts from hotel.json
        default_replicas = 1
        services = [
            "rate", "profile", "reservation", "geo",
            "search", "user", "recommendation", "review"
        ]
        
        if app_config:
            for service in services:
                env_key = f"{service.upper()}_REPLICAS"
                replicas = app_config.get(service, {}).get("replicas", default_replicas)
                env_vars[env_key] = str(replicas)
            frontend_replicas = app_config.get("frontend", {}).get("replicas", default_replicas)
            env_vars["FRONTEND_REPLICAS"] = str(frontend_replicas)
        else:
            # Use defaults if no config provided
            for service in services:
                env_key = f"{service.upper()}_REPLICAS"
                env_vars[env_key] = str(default_replicas)
            env_vars["FRONTEND_REPLICAS"] = str(default_replicas)
        
        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "info"
        
        return env_vars
    
    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for hotel application."""
        return DockerConfig(
            compose_file="scripts/local/containers+svcs.yaml",
            network_name="local_hotel_network",
            loadgen_image_name="hotel_client_bench:<features>",  # Actual tag is dynamic based on features
            loadgen_binary_name="hotel_client_bench",
            app_config_filename="hotel.json",
        )
    
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for hotel application.
        
        Includes frontend and replicated service containers based on
        replica counts from environment variables.
        """
        frontend_replicas = int(env_vars.get("FRONTEND_REPLICAS", 1))
        container_names = [
            f"local-hotel-frontend-service-{i}"
            for i in range(1, frontend_replicas + 1)
        ]
        
        # Services that can be replicated
        replicated_services = {
            "rate": int(env_vars.get("RATE_REPLICAS", 1)),
            "profile": int(env_vars.get("PROFILE_REPLICAS", 1)),
            "reservation": int(env_vars.get("RESERVATION_REPLICAS", 1)),
            "geo": int(env_vars.get("GEO_REPLICAS", 1)),
            "search": int(env_vars.get("SEARCH_REPLICAS", 1)),
            "user": int(env_vars.get("USER_REPLICAS", 1)),
        }
        
        # Generate container names for each replica
        for service, count in replicated_services.items():
            for i in range(1, count + 1):
                container_names.append(f"local-{service}-service-{i}")
        
        return container_names
    
    def create_load_generator(self, features: Optional[str] = None, project_name: Optional[str] = None) -> LoadGenerator:
        """
        Create a load generator instance for hotel application.

        Args:
            features: Optional cargo features used to build the image
            project_name: Optional docker compose project name for namespace isolation
        """
        return HotelLoadGenerator(features=features, project_name=project_name)

    def create_builder(self) -> AppBuilder:
        """Create a builder instance for hotel application."""
        return HotelBuilder()
    
    def get_image_tag(self, features: Optional[str] = None) -> str:
        """
        Get the docker image tag for the given features.

        Args:
            features: Optional cargo features

        Returns:
            Docker image tag string
        """
        return normalize_features_to_tag(features)

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
        """Run hotel experiment with namespace isolation."""
        import sys

        # Generate project name for namespace isolation
        project_name = _safe_project_name(
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )

        # Setup paths - write directly to output_dir (no run_* subdirectory)
        output_dir.mkdir(parents=True, exist_ok=True)

        # Generate project-specific hotel.json
        project_config_path = output_dir / "hotel.json"
        _generate_hotel_config(
            template_config=config.app_config,
            project_name=project_name,
            output_path=project_config_path,
        )

        # Generate project-specific gen_config.json
        project_gen_config_path = output_dir / "gen_config.json"
        _generate_gen_config(
            template_config=config.gen_config,
            project_name=project_name,
            output_path=project_gen_config_path,
        )

        # Generate environment variables
        env_vars = self.generate_env_vars(config.gen_config, config.app_config, config.app_dir)

        # Add image tag based on policy/features
        tag = self.get_image_tag(features=policy)
        env_vars["HOTEL_IMAGE_TAG"] = tag if tag else "latest"

        # Clean up old build logs before building
        build_logs_dir = repo_root / "exp" / "hotel" / "data" / "out" / config.experiment_name / str(iteration) / policy / "build_logs"
        if build_logs_dir.exists():
            logger.info(f"Cleaning build logs directory: {build_logs_dir}")
            shutil.rmtree(build_logs_dir, ignore_errors=True)

        # Build docker images (use ORIGINAL config for build, not project-specific)
        builder = self.create_builder()
        build_cmds = builder.build(
            repo_root=repo_root,
            app_dir=config.app_dir,
            features=policy,
            rust_log=env_vars.get("LOG_LEVEL", "info"),
            no_cache=no_cache,
            gen_config_path=(config.in_dir / "gen_config.json"),
            dry_run=dry_run,
            build_logs_dir=build_logs_dir,
        )
        if dry_run and build_cmds:
            print("\n".join(" ".join(cmd) for cmd in build_cmds))

        docker_compose_path = config.app_dir / "scripts" / "local" / "containers+svcs.yaml"

        # Setup environment for docker compose
        env = os.environ.copy()
        env.update({k: str(v) for k, v in env_vars.items()})

        # Path to mount project-specific config
        env["PROJECT_CONFIG_PATH"] = str(project_config_path.resolve())

        # Docker compose commands with project name
        up_cmd = [
            "docker", "compose",
            "-f", str(docker_compose_path),
            "-p", project_name,
            "up", "-d",
        ]
        down_cmd = [
            "docker", "compose",
            "-f", str(docker_compose_path),
            "-p", project_name,
            "down", "--volumes",
        ]

        if dry_run:
            print(f"[dry-run] would run hotel policy={policy} iteration={iteration}")
            print(f"[dry-run] project name: {project_name}")
            print(f"[dry-run] generated config: {project_config_path}")
            print(f"[dry-run] would write outputs under: {output_dir}")
            print("[dry-run] compose up:", " ".join(up_cmd))
            print("[dry-run] compose down:", " ".join(down_cmd))
            return

        # Save metadata
        metadata = {
            "app": "hotel",
            "experiment": config.experiment_name,
            "iteration": iteration,
            "policy": policy,
            "docker_project": project_name,
        }
        with (output_dir / "metadata.json").open("w", encoding="utf-8") as fh:
            json.dump(metadata, fh, indent=2, sort_keys=True)

        # Initialize CPU monitor
        cpu_stats_file = output_dir / "cpu_stats.csv"
        cpu_monitor = CPUMonitor(output_path=cpu_stats_file, poll_interval=2.0)

        log_threads = []
        try:
            # Start services
            print(f"Starting hotel services for policy={policy} iteration={iteration} project={project_name}")
            subprocess.run(
                up_cmd,
                cwd=config.app_dir,
                env=env,
                check=True,
            )

            # Wait for services to be ready
            time.sleep(3)

            # Start CPU monitoring after services are up
            cpu_monitor.start()

            # Get container names for log streaming
            container_names = docker.get_container_names(
                compose_path=docker_compose_path,
                project_name=project_name,
                env_vars=env,
            )

            # Stream logs
            if container_names:
                logs_dir = output_dir / "logs"
                print(f"Streaming logs for {len(container_names)} containers to {logs_dir}")
                log_threads = docker.stream_logs(
                    container_names=container_names,
                    output_dir=logs_dir,
                    follow=True,
                )
            else:
                print("Warning: No containers found for log streaming")

            # Run load generator with project-specific gen_config
            load_gen = self.create_load_generator(features=policy, project_name=project_name)
            load_gen.run(
                output_dir=output_dir,
                env_vars=env_vars,
                gen_config_path=project_gen_config_path,
            )

        finally:
            # Stop CPU monitoring before stopping services
            try:
                cpu_monitor.stop()
            except Exception as e:
                logger.warning(f"Error stopping CPU monitor: {e}")

            # Cleanup
            subprocess.run(down_cmd, cwd=config.app_dir, env=env, check=False)

            # Wait for log threads to finish
            for thread in log_threads:
                thread.join(timeout=5)
