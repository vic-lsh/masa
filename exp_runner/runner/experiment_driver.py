import logging
import os
import time
from pathlib import Path
from typing import Callable, Optional, Type

from .apps.base import AppPlugin
from .config import ExperimentConfig
from .cpu_monitor import CPUMonitor
from .deployment_manager import DeploymentManager
from .executor import CommandExecutor, SubprocessExecutor

logger = logging.getLogger(__name__)


class ExpDriver:
    """
    Orchestrates a single experiment workload execution.
    Platform-agnostic (works for Docker and K8s).
    """

    def __init__(
        self,
        app: AppPlugin,
        deployment: DeploymentManager,
        executor: Optional[CommandExecutor] = None,
        cpu_monitor_factory: Optional[Type[CPUMonitor]] = None,
    ):
        self.app = app
        self.deployment = deployment
        self.executor = executor or SubprocessExecutor()
        self.cpu_monitor_factory = cpu_monitor_factory or CPUMonitor

    def run_workload(
        self,
        config: ExperimentConfig,
        policy: str,
        iteration: int,
        output_dir: Path,
        repo_root: Path,
        no_cache: bool = False,
        dry_run: bool = False,
    ) -> None:
        """
        Execute a single workload (setup -> build -> deploy -> run -> teardown).
        """

        # Determine platform
        use_k8s = hasattr(self.deployment, "kube_context")

        # 1. Setup & Configuration
        output_dir.mkdir(parents=True, exist_ok=True)

        # Prepare workload (generate configs, env vars)
        env_vars = self.app.prepare_workload(
            config=config,
            policy=policy,
            iteration=iteration,
            output_dir=output_dir,
            repo_root=repo_root,
            use_k8s=use_k8s,
        )

        project_name = env_vars.get("DOCKER_COMPOSE_PROJECT_NAME", "")

        if not project_name and not dry_run:
            logger.warning("No DOCKER_COMPOSE_PROJECT_NAME in env_vars")

        # 2. Build Images
        template_gen_config = config.in_dir / "gen_config.json"
        builder = self.app.create_builder()
        build_logs_dir = output_dir / "build_logs"

        if dry_run:
            # For dry-run, we rely on the executor (MockCommandExecutor) to record commands
            # We still pass dry_run=True for compatibility with builders that might use it
            # to skip side effects not captured by executor (like file I/O).
            builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log=env_vars.get("LOG_LEVEL", "info"),
                no_cache=no_cache,
                gen_config_path=template_gen_config,
                dry_run=True,
                executor=self.executor,
            )
        else:
            builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log=env_vars.get("LOG_LEVEL", "info"),
                no_cache=no_cache,
                gen_config_path=template_gen_config,
                dry_run=False,
                build_logs_dir=build_logs_dir,
                executor=self.executor,
            )

        # 3. Deploy
        deploy_root, deploy_file = self.app.get_deployment_location(
            output_dir, use_k8s, repo_root
        )

        if dry_run:
            logger.info(
                f"[dry-run] Would deploy from {deploy_root}/{deploy_file} with project={project_name}"
            )
            return

        # Load images into Kind if needed (K8s mode)
        if use_k8s and hasattr(self.deployment, "load_image_to_cluster"):
            kind_cluster_name = os.environ.get("KIND_CLUSTER_NAME")
            if kind_cluster_name:
                images = self.app.get_required_images(policy)
                if images:
                    self.deployment.load_image_to_cluster(kind_cluster_name, images)
            else:
                logger.debug("KIND_CLUSTER_NAME not set, skipping kind load")

        cpu_monitor = None

        try:
            # Start services
            self.deployment.start(
                app_dir=deploy_root,
                deployment_config=deploy_file,
                env_vars=env_vars,
                project_name=project_name,
            )

            # Allow stabilization
            time.sleep(5)

            # Start Monitoring
            container_names = self.deployment.get_container_names(
                config_path=deploy_root / deploy_file,
                project_name=project_name,
                env_vars=env_vars,
            )

            if not container_names:
                logger.warning("No containers found to monitor/log.")

            # CPU Monitor
            cpu_stats_file = output_dir / "cpu_stats.csv"
            cpu_monitor = self.cpu_monitor_factory(
                output_path=cpu_stats_file,
                poll_interval=2.0,
                container_names=container_names,
            )
            cpu_monitor.start()

            # Log Streaming
            logs_dir = output_dir / "logs"
            self.deployment.stream_logs(container_names, logs_dir, follow=True)

            # 4. Execute Task (Load Generator)
            loadgen_spec = self.app.get_loadgen_spec(
                output_dir=output_dir,
                features=policy,
                env_vars=env_vars,
                use_k8s=use_k8s,
            )

            # Disable auto-cleanup so we can copy artifacts
            original_cleanup = loadgen_spec.cleanup
            loadgen_spec.cleanup = False

            try:
                loadgen_log = output_dir / "loadgen.log"
                self.deployment.run_task(loadgen_spec, log_file=loadgen_log)

                # 5. Collect Artifacts
                for src, dst in loadgen_spec.artifacts:
                    try:
                        self.deployment.copy_from_container(
                            loadgen_spec.name, src, output_dir / dst
                        )
                    except Exception as e:
                        logger.warning(f"Failed to copy artifact {src} -> {dst}: {e}")
                        # Fallback: try to extract from logs if use_k8s
                        if use_k8s and loadgen_log.exists():
                            logger.info("Attempting to extract artifacts from logs...")
                            self._extract_artifacts_from_log(
                                loadgen_log, output_dir / dst
                            )

            finally:
                if original_cleanup:
                    self.deployment.cleanup_task(loadgen_spec)

        except Exception as e:
            logger.error(f"Workload execution failed: {e}")
            failed = self.deployment.check_project_health(
                deploy_root / deploy_file, project_name, env_vars
            )
            if failed:
                logger.error(f"Failed containers: {failed}")
            raise
        finally:
            # Teardown
            if cpu_monitor:
                try:
                    cpu_monitor.stop()
                except Exception as e:
                    logger.warning(f"Failed to stop CPU monitor: {e}")

            try:
                self.deployment.stop(
                    app_dir=deploy_root,
                    deployment_config=deploy_file,
                    env_vars=env_vars,
                    project_name=project_name,
                )
            except Exception as e:
                logger.warning(f"Failed to stop deployment: {e}")

    def _extract_artifacts_from_log(self, log_file: Path, output_dir: Path) -> None:
        """
        Extract artifacts from log file.

        The log file is expected to contain output in the format:
        ---BEGIN TRACES---
        FILE: filename
        content
        ---END FILE---
        ...
        ---END TRACES---
        """
        if not log_file.exists():
            logger.warning(
                f"Log file {log_file} does not exist, cannot extract artifacts."
            )
            return

        output_dir.mkdir(parents=True, exist_ok=True)

        try:
            with open(log_file, "r", encoding="utf-8", errors="replace") as f:
                content = f.read()

            if "---BEGIN TRACES---" not in content:
                logger.warning("No traces found in log file.")
                return

            traces_block = content.split("---BEGIN TRACES---")[1].split(
                "---END TRACES---"
            )[0]

            import re

            # Pattern: FILE: (.*)\n(.*?)---END FILE---
            # Use DOTALL to match newlines
            pattern = re.compile(r"FILE: (.*?)\n(.*?)---END FILE---", re.DOTALL)

            count = 0
            for match in pattern.finditer(traces_block):
                filename = match.group(1).strip()
                file_content = match.group(2)

                # Check if filename is just a name or path
                filename = os.path.basename(filename)

                out_path = output_dir / filename
                with open(out_path, "w", encoding="utf-8") as out_f:
                    out_f.write(file_content)
                count += 1

            logger.info(f"Extracted {count} artifacts from log to {output_dir}.")

        except Exception as e:
            logger.warning(f"Failed to extract artifacts from log: {e}")
