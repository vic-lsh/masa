"""
Configuration loading and validation for experiments.
"""

import json
import logging
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Optional

import yaml

from .experiment_config_v2 import ExperimentConfigV2
from .legacy import convert_experiment_config_to_gen_config

if TYPE_CHECKING:
    from .apps.base import AppPlugin

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
    app_config_filename: Optional[str] = None
    config_format: str = "legacy"
    experiment_v2: Optional[ExperimentConfigV2] = None
    _generated_gen_config_path: Optional[Path] = None

    @classmethod
    def load(
        cls,
        experiment_name: str,
        app_name: str,
        repo_root: Path,
        app_plugin: "AppPlugin",  # type: ignore
        compat: bool = False,
    ) -> "ExperimentConfig":
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
        in_dir = exp_dir / "in" / experiment_name
        out_dir = exp_dir / "out" / experiment_name
        plot_dir = exp_dir / "plots" / experiment_name

        # Validate directories exist
        if not exp_dir.exists():
            raise FileNotFoundError(f"Experiment directory not found: {exp_dir}")

        if not app_dir.exists():
            raise FileNotFoundError(f"Application directory not found: {app_dir}")

        if not in_dir.exists():
            raise FileNotFoundError(
                f"Expected configuration for experiment at: {in_dir}"
            )

        config_format = "legacy" if compat else "v2"
        experiment_v2: Optional[ExperimentConfigV2] = None

        if compat:
            gen_config_path = in_dir / "gen_config.json"
            if not gen_config_path.exists():
                raise FileNotFoundError(
                    f"Missing gen_config.json at: {gen_config_path}"
                )

            with open(gen_config_path) as f:
                gen_config = json.load(f)

            policies_path = in_dir / "policies"
            if not policies_path.exists():
                raise FileNotFoundError(f"Missing policies file at: {policies_path}")

            with open(policies_path) as f:
                policies = [line.strip() for line in f if line.strip()]
        else:
            experiment_yaml = in_dir / "experiment.yaml"
            if not experiment_yaml.exists():
                raise FileNotFoundError(
                    f"Missing experiment.yaml at: {experiment_yaml}. "
                    "Use --compat for legacy gen_config.json/policies input or run "
                    "'uv run -m exp_runner migrate-config ...' to migrate."
                )

            experiment_v2 = ExperimentConfigV2.from_yaml(experiment_yaml)
            if experiment_v2.app and experiment_v2.app != app_name:
                raise ValueError(
                    f"experiment.yaml app='{experiment_v2.app}' does not match '{app_name}'"
                )

            frontend_defaults = {
                "hotel": "http://frontend:8660",
                "socialnet": "http://compose-post-service:8080",
                "synthetic": "http://synthetic-frontend-service:8000",
                "mssim": "http://orchestrator:50051",
            }
            gen_config = convert_experiment_config_to_gen_config(
                experiment_v2,
                frontend_addr=frontend_defaults.get(app_name, "http://frontend:8080"),
            )
            policies = experiment_v2.execution.policies

        logger.info(f"Loaded {config_format} experiment config for {experiment_name}")

        # Validate required fields in gen_config (app-specific)
        required_fields = app_plugin.get_required_gen_config_fields()
        for field in required_fields:
            if field not in gen_config:
                raise ValueError(f"config missing required field: {field}")

        if not policies:
            raise ValueError("No policies configured")

        logger.info(f"Loaded policies: {', '.join(policies)}")

        # Load app-specific config
        docker_config = app_plugin.get_docker_config()
        app_config = None

        if docker_config.app_config_filename:
            app_config_path = in_dir / docker_config.app_config_filename

            if not app_config_path.exists():
                raise FileNotFoundError(f"App config not found at: {app_config_path}")

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
            app_config_filename=docker_config.app_config_filename,
            config_format=config_format,
            experiment_v2=experiment_v2,
        )

    def get_repeats(self) -> int:
        """Get number of experiment repetitions."""
        return self.gen_config["Repeats"]

    def get_gen_config_path(self) -> Path:
        """
        Return a filesystem path to gen_config.json for build tooling.
        """
        legacy_path = self.in_dir / "gen_config.json"
        if legacy_path.exists():
            return legacy_path

        if self._generated_gen_config_path is None:
            gen_config_path = self.in_dir / ".generated.gen_config.json"
            gen_config_path.write_text(
                json.dumps(self.gen_config, indent=2),
                encoding="utf-8",
            )
            self._generated_gen_config_path = gen_config_path
        return self._generated_gen_config_path

    def write_plot_compat_inputs(self, target_dir: Path) -> None:
        """
        Materialize legacy plotting inputs in target_dir.
        """
        target_dir.mkdir(parents=True, exist_ok=True)
        (target_dir / "gen_config.json").write_text(
            json.dumps(self.gen_config, indent=2),
            encoding="utf-8",
        )
        (target_dir / "policies").write_text(
            "\n".join(self.policies) + "\n",
            encoding="utf-8",
        )
        if self.experiment_v2 is not None:
            (target_dir / "experiment.yaml").write_text(
                yaml.safe_dump(self.experiment_v2.to_dict(), sort_keys=False),
                encoding="utf-8",
            )

        if self.app_config_filename and self.app_config is not None:
            (target_dir / self.app_config_filename).write_text(
                json.dumps(self.app_config, indent=2),
                encoding="utf-8",
            )
