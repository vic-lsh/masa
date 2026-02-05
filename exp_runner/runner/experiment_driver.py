import logging
import time
from pathlib import Path

from .apps.base import AppPlugin
from .config import ExperimentConfig
from .cpu_monitor import CPUMonitor
from .deployment_manager import DeploymentManager

logger = logging.getLogger(__name__)


class ExpDriver:
    """
    Orchestrates a single experiment workload execution.
    Platform-agnostic (works for Docker and K8s).
    """

    def __init__(self, app: AppPlugin, deployment: DeploymentManager):
        self.app = app
        self.deployment = deployment

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
            builder.build(
                repo_root=repo_root,
                app_dir=config.app_dir,
                features=policy,
                rust_log=env_vars.get("LOG_LEVEL", "info"),
                no_cache=no_cache,
                gen_config_path=template_gen_config,
                dry_run=True,
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
            cpu_monitor = CPUMonitor(
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
