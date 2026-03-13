"""
Main experiment orchestration logic.
"""

import collections
import logging
import os
import shutil
import time
from argparse import Namespace
from pathlib import Path
from typing import Optional

from .apps.base import AppPlugin
from .config import ExperimentConfig
from .deployment_manager import DeploymentManager
from .docker_manager import DockerManager
from .executor import CommandExecutor, MockCommandExecutor, SubprocessExecutor
from .k8s_manager import K8sManager
from .plotting import generate_all_plots

logger = logging.getLogger(__name__)


class Experiment:
    """
    Orchestrates the execution of a performance experiment.

    An experiment consists of multiple iterations, each testing different
    scheduling policies. For each policy, Docker services are built and started,
    a load generator is run, and logs are collected.
    """

    def __init__(
        self,
        app: AppPlugin,
        config: ExperimentConfig,
        repo_root: Path,
        plot: bool = False,
        no_cache: bool = False,
        rm_data: bool = False,
        dry_run: bool = False,
        smoke_test: bool = False,
        use_k8s: bool = False,
    ):
        """
        Initialize experiment runner.

        Args:
            app: Application plugin for app-specific behavior
            config: Experiment configuration
            repo_root: Path to repository root
            plot: Whether to generate plots after experiment
            no_cache: Whether to disable Docker cache during builds
            rm_data: Whether to remove existing data from output directory before running
            smoke_test: Whether to verify results after execution (e.g. check goodput)
            use_k8s: Whether to use Kubernetes instead of Docker Compose
        """
        self.app = app
        self.config = config
        self.repo_root = repo_root
        self.plot = plot
        self.no_cache = no_cache
        self.rm_data = rm_data
        self.dry_run = dry_run
        self.smoke_test = smoke_test
        self.use_k8s = use_k8s

        self.executor: CommandExecutor
        if self.dry_run:
            self.executor = MockCommandExecutor()
        else:
            self.executor = SubprocessExecutor()

        self.deployment: DeploymentManager
        if use_k8s:
            self.deployment = K8sManager(repo_root, executor=self.executor)
        else:
            self.deployment = DockerManager(repo_root, executor=self.executor)

        # Setup working directories
        self.app_scripts_dir = config.app_dir / "scripts"
        self.app_local_dir = self.app_scripts_dir / "local"

        # Track current output directory for error reporting
        self.current_output_dir: Optional[Path] = None

    def run(self) -> None:
        """
        Run the full experiment.

        This is the main entry point that orchestrates the entire experiment workflow.
        """
        logger.info(f"Starting experiment: {self.config.experiment_name}")
        logger.info(f"Application: {self.config.app_name}")
        logger.info(f"Policies: {', '.join(self.config.policies)}")
        logger.info(f"Iterations: {self.config.get_repeats()}")
        if self.dry_run:
            logger.info("Dry-run mode enabled (no commands will be executed)")

        if not self.dry_run:
            # Prepare directories and backup old configs
            self._prepare_experiment()

        # Run experiment iterations
        self._run_iterations()

        # Mark experiment as complete
        if not self.dry_run:
            self._mark_complete()

        # Verify results if requested (smoke test)
        if self.smoke_test and not self.dry_run:
            logger.info("Running smoke test verification...")
            if not self.app.verify_results(self.config):
                raise RuntimeError("Smoke test verification failed")

        # Generate plots if requested
        if self.plot and not self.dry_run:
            self._generate_plots()

        logger.info(f"Experiment {self.config.experiment_name} completed successfully")

    def _prepare_experiment(self) -> None:
        """Prepare directories and backup old configurations."""
        logger.info("Preparing experiment directories")

        # Create necessary directories
        self.app_local_dir.mkdir(parents=True, exist_ok=True)
        self.config.out_dir.mkdir(parents=True, exist_ok=True)

        # Backup old configs
        curr_ts = int(time.time())
        backup_dir = Path(f"/tmp/masa-save-{curr_ts}")
        backup_dir.mkdir(parents=True, exist_ok=True)

        # Note: gen_config.json is no longer stored in exp_scripts_dir,
        # so we don't need to backup it from there.

        # Note: App config files are no longer stored in apps/ directory,
        # so we don't need to backup them from there.

        # Backup old output
        if self.config.out_dir.exists() and any(self.config.out_dir.iterdir()):
            try:
                shutil.copytree(self.config.out_dir, backup_dir / "out")
                logger.debug(f"Backed up old output to {backup_dir}")
            except Exception as e:
                logger.warning(f"Failed to backup old output: {e}")

        # Clear output directory only if rm_data flag is set
        if self.rm_data:
            if self.config.out_dir.exists():
                for item in self.config.out_dir.iterdir():
                    if item.is_dir():
                        shutil.rmtree(item)
                    else:
                        item.unlink()
                logger.info(
                    f"Removed existing data from output directory: {self.config.out_dir}"
                )
        else:
            logger.debug(
                f"Keeping existing data in output directory: {self.config.out_dir}"
            )

        # Note: plot directory is only cleared when actually generating plots
        # (see _generate_plots method)

        logger.info(f"Backed up old configs to {backup_dir}")

    def _run_iterations(self) -> None:
        """Run all experiment iterations."""
        repeats = self.config.get_repeats()

        # Set environment variables for app directories
        os.environ["MASA_APP_DIR"] = str(self.config.app_dir)
        os.environ["MASA_APP_NAME"] = self.config.app_name

        for iteration in range(repeats):
            logger.info(f"{'=' * 60}")
            logger.info(f"Iteration {iteration}")
            logger.info(f"{'=' * 60}")

            for policy in self.config.policies:
                logger.info(f"{'*' * 50}")
                logger.info(f"Policy: {policy}")
                logger.info(f"{'*' * 50}")

                # Setup output directory for this iteration/policy
                output_dir = self.config.out_dir / str(iteration) / policy
                if not self.dry_run:
                    output_dir.mkdir(parents=True, exist_ok=True)
                self.current_output_dir = output_dir
                try:
                    self.app.run_workload(
                        repo_root=self.repo_root,
                        config=self.config,
                        deployment=self.deployment,
                        policy=policy,
                        iteration=iteration,
                        output_dir=output_dir,
                        app_local_dir=self.app_local_dir,
                        no_cache=self.no_cache,
                        dry_run=self.dry_run,
                        executor=self.executor,
                    )
                except Exception as e:
                    logger.error(
                        f"Error during iteration {iteration}, policy {policy}: {e}"
                    )
                    if os.environ.get("CI") == "true":
                        self._print_log_tails(output_dir)
                    raise

    def _print_log_tails(self, output_dir: Path, num_lines: int = 50) -> None:
        """
        Print the tail of all log files in the output directory.

        Args:
            output_dir: Directory containing log files
            num_lines: Number of lines to print from each log file
        """
        if not output_dir.exists():
            logger.warning(f"Output directory does not exist: {output_dir}")
            return

        # Include nested logs (e.g., MSSIM stores orchestrator.log under per-RPS subdirectories)
        log_files = list(output_dir.rglob("*.log"))
        if not log_files:
            logger.warning(f"No log files found in {output_dir}")
            return

        # Sort logs by modification time in reverse order (newest first)
        log_files.sort(
            key=lambda p: os.path.getmtime(p) if p.exists() else 0, reverse=True
        )

        # Avoid dumping huge numbers of logs on failure
        max_logs = 10
        if len(log_files) > max_logs:
            log_files = log_files[:max_logs]

        logger.error("=" * 80)
        logger.error(f"Tail of relevant logs from {output_dir} (newest first):")
        logger.error("=" * 80)

        for log_file in log_files:
            try:
                with open(log_file, "r", encoding="utf-8", errors="ignore") as f:
                    # Use deque to efficiently read only the last N lines without loading entire file into memory
                    tail_lines = collections.deque(f, maxlen=num_lines)

                    logger.error("")
                    logger.error(f"--- {log_file} (last {len(tail_lines)} lines) ---")
                    for line in tail_lines:
                        # Remove trailing newline to avoid double newlines in logging
                        logger.error(line.rstrip())
            except Exception as e:
                logger.warning(f"Failed to read log file {log_file}: {e}")

        logger.error("=" * 80)

    def _mark_complete(self) -> None:
        """Mark experiment as complete."""
        done_file = self.config.out_dir / "done"
        done_file.touch()
        logger.info(f"Marked experiment as complete: {done_file}")

    def _generate_plots(self) -> None:
        """Generate plots from experiment results."""
        logger.info("Generating plots")

        # Clear plot directory before generating new plots
        if self.config.plot_dir.exists():
            shutil.rmtree(self.config.plot_dir)
        self.config.plot_dir.mkdir(parents=True, exist_ok=True)
        logger.debug(f"Cleared plot directory: {self.config.plot_dir}")

        # Create args-like object for plotting functions
        args = Namespace(
            config_dir=self.config.in_dir,
            data_dir=self.config.out_dir,
            output_dir=self.config.plot_dir,
        )

        try:
            generate_all_plots(args)
            logger.info("Plots generated successfully")
        except Exception as e:
            logger.error(f"Failed to generate plots: {e}")
            raise
