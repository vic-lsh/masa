"""
Synthetic application plugin.
"""

import json
import logging
import re
import shlex
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Optional

from .base import AppBuilder, AppPlugin, DockerConfig, LoadGenerator
from .utils import normalize_features_to_tag, get_docker_progress_flag

logger = logging.getLogger(__name__)


class SyntheticLoadGenerator(LoadGenerator):
    """Load generator for the synthetic benchmark application."""

    def __init__(self, features: Optional[str] = None):
        """
        Initialize load generator with optional features for image tagging.

        Args:
            features: Cargo features used to build the image
        """
        self.features = features

    def get_container_name(self) -> str:
        return "synthetic_client_bench"

    def get_network_name(self) -> str:
        return "local_synthetic_network"

    def get_image_name(self) -> str:
        tag = normalize_features_to_tag(self.features)
        if tag and tag != "latest":
            return f"synthetic_client_bench:{tag}"
        else:
            return "synthetic_client_bench:latest"

    def get_binary_name(self) -> str:
        return "synthetic_client_bench"


class SyntheticApp(AppPlugin):
    """
    Plugin for the synthetic benchmark application.
    
    The synthetic app has a simpler architecture with child services that can
    be configured with different replication strategies.
    """
    
    def get_app_name(self) -> str:
        return "synthetic"
    
    def load_app_config(self, config_path: Path) -> dict:
        """Load config.docker.json configuration file."""
        with open(config_path) as f:
            return json.load(f)
    
    def generate_env_vars(
        self,
        gen_config: dict,
        app_config: Optional[dict],
        app_dir: Path
    ) -> dict:
        """
        Generate environment variables for synthetic application.
        
        Calculates child replica counts from config and extracts frontend port.
        """
        env_vars = {}
        
        # Extract frontend port from address (e.g., "http://[::1]:8658" -> "8658")
        addr = gen_config.get("Addr", "")
        match = re.search(r':(\d+)$', addr)
        if match:
            env_vars["FRONTEND_PORT"] = match.group(1)
        else:
            raise ValueError(f"Unable to extract port from address: {addr}")
        
        if app_config:
            # Calculate replica counts
            constant_replicas = app_config.get("child_constant_replicas", 1)
            env_vars["CONSTANT_REPLICAS"] = str(constant_replicas)
            
            # Calculate presampled replicas from child_presampled_services
            presampled_services = app_config.get("child_presampled_services", [])
            presampled_replicas = sum(
                service[0] for service in presampled_services if service
            ) if presampled_services else 0
            env_vars["PRESAMPLED_REPLICAS"] = str(presampled_replicas)
            
            # Random replicas is always 1
            random_replicas = 1
            env_vars["RANDOM_REPLICAS"] = str(random_replicas)
            
            # Callgraph replicas from child_callgraph_services (each defaults to 1)
            callgraph_services = app_config.get("child_callgraph_services", [])
            callgraph_replicas = sum(
                service.get("replicas", 1) for service in callgraph_services
            )

            # Total child replicas
            child_replicas = (
                constant_replicas
                + random_replicas
                + presampled_replicas
                + callgraph_replicas
            )
            env_vars["CHILD_REPLICAS"] = str(child_replicas)
            
            # CPUs per replica
            cpus_per_replica = app_config.get("child_cpus_per_replica", 1)
            env_vars["CPUS_PER_REPLICA"] = str(cpus_per_replica)
        else:
            # Use defaults if no config provided
            env_vars["CONSTANT_REPLICAS"] = "1"
            env_vars["PRESAMPLED_REPLICAS"] = "0"
            env_vars["RANDOM_REPLICAS"] = "1"
            env_vars["CHILD_REPLICAS"] = "2"
            env_vars["CPUS_PER_REPLICA"] = "1"
        
        # Set default log level if not specified
        if "LOG_LEVEL" not in env_vars:
            env_vars["LOG_LEVEL"] = "info"
        
        return env_vars
    
    def get_docker_config(self) -> DockerConfig:
        """Return Docker configuration for synthetic application."""
        return DockerConfig(
            compose_file="scripts/local/containers+svcs.yaml",
            network_name="local_synthetic_network",
            loadgen_image_name="synthetic_client_bench:<features>",
            loadgen_binary_name="synthetic_client_bench",
            app_config_filename="config.docker.json",
        )
    
    def get_container_names(self, env_vars: dict) -> list[str]:
        """
        Get list of container names for synthetic application.
        
        Includes frontend and child service containers based on replica count.
        """
        container_names = ["synthetic_frontend"]
        
        # Get child replica count
        child_replicas = int(env_vars.get("CHILD_REPLICAS", 1))
        
        # Generate container names for each child replica
        for i in range(1, child_replicas + 1):
            container_names.append(f"local-child-service-{i}")
        
        return container_names
    
    def create_load_generator(self, features: Optional[str] = None) -> LoadGenerator:
        """
        Create a load generator instance for synthetic application.
        
        Args:
            features: Optional cargo features used to build the image
        """
        return SyntheticLoadGenerator(features=features) if features is not None else SyntheticLoadGenerator()

    def create_builder(self) -> AppBuilder:
        """
        Create a builder instance for synthetic application.
        """
        return SyntheticBuilder()

    def get_image_tag(self, features: Optional[str] = None) -> str:
        """
        Get the docker image tag for the given features.
        
        Args:
            features: Optional cargo features
            
        Returns:
            Docker image tag string
        """
        return normalize_features_to_tag(features)


