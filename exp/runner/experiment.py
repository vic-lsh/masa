"""
Main experiment orchestration logic.
"""

import logging
import os
import shutil
import time
from argparse import Namespace
from pathlib import Path
from typing import Optional

from .apps.base import AppPlugin
from .config import ExperimentConfig
from .docker_manager import DockerManager
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
        """
        self.app = app
        self.config = config
        self.repo_root = repo_root
        self.plot = plot
        self.no_cache = no_cache
        self.rm_data = rm_data
        self.dry_run = dry_run
        self.docker = DockerManager(repo_root)
        
        # Setup working directories
        self.exp_scripts_dir = config.exp_dir / "scripts"
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
            # Copy configs from input to working directories
            self._copy_configs()
        
        # Run experiment iterations
        self._run_iterations()

        # Mark experiment as complete
        if not self.dry_run:
            self._mark_complete()
        
        # Generate plots if requested
        if self.plot and not self.dry_run:
            self._generate_plots()
        
        logger.info(f"Experiment {self.config.experiment_name} completed successfully")
    
    def _prepare_experiment(self) -> None:
        """Prepare directories and backup old configurations."""
        logger.info("Preparing experiment directories")
        
        # Create necessary directories
        self.exp_scripts_dir.mkdir(parents=True, exist_ok=True)
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
                logger.info(f"Removed existing data from output directory: {self.config.out_dir}")
        else:
            logger.debug(f"Keeping existing data in output directory: {self.config.out_dir}")
        
        # Note: plot directory is only cleared when actually generating plots
        # (see _generate_plots method)
        
        logger.info(f"Backed up old configs to {backup_dir}")
    
    def _copy_configs(self) -> None:
        """Copy configuration files from input to working directories."""
        logger.info("Copying configuration files")
        
        # Note: gen_config.json and app config files are no longer copied to working directories.
        # They are passed directly to Docker build via GEN_CONFIG_PATH.
    
    def _run_iterations(self) -> None:
        """Run all experiment iterations."""
        repeats = self.config.get_repeats()
        
        # Set environment variables for app directories
        os.environ["MASA_APP_DIR"] = str(self.config.app_dir)
        os.environ["MASA_APP_NAME"] = self.config.app_name
        
        for iteration in range(repeats):
            logger.info(f"{'='*60}")
            logger.info(f"Iteration {iteration}")
            logger.info(f"{'='*60}")
            
            for policy in self.config.policies:
                logger.info(f"{'*'*50}")
                logger.info(f"Policy: {policy}")
                logger.info(f"{'*'*50}")
                
                # Setup output directory for this iteration/policy
                output_dir = self.config.out_dir / str(iteration) / policy
                if not self.dry_run:
                    output_dir.mkdir(parents=True, exist_ok=True)
                self.current_output_dir = output_dir
                try:
                    self.app.run_workload(
                        repo_root=self.repo_root,
                        config=self.config,
                        docker=self.docker,
                        policy=policy,
                        iteration=iteration,
                        output_dir=output_dir,
                        app_local_dir=self.app_local_dir,
                        no_cache=self.no_cache,
                        dry_run=self.dry_run,
                    )
                except Exception as e:
                    logger.error(f"Error during iteration {iteration}, policy {policy}: {e}")
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
        log_files = sorted(output_dir.rglob("*.log"))
        if not log_files:
            logger.warning(f"No log files found in {output_dir}")
            return

        # Avoid dumping huge numbers of logs on failure
        max_logs = 10
        if len(log_files) > max_logs:
            log_files = log_files[:max_logs]
        
        logger.error("=" * 80)
        logger.error(f"Tail of relevant logs from {output_dir}:")
        logger.error("=" * 80)
        
        for log_file in log_files:
            try:
                with open(log_file, "r", encoding="utf-8", errors="ignore") as f:
                    lines = f.readlines()
                    tail_lines = lines[-num_lines:] if len(lines) > num_lines else lines
                    
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
