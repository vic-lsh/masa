"""
Unified build orchestrator for all apps.

Uses the shared multi-stage Dockerfile to build images for any app.
Replaces app-specific build logic with a common pattern.
"""

import logging
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import TYPE_CHECKING, Optional

logger = logging.getLogger(__name__)

if TYPE_CHECKING:  # pragma: no cover
    from .apps.base import AppPlugin
    from .executor import CommandExecutor


class BuildOrchestrator:
    """
    Builds Docker images for any app using the shared multi-stage Dockerfile.

    The Dockerfile (exp_runner/common/docker-build/Dockerfile) expects these build args:
    - APP: cargo package name (e.g., "hotel")
    - FEATURES: cargo features (e.g., "prio_global,early")
    - BINARY_NAME: which binary to copy into runtime image
    - LOG_LEVEL: rust log level
    - GEN_CONFIG_PATH: path to gen_config.json

    Build process:
    1. Stage 1 (`builder`): Build all binaries (shared, cached)
    2. Stage 2 (`runtime-base`): Runtime base with shared dependencies (cached)
    3. Stage 3 (`runtime`): Per-binary runtime images (parallel)
    """

    def __init__(
        self,
        repo_root: Path,
        dockerfile_path: Optional[Path] = None,
        executor: Optional["CommandExecutor"] = None,
    ):
        """
        Initialize build orchestrator.

        Args:
            repo_root: Repository root path
            dockerfile_path: Path to Dockerfile (defaults to common/docker-build/Dockerfile)
            executor: Command executor (defaults to SubprocessExecutor)
        """
        self.repo_root = repo_root

        if dockerfile_path is None:
            self.dockerfile_path = (
                repo_root / "exp_runner" / "common" / "docker-build" / "Dockerfile"
            )
        else:
            self.dockerfile_path = dockerfile_path

        if executor is None:
            from .executor import SubprocessExecutor

            self.executor: "CommandExecutor" = SubprocessExecutor()
        else:
            self.executor = executor

    def build(
        self,
        app: "AppPlugin",
        features: Optional[str],
        gen_config_path: Path,
        rust_log: str = "info",
        no_cache: bool = False,
        dry_run: bool = False,
    ) -> None:
        """
        Build all images for an application.

        Args:
            app: Application plugin
            features: Cargo features (e.g., "prio_global,early")
            gen_config_path: Path to gen_config.json
            rust_log: Rust log level
            no_cache: Whether to disable Docker cache
            dry_run: If True, only print commands without executing
        """
        binaries = app.get_binaries()
        package = app.get_cargo_package()
        parallelism = app.get_build_parallelism()

        logger.info(f"Building {len(binaries)} images for {package}")
        logger.info(f"Features: {features or 'none'}")
        logger.info(f"Parallelism: {parallelism}")

        # Build args common to all stages
        common_build_args = {
            "APP": package,
            "LOG_LEVEL": rust_log,
        }

        if features:
            common_build_args["FEATURES"] = features

        # Stage 1: Build all binaries (shared, cached)
        self._build_stage(
            "builder",
            build_args=common_build_args,
            no_cache=no_cache,
            dry_run=dry_run,
        )

        # Stage 2: Runtime base (shared, cached)
        runtime_base_args = {
            **common_build_args,
            "GEN_CONFIG_PATH": str(gen_config_path),
        }
        self._build_stage(
            "runtime-base",
            build_args=runtime_base_args,
            no_cache=no_cache,
            dry_run=dry_run,
        )

        # Stage 3: Per-binary runtime images (parallel)
        if parallelism > 1:
            with ThreadPoolExecutor(max_workers=parallelism) as executor:
                futures = []
                for binary in binaries:
                    future = executor.submit(
                        self._build_binary_image,
                        binary,
                        common_build_args,
                        no_cache,
                        dry_run,
                    )
                    futures.append(future)

                # Wait for all builds to complete
                for future in futures:
                    future.result()
        else:
            # Sequential build
            for binary in binaries:
                self._build_binary_image(binary, common_build_args, no_cache, dry_run)

        logger.info(f"Build completed for {package}")

    def _build_stage(
        self,
        target: str,
        build_args: dict[str, str],
        no_cache: bool,
        dry_run: bool,
    ) -> None:
        """Build a specific stage of the Dockerfile."""
        cmd = ["docker", "buildx", "build"]

        if no_cache:
            cmd.append("--no-cache")

        cmd.extend(["--target", target])

        for key, value in build_args.items():
            cmd.extend(["--build-arg", f"{key}={value}"])

        cmd.extend(["-f", str(self.dockerfile_path)])
        cmd.append(str(self.repo_root))

        logger.debug(f"Building stage '{target}': {' '.join(cmd)}")

        if not dry_run:
            self.executor.execute(cmd)

    def _build_binary_image(
        self,
        binary_name: str,
        base_build_args: dict[str, str],
        no_cache: bool,
        dry_run: bool,
    ) -> None:
        """Build a runtime image for a specific binary."""
        build_args = {
            **base_build_args,
            "BINARY_NAME": binary_name,
        }

        # Determine image tag
        features = base_build_args.get("FEATURES", "")
        if features:
            tag = f"{binary_name}:{features}"
        else:
            tag = f"{binary_name}:latest"

        cmd = ["docker", "buildx", "build"]

        if no_cache:
            cmd.append("--no-cache")

        cmd.extend(["--target", "runtime"])

        for key, value in build_args.items():
            cmd.extend(["--build-arg", f"{key}={value}"])

        cmd.extend(["-t", tag])
        cmd.extend(["-f", str(self.dockerfile_path)])
        cmd.append(str(self.repo_root))

        logger.info(f"Building image: {tag}")
        logger.debug(f"Command: {' '.join(cmd)}")

        if not dry_run:
            self.executor.execute(cmd)
