"""
Main experiment orchestration logic.
"""

import logging
import os
import shutil
import time
from pathlib import Path
from typing import Optional

from .apps.base import AppPlugin
from .config import ExperimentConfig
from .docker_manager import DockerManager

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
        no_cache: bool = False
    ):
        """
        Initialize experiment runner.
        
        Args:
            app: Application plugin for app-specific behavior
            config: Experiment configuration
            repo_root: Path to repository root
            plot: Whether to generate plots after experiment
            no_cache: Whether to disable Docker cache during builds
        """
        self.app = app
        self.config = config
        self.repo_root = repo_root
        self.plot = plot
        self.no_cache = no_cache
        self.docker = DockerManager(repo_root)
        
        # Setup working directories
        self.exp_scripts_dir = config.exp_dir / "scripts"
        self.app_scripts_dir = config.app_dir / "scripts"
        self.app_local_dir = self.app_scripts_dir / "local"
    
    def run(self) -> None:
        """
        Run the full experiment.
        
        This is the main entry point that orchestrates the entire experiment workflow.
        """
        logger.info(f"Starting experiment: {self.config.experiment_name}")
        logger.info(f"Application: {self.config.app_name}")
        logger.info(f"Policies: {', '.join(self.config.policies)}")
        logger.info(f"Iterations: {self.config.get_repeats()}")
        
        # Prepare directories and backup old configs
        self._prepare_experiment()
        
        # Copy configs from input to working directories
        self._copy_configs()
        
        # Run experiment iterations
        try:
            self._run_iterations()
        finally:
            # Mark experiment as complete
            self._mark_complete()
        
        # Generate plots if requested
        if self.plot:
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
        
        # Backup gen_config.json if exists
        gen_config_path = self.exp_scripts_dir / "gen_config.json"
        if gen_config_path.exists():
            shutil.copy(gen_config_path, backup_dir)
            logger.debug(f"Backed up gen_config.json to {backup_dir}")
        
        # Backup app config if exists
        docker_config = self.app.get_docker_config()
        if docker_config.app_config_filename:
            app_config_dest = self.app_local_dir / docker_config.app_config_filename
            if app_config_dest.exists():
                shutil.copy(app_config_dest, backup_dir)
                logger.debug(f"Backed up app config to {backup_dir}")
        
        # Backup old output
        if self.config.out_dir.exists() and any(self.config.out_dir.iterdir()):
            try:
                shutil.copytree(self.config.out_dir, backup_dir / "out")
                logger.debug(f"Backed up old output to {backup_dir}")
            except Exception as e:
                logger.warning(f"Failed to backup old output: {e}")
        
        # Clear output and plot directories
        if self.config.out_dir.exists():
            for item in self.config.out_dir.iterdir():
                if item.is_dir():
                    shutil.rmtree(item)
                else:
                    item.unlink()
        
        if self.config.plot_dir.exists():
            shutil.rmtree(self.config.plot_dir)
        self.config.plot_dir.mkdir(parents=True, exist_ok=True)
        
        logger.info(f"Backed up old configs to {backup_dir}")
    
    def _copy_configs(self) -> None:
        """Copy configuration files from input to working directories."""
        logger.info("Copying configuration files")
        
        # Copy gen_config.json
        src = self.config.in_dir / "gen_config.json"
        dst = self.exp_scripts_dir / "gen_config.json"
        shutil.copy(src, dst)
        logger.debug(f"Copied {src} to {dst}")
        
        # Copy app config if present
        docker_config = self.app.get_docker_config()
        if docker_config.app_config_filename:
            src = self.config.in_dir / docker_config.app_config_filename
            dst = self.app_local_dir / docker_config.app_config_filename
            
            if src.exists():
                shutil.copy(src, dst)
                logger.debug(f"Copied {src} to {dst}")
            elif docker_config.app_config_required:
                raise FileNotFoundError(
                    f"Required app config not found: {src}"
                )
    
    def _run_iterations(self) -> None:
        """Run all experiment iterations."""
        repeats = self.config.get_repeats()
        docker_config = self.app.get_docker_config()
        
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
                output_dir.mkdir(parents=True, exist_ok=True)
                
                try:
                    # Generate environment variables
                    env_vars = self.app.generate_env_vars(
                        self.config.gen_config,
                        self.config.app_config,
                        self.config.app_dir
                    )
                    
                    # Write .env file
                    env_file = self.app_local_dir / ".env"
                    with open(env_file, "w") as f:
                        for key, value in env_vars.items():
                            f.write(f"{key}={value}\n")
                    logger.debug(f"Wrote environment variables to {env_file}")
                    
                    # Build and start Docker services
                    self.docker.build(
                        app_name=self.config.app_name,
                        app_dir=self.config.app_dir,
                        features=policy,
                        rust_log="info",
                        no_cache=self.no_cache
                    )
                    
                    self.docker.start(
                        app_dir=self.config.app_dir,
                        compose_file=docker_config.compose_file,
                        env_vars=env_vars
                    )
                    
                    # Get container names for log collection
                    container_names = self.app.get_container_names(env_vars)
                    
                    # Start streaming logs in background
                    log_threads = self.docker.stream_logs(
                        container_names=container_names,
                        output_dir=output_dir,
                        follow=True
                    )
                    
                    # Run load generator (blocking)
                    loadgen = self.app.create_load_generator()
                    loadgen.run(output_dir=output_dir, env_vars=env_vars)
                    
                    logger.info(f"Load generator completed for policy {policy}")
                    
                    # Wait a moment for logs to flush
                    time.sleep(2)
                    
                except Exception as e:
                    logger.error(f"Error during iteration {iteration}, policy {policy}: {e}")
                    raise
                finally:
                    # Stop Docker services
                    self.docker.stop(
                        app_dir=self.config.app_dir,
                        compose_file=docker_config.compose_file
                    )
                    
                    # Clean up .env file
                    env_file = self.app_local_dir / ".env"
                    if env_file.exists():
                        env_file.unlink()
    
    def _mark_complete(self) -> None:
        """Mark experiment as complete."""
        done_file = self.config.out_dir / "done"
        done_file.touch()
        logger.info(f"Marked experiment as complete: {done_file}")
    
    def _generate_plots(self) -> None:
        """Generate plots from experiment results."""
        logger.info("Generating plots")
        
        plotting_script = self.repo_root / "exp" / "common" / "scripts" / "plotting" / "plot-experiment.sh"
        
        if not plotting_script.exists():
            logger.warning(f"Plotting script not found: {plotting_script}")
            return
        
        try:
            import subprocess
            
            # Change to experiment directory for plotting
            subprocess.run(
                [str(plotting_script), self.config.experiment_name],
                cwd=self.config.exp_dir,
                check=True,
            )
            logger.info("Plots generated successfully")
        except Exception as e:
            logger.error(f"Failed to generate plots: {e}")
            raise
