"""
Hotel application plugin.
"""

import hashlib
import json
import logging
import re
import shlex
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import TYPE_CHECKING, Optional, Tuple

if TYPE_CHECKING:
    from ..config import ExperimentConfig
    from ..deployment_manager import DeploymentManager as DockerManager

from ..deployment_manager import TaskSpec
from ..executor import CommandExecutor, MockCommandExecutor, SubprocessExecutor
from .base import AppBuilder, AppPlugin, DockerConfig
from .utils import get_docker_progress_flag, normalize_features_to_tag

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


def create_gen_config_dict(
    *,
    template_config: dict,
) -> dict:
    """
    Generate project-specific gen_config.json content with namespaced frontend address.

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

    return config


def create_hotel_config_dict(
    *,
    template_config: dict,
    project_name: str,
) -> dict:
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
        "reservation": {
            "mongodbAddr": "reservation-mongo",
            "redisAddr": "reservation-redis",
        },
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
    return config


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

    Mirrors the behavior of exp_runner/common/scripts/docker-build.sh, but lives in Python
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
        executor: Optional[CommandExecutor] = None,
    ) -> Optional[list[list[str]]]:
        executor = executor or SubprocessExecutor()
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

        logger.info(
            f"Building {len(binaries_list)} docker images for hotel app using multi-stage build"
        )
        if features:
            logger.info(f"Using features: {features}")

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
            "./exp_runner/common/docker-build/Dockerfile",
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

        try:
            executor.run(
                builder_cmd,
                cwd=repo_root,
                check=True,
                capture_output=False,
            )
        except subprocess.CalledProcessError:
            logger.error(
                f"Failed to build builder stage. Command: {shlex.join(builder_cmd)}"
            )
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
        runtime_base_build_args.extend(
            ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
        )
        # Use consistent cache ID based on features across all stages
        runtime_base_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

        runtime_base_cmd: list[str] = [
            "docker",
            "buildx",
            "build",
            "-f",
            "./exp_runner/common/docker-build/Dockerfile",
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

        try:
            executor.run(
                runtime_base_cmd,
                cwd=repo_root,
                check=True,
                capture_output=False,
            )
        except subprocess.CalledProcessError:
            logger.error(
                f"Failed to build runtime-base stage. Command: {shlex.join(runtime_base_cmd)}"
            )
            raise
        stage2_duration = time.time() - stage2_start_time
        logger.info(
            f"Stage 2 complete: Runtime-base image built ({stage2_duration:.2f}s)"
        )

        # Stage 3: Build per-binary runtime images IN PARALLEL
        def build_runtime_image(
            binary_name: str,
        ) -> tuple[str, bool, Optional[str], Optional[Path]]:
            """Build a single runtime image. Returns (binary_name, success, error_msg, log_file)."""
            log_file = None
            try:
                runtime_build_args: list[str] = []
                if features:
                    runtime_build_args.extend(["--build-arg", f"FEATURES={features}"])
                runtime_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
                runtime_build_args.extend(["--build-arg", f"APP={app}"])
                runtime_build_args.extend(
                    ["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"]
                )
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
                    "./exp_runner/common/docker-build/Dockerfile",
                    "--target",
                    "runtime",
                    *runtime_build_args,
                    "--ulimit",
                    "nofile=4096:4096",
                ]

                if no_cache:
                    runtime_cmd.append("--no-cache")

                runtime_cmd.extend(["-t", image_name, "."])

                if dry_run and isinstance(executor, MockCommandExecutor):
                    # Add progress flag for dry-run display
                    runtime_cmd.insert(-1, get_docker_progress_flag())
                    executor.run(runtime_cmd)
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
                        executor.run(
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
                    executor.run(
                        build_cmd_with_progress,
                        cwd=repo_root,
                        check=True,
                        capture_output=False,
                    )

                logger.info(f"Successfully built docker image: {image_name}")
                return (binary_name, True, None, log_file)

            except subprocess.CalledProcessError:
                error_msg = f"Failed to build runtime image for {binary_name}"
                return (binary_name, False, error_msg, log_file)

        # Build all runtime images in parallel
        # Note: If executor is MockCommandExecutor, we probably don't want parallel execution
        # as it might race on history. But since it's just appending to a list, it might be ok?
        # Actually MockExecutor is not thread safe by default for list append, but in Python GIL helps.
        # However, for Dry Run, we can just run them sequentially.
        if dry_run:
            for binary_name in binaries_list:
                build_runtime_image(binary_name)
        else:
            logger.info("Stage 3: Building all runtime images in parallel")
            stage3_start_time = time.time()
            build_errors = []
            completed_binaries = []

            with ThreadPoolExecutor(max_workers=10) as pool:
                # Submit all build tasks
                futures = {
                    pool.submit(build_runtime_image, binary): binary
                    for binary in binaries_list
                }

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
                raise RuntimeError(
                    f"Failed to build {len(build_errors)} runtime images"
                )

            stage3_duration = time.time() - stage3_start_time
            logger.info(
                f"Stage 3 complete: Built all {len(completed_binaries)} runtime images ({stage3_duration:.2f}s)"
            )
            if build_logs_dir:
                logger.info(f"Build logs written to: {build_logs_dir}")

        if dry_run and isinstance(executor, MockCommandExecutor):
            return [cmd.args for cmd in executor.history]

        # Calculate and print build duration
        build_duration = time.time() - build_start_time
        logger.info(
            f"Docker image building took {build_duration:.2f} seconds ({build_duration / 60:.2f} minutes)"
        )
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

    # ===== NEW SIMPLIFIED INTERFACE (Phase 4) =====

    def get_binaries(self) -> list[str]:
        """Return list of binary names for hotel app."""
        return [
            "hotel_frontend",
            "hotel_geo",
            "hotel_rate",
            "hotel_review",
            "hotel_search",
            "hotel_profile",
            "hotel_reservation",
            "hotel_user",
            "hotel_recommendation",
            "hotel_client_bench",
        ]

    def get_frontend_name(self) -> str:
        """Return the frontend service name."""
        return "hotel-frontend-service"

    def get_cargo_package(self) -> str:
        """Return cargo package name."""
        return "hotel"

    def get_build_parallelism(self) -> int:
        """Hotel has 10 binaries - use parallelism of 4."""
        return 4

    def get_default_topology_path(self, repo_root: Path) -> Optional[Path]:
        """
        Hotel topology is implicit in the application code.
        Return None to indicate no explicit topology file.
        """
        return None

    def customize_env_vars(self, topology, experiment, base_env):
        """
        Add hotel-specific environment variables.

        Args:
            topology: Topology specification
            experiment: Experiment configuration
            base_env: Base environment variables from generator

        Returns:
            Updated environment variables
        """
        # Add any hotel-specific env vars if needed
        if "LOG_LEVEL" not in base_env:
            base_env["LOG_LEVEL"] = "info"

        return base_env

    # ===== LEGACY INTERFACE (backward compatibility) =====

    @property
    def supports_k8s(self) -> bool:
        return False

    def load_app_config(self, config_path: Path) -> dict:
        """Load hotel.json configuration file."""
        with open(config_path) as f:
            return json.load(f)

    def generate_env_vars(
        self, gen_config: dict, app_config: Optional[dict], app_dir: Path
    ) -> dict:
        """
        Generate environment variables for hotel application.

        Extracts frontend port from gen_config and replica counts from hotel.json.
        """
        env_vars = {}

        # Extract frontend port from address (e.g., "http://[::1]:8659" -> "8659")
        addr = gen_config.get("Addr", "")
        match = re.search(r":(\d+)$", addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")

        # Set replica counts from hotel.json
        default_replicas = 1
        services = [
            "rate",
            "profile",
            "reservation",
            "geo",
            "search",
            "user",
            "recommendation",
            "review",
        ]

        if app_config:
            for service in services:
                env_key = f"{service.upper()}_REPLICAS"
                replicas = app_config.get(service, {}).get("replicas", default_replicas)
                env_vars[env_key] = str(replicas)
            frontend_replicas = app_config.get("frontend", {}).get(
                "replicas", default_replicas
            )
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
            f"local-hotel-frontend-service-{i}" for i in range(1, frontend_replicas + 1)
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
        """
        # Store state for get_deployment_location
        self._last_use_k8s = use_k8s
        self._last_use_new_generator = use_new_generator

        if use_new_generator:
            # New path: Use generators
            from ..topology import TopologyResolver
            from ..generators.compose import ComposeGenerator
            from ..generators.helm import HelmValuesGenerator
            from ..experiment_config_v2 import (
                ExperimentConfigV2,
                ExecutionSpec,
                LoadGenSpec,
            )

            # 1. Resolve Topology
            resolver = TopologyResolver(repo_root)
            topology = resolver.resolve("hotel", "default")

            # 2. Create ExperimentConfigV2 adapter from legacy config
            exp_v2 = ExperimentConfigV2(
                name=config.experiment_name,
                app="hotel",
                execution=ExecutionSpec(
                    repeats=1,
                    policies=[policy],
                ),
                replica_overrides={},
                loadgen=LoadGenSpec(rps=[]),
            )

            # Extract replica overrides from config.app_config if present
            if config.app_config:
                overrides = {}
                for svc, data in config.app_config.items():
                    if isinstance(data, dict) and "replicas" in data:
                        topo_name = (
                            f"{svc}-service" if svc != "frontend" else "frontend"
                        )
                        overrides[topo_name] = int(data["replicas"])
                exp_v2.replica_overrides = overrides

            # 3. Generate Deployment
            project_name = _safe_project_name(
                experiment_name=config.experiment_name,
                iteration=iteration,
                policy=policy,
            )

            tag = self.get_image_tag(features=policy)
            image_tag = tag if tag else "latest"

            if use_k8s:
                generator = HelmValuesGenerator()
                deploy = generator.generate(
                    topology=topology,
                    experiment=exp_v2,
                    output_dir=output_dir,
                    project_name=project_name,
                    policy=policy,
                    image_tag=image_tag,
                )
            else:
                generator = ComposeGenerator()
                deploy = generator.generate(
                    topology=topology,
                    experiment=exp_v2,
                    output_dir=output_dir,
                    project_name=project_name,
                    policy=policy,
                    image_tag=image_tag,
                )

            # 4. Generate gen_config.json
            gen_config_dict = create_gen_config_dict(
                template_config=config.gen_config,
            )

            # Update Addr
            if use_k8s:
                gen_config_dict["Addr"] = f"http://{project_name}-frontend:8660"
            else:
                gen_config_dict["Addr"] = "http://frontend:8660"

            project_gen_config_path = output_dir / "gen_config.json"
            with project_gen_config_path.open("w") as f:
                json.dump(gen_config_dict, f, indent=2)

            # 5. Generate app config (config.json)
            import copy

            app_config = copy.deepcopy(config.app_config)

            svc_map = {
                "geo": "geo-service",
                "profile": "profile-service",
                "rate": "rate-service",
                "recommendation": "recommendation-service",
                "reservation": "reservation-service",
                "review": "review-service",
                "search": "search-service",
                "user": "user-service",
                "frontend": "frontend",
            }

            def update_addr(addr, hostname, port=None):
                if "://" in addr:
                    protocol, rest = addr.split("://", 1)
                    if ":" in rest and not port:
                        _, port = rest.rsplit(":", 1)
                    elif not port:
                        port = "8080"
                    return f"{protocol}://{hostname}:{port}"
                return addr

            if use_k8s:
                # K8s: {project}-{name}
                prefix = f"{project_name}-"
                infra_prefix = f"{project_name}-"
            else:
                # Docker:
                # Services: {project}-{name} (app appends -1)
                prefix = f"{project_name}-"
                # Infra: {name} (resolves to service alias)
                infra_prefix = ""

            for key, hostname in svc_map.items():
                full_hostname = f"{prefix}{hostname}"
                if key in app_config:
                    if "ip" in app_config[key]:
                        app_config[key]["ip"] = full_hostname
                    # Update port to 8080 for backend services (frontend stays 8660)
                    if key != "frontend" and "port" in app_config[key]:
                        app_config[key]["port"] = 8080

            infra_map = {
                "profile": {
                    "mongodbAddr": "profile-mongo",
                    "redisAddr": "profile-redis",
                },
                "rate": {"mongodbAddr": "rate-mongo", "redisAddr": "rate-redis"},
                "reservation": {
                    "mongodbAddr": "reservation-mongo",
                    "redisAddr": "reservation-redis",
                },
                "user": {"mongodbAddr": "user-mongo"},
                "review": {"mongodbAddr": "rate-mongo", "redisAddr": "rate-redis"},
            }

            for key, infra_items in infra_map.items():
                if key in app_config:
                    for addr_key, infra_name in infra_items.items():
                        if addr_key in app_config[key]:
                            full_infra_name = f"{infra_prefix}{infra_name}"
                            # Use standard ports since ComposeGenerator doesn't override infra command
                            if "mongo" in addr_key:
                                app_config[key][addr_key] = (
                                    f"mongodb://{full_infra_name}:27017"
                                )
                            elif "redis" in addr_key:
                                app_config[key][addr_key] = (
                                    f"redis://{full_infra_name}:6379"
                                )
                            elif "memcached" in addr_key:
                                app_config[key][addr_key] = (
                                    f"tcp://{full_infra_name}:11211"
                                )

            with (output_dir / "config.json").open("w") as f:
                json.dump(app_config, f, indent=2)

            # Return env vars from generator
            return deploy.env_vars

        if use_k8s:
            raise NotImplementedError(
                "Hotel app does not support Kubernetes yet (legacy path)"
            )

        # Generate project name for namespace isolation
        project_name = _safe_project_name(
            experiment_name=config.experiment_name,
            iteration=iteration,
            policy=policy,
        )

        # 1. Calculate configuration (Pure Logic)
        hotel_config_dict = create_hotel_config_dict(
            template_config=config.app_config,
            project_name=project_name,
        )

        gen_config_dict = create_gen_config_dict(
            template_config=config.gen_config,
        )

        # 2. Generate config files (Effects)
        output_dir.mkdir(parents=True, exist_ok=True)

        project_config_path = output_dir / "hotel.json"
        with project_config_path.open("w") as f:
            json.dump(hotel_config_dict, f, indent=2)

        project_gen_config_path = output_dir / "gen_config.json"
        with project_gen_config_path.open("w") as f:
            json.dump(gen_config_dict, f, indent=2)

        # 3. Generate environment variables
        env_vars = self.generate_env_vars(
            config.gen_config, config.app_config, config.app_dir
        )

        # Add image tag based on policy/features
        tag = self.get_image_tag(features=policy)
        env_vars["HOTEL_IMAGE_TAG"] = tag if tag else "latest"
        env_vars["DOCKER_COMPOSE_PROJECT_NAME"] = project_name
        env_vars["PROJECT_CONFIG_PATH"] = str(project_config_path.resolve())

        # Clean up old build logs - handled by ExpDriver now (it uses output_dir/build_logs)
        # But we might want to ensure we don't have stale ones if output_dir is reused?
        # ExpDriver doesn't explicitly clean output_dir/build_logs before build,
        # but builder might overwrite.

        return env_vars

    def get_deployment_location(
        self, output_dir: Path, use_k8s: bool, repo_root: Path
    ) -> Tuple[Path, str]:
        # Check if we generated new files
        if getattr(self, "_last_use_new_generator", False):
            if use_k8s:
                # Helm: return directory containing values.yaml and chart name
                # But DeploymentManager.start expects (app_dir, deployment_config)
                # For Helm: app_dir is chart dir, deployment_config is values file path
                return repo_root / "charts/hotel", str(output_dir / "values.yaml")
            else:
                # Compose: return output_dir and docker-compose.yaml
                return output_dir, "docker-compose.yaml"

        if use_k8s:
            raise NotImplementedError("Hotel app does not support Kubernetes yet")

        # Default app dir is config.app_dir which is passed to ExpDriver -> deployment.start
        # But here we return (deploy_root, deploy_file).
        # We need to return the directory containing the compose file.
        # DockerConfig says: compose_file="scripts/local/containers+svcs.yaml"
        # So it expects to be run from `apps/hotel`.
        return repo_root / "apps/hotel", "scripts/local/containers+svcs.yaml"

    def get_loadgen_spec(
        self,
        output_dir: Path,
        features: Optional[str],
        env_vars: dict,
        use_k8s: bool,
    ) -> TaskSpec:

        project_name = env_vars.get("DOCKER_COMPOSE_PROJECT_NAME")

        # Image
        tag = self.get_image_tag(features)
        image = f"hotel_client_bench:{tag}" if tag else "hotel_client_bench:latest"
        binary = "hotel_client_bench"

        # Network
        # Legacy uses hotel_network, New generator uses hotel-network
        if getattr(self, "_last_use_new_generator", False):
            suffix = "hotel-network"
        else:
            suffix = "hotel_network"

        network = f"{project_name}_{suffix}" if project_name else f"local_{suffix}"

        # Env Vars
        task_env = {
            "BINARY_NAME": binary,
            "LOG_LEVEL": env_vars.get("LOG_LEVEL", "info"),
        }
        task_env.update(env_vars)

        # Config mounting
        # Hotel needs gen_config.json mounted as well?
        # HotelLoadGenerator didn't mount gen_config.json explicitly in the old code?
        # Let's check `HotelLoadGenerator.run` -> `LoadGenerator.run` which DOES mount it
        # if `gen_config_path` is passed.
        # And `HotelApp.run_workload` passed `project_gen_config_path`.

        gen_config_path = output_dir / "gen_config.json"
        volumes = {}
        if gen_config_path.exists():
            volumes[str(gen_config_path)] = "/usr/gen_config.json"

        return TaskSpec(
            name=f"{project_name}-loadgen" if project_name else "hotel-loadgen",
            image=image,
            env_vars=task_env,
            network=network,
            volumes=volumes,
            cleanup=True,
            artifacts=[("/tmp/masa-load-gen/.", ".")],
        )
