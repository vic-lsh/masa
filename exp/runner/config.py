"""
Configuration loading and validation for experiments.
"""

import json
import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

logger = logging.getLogger(__name__)


@dataclass
class ExperimentConfig:
    """
    Configuration for a single experiment.
    
    Loads and validates all necessary configuration files for running an experiment.
    """
    
    experiment_name: str
    app_name: str
    repo_root: Path
    
    # Directories
    exp_dir: Path
    app_dir: Path
    in_dir: Path
    out_dir: Path
    plot_dir: Path
    
    # Configuration data
    gen_config: dict
    policies: list[str]
    app_config: Optional[dict]
    
    @classmethod
    def load(
        cls,
        experiment_name: str,
        app_name: str,
        repo_root: Path,
        app_plugin: 'AppPlugin'  # type: ignore
    ) -> 'ExperimentConfig':
        """
        Load experiment configuration from disk.
        
        Args:
            experiment_name: Name of the experiment to run
            app_name: Name of the application (hotel, synthetic)
            repo_root: Path to repository root
            app_plugin: Application plugin for loading app-specific config
            
        Returns:
            ExperimentConfig instance with all loaded configurations
            
        Raises:
            FileNotFoundError: If required configuration files are missing
            ValueError: If configuration is invalid
        """
        # Setup directory paths
        exp_dir = repo_root / "exp" / app_name
        app_dir = repo_root / "apps" / app_name
        exp_data_dir = exp_dir / "data"
        in_dir = exp_data_dir / "in" / experiment_name
        out_dir = exp_data_dir / "out" / experiment_name
        plot_dir = exp_data_dir / "plots" / experiment_name
        
        # Validate directories exist
        if not exp_dir.exists():
            raise FileNotFoundError(f"Experiment directory not found: {exp_dir}")
        
        if not app_dir.exists():
            raise FileNotFoundError(f"Application directory not found: {app_dir}")
        
        if not in_dir.exists():
            raise FileNotFoundError(
                f"Expected configuration for experiment at: {in_dir}"
            )
        
        # Load gen_config.json
        gen_config_path = in_dir / "gen_config.json"
        if not gen_config_path.exists():
            raise FileNotFoundError(f"Missing gen_config.json at: {gen_config_path}")
        
        with open(gen_config_path) as f:
            gen_config = json.load(f)
        
        logger.info(f"Loaded gen_config.json from {gen_config_path}")
        
        # Validate required fields in gen_config
        required_fields = ["Repeats", "Addr"]
        for field in required_fields:
            if field not in gen_config:
                raise ValueError(f"gen_config.json missing required field: {field}")
        
        # Load policies file
        policies_path = in_dir / "policies"
        if not policies_path.exists():
            raise FileNotFoundError(f"Missing policies file at: {policies_path}")
        
        with open(policies_path) as f:
            policies_text = f.read().strip()
            policies = policies_text.split()
        
        if not policies:
            raise ValueError("policies file is empty or contains no policies")
        
        logger.info(f"Loaded policies: {', '.join(policies)}")
        
        # Load app-specific config
        docker_config = app_plugin.get_docker_config()
        app_config = None
        
        if docker_config.app_config_filename:
            app_config_path = in_dir / docker_config.app_config_filename
            
            if not app_config_path.exists():
                raise FileNotFoundError(
                    f"App config not found at: {app_config_path}"
                )
            
            app_config = app_plugin.load_app_config(app_config_path)
            logger.info(f"Loaded app config from {app_config_path}")
        
        return cls(
            experiment_name=experiment_name,
            app_name=app_name,
            repo_root=repo_root,
            exp_dir=exp_dir,
            app_dir=app_dir,
            in_dir=in_dir,
            out_dir=out_dir,
            plot_dir=plot_dir,
            gen_config=gen_config,
            policies=policies,
            app_config=app_config,
        )
    
    def get_repeats(self) -> int:
        """Get number of experiment repetitions."""
        return self.gen_config["Repeats"]
