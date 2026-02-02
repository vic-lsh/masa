"""
Tests for the MSSIM application plugin.
"""

import json
import tempfile
from pathlib import Path

from exp_runner.runner.apps import get_app_plugin
from exp_runner.runner.config import ExperimentConfig


def test_get_app_plugin_mssim() -> None:
    app = get_app_plugin("mssim")
    assert app.get_app_name() == "mssim"


def test_experiment_config_load_mssim_minimal() -> None:
    app = get_app_plugin("mssim")

    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir) / "repo"
        repo_root.mkdir()

        # Minimal repo layout required by ExperimentConfig.load
        (repo_root / "apps" / "mssim").mkdir(parents=True)
        in_dir = repo_root / "exp" / "mssim" / "data" / "in" / "e2e_test"
        in_dir.mkdir(parents=True)

        (in_dir / "policies").write_text("fifo\n", encoding="utf-8")
        (in_dir / "gen_config.json").write_text(
            json.dumps({"Repeats": 1, "Rps": [200], "DurationSecs": 1, "WarmupSecs": 0}),
            encoding="utf-8",
        )
        (in_dir / "mssim.json").write_text(
            json.dumps(
                {
                    "trace_dir": "trace-analysis/golden/S_86516878",
                    "config_dir": "apps/mssim/simulator/example_config/",
                    "slo_ms": 100,
                    "extra_env": {},
                }
            ),
            encoding="utf-8",
        )

        cfg = ExperimentConfig.load(
            experiment_name="e2e_test",
            app_name="mssim",
            repo_root=repo_root,
            app_plugin=app,
        )

        assert cfg.app_name == "mssim"
        assert cfg.experiment_name == "e2e_test"
        assert cfg.policies == ["fifo"]
        assert cfg.gen_config["Rps"] == [200]
        assert isinstance(cfg.app_config, dict)