class SyntheticBuilder(AppBuilder):
    """
    Build logic for the synthetic app docker images.
    
    Uses multi-stage multi-target build:
    - Stage 1 (builder): Build all binaries once
    - Stage 2 (runtime-base): Base runtime image with dependencies
    - Stage 3 (runtime): Per-binary runtime images
    
    The synthetic app requires three separate docker images:
    - synthetic_frontend:latest - frontend service
    - synthetic_child:latest - child service (can be scaled)
    - synthetic_client_bench:latest - load generator
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
    ) -> Optional[list[list[str]]]:
        app = "synthetic"
        
        # Services to build (each gets its own image)
        binaries = [
            "synthetic_frontend",
            "synthetic_child",
            "synthetic_client_bench",
        ]

        # Generate tag based on features for deterministic, feature-specific images
        tag = normalize_features_to_tag(features)

        logger.info(f"Building {len(binaries)} docker images for synthetic app using multi-stage build")
        if features:
            logger.info(f"Using features: {features}")
        
        # Collect commands if dry_run
        commands: list[list[str]] = []
        
        # Start timing the docker build
        build_start_time = time.time()
        
        if gen_config_path is None:
            raise ValueError("gen_config_path is required for synthetic app")
        gen_config_path_rel = gen_config_path.relative_to(repo_root)
        
        try:
            # Stage 1: Build all binaries once (shared across all images)
            logger.info("Stage 1: Building all binaries for synthetic app")
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
            logger.info("Stage 1 complete: All binaries built")
            
            # Stage 2: Build runtime-base (shared across all images)
            logger.info("Stage 2: Building runtime-base image")
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
            logger.info("Stage 2 complete: Runtime-base image built")
            
            # Stage 3: Build per-binary runtime images
            for binary_name in binaries:
                logger.info(f"Stage 3: Building runtime image for {binary_name}")
                
                runtime_build_args: list[str] = []
                if features:
                    runtime_build_args.extend(["--build-arg", f"FEATURES={features}"])
                runtime_build_args.extend(["--build-arg", f"LOG_LEVEL={rust_log}"])
                runtime_build_args.extend(["--build-arg", f"APP={app}"])
                runtime_build_args.extend(["--build-arg", f"GEN_CONFIG_PATH={gen_config_path_rel}"])
                runtime_build_args.extend(["--build-arg", f"BINARY_NAME={binary_name}"])
                # Use consistent cache ID based on features across all stages
                runtime_build_args.extend(["--build-arg", f"CACHE_ID={cache_id}"])

                # Image name: synthetic_<binary>:<tag> or synthetic_<binary>:latest if no features
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
                    get_docker_progress_flag(),
                ]
                
                if no_cache:
                    runtime_cmd.append("--no-cache")
                
                runtime_cmd.extend(["-t", image_name, "."])
                
                if dry_run:
                    commands.append(runtime_cmd.copy())
                else:
                    try:
                        subprocess.run(
                            runtime_cmd,
                            cwd=repo_root,
                            check=True,
                            capture_output=False,
                        )
                    except subprocess.CalledProcessError as e:
                        logger.error(f"Failed to build runtime image for {binary_name}. Command: {shlex.join(runtime_cmd)}")
                        raise
                
                logger.info(f"Successfully built docker image: {image_name}")
        
        finally:
             pass
        
        if dry_run:
            return commands
        
        # Calculate and print build duration
        build_duration = time.time() - build_start_time
        logger.info(f"Docker image building took {build_duration:.2f} seconds ({build_duration/60:.2f} minutes)")
        logger.info("All synthetic app docker images built successfully")
        return None
