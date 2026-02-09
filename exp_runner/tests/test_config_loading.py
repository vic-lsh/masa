"""Tests for ExperimentConfig loading modes."""

import json
import tempfile
from pathlib import Path

import yaml

from exp_runner.runner.apps import get_app_plugin
from exp_runner.runner.config import ExperimentConfig
from exp_runner.runner.experiment_config_v2 import (
    ApiSpec,
    ExperimentConfigV2,
    ExecutionSpec,
    LoadGenSpec,
)


def _write_minimal_layout(repo_root: Path, app_name: str, exp_name: str) -> Path:
    (repo_root / "apps" / app_name).mkdir(parents=True)
    in_dir = repo_root / "exp" / app_name / "in" / exp_name
    in_dir.mkdir(parents=True)
    return in_dir


def test_load_v2_config_by_default() -> None:
    app = get_app_plugin("synthetic")

    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir) / "repo"
        in_dir = _write_minimal_layout(repo_root, "synthetic", "v2_exp")

        exp_v2 = ExperimentConfigV2(
            name="v2_exp",
            app="synthetic",
            execution=ExecutionSpec(
                repeats=1,
                policies=["fifo", "prio_global"],
                warmup_secs=1,
                duration_secs=2,
            ),
            loadgen=LoadGenSpec(
                rps=[100],
                apis=[ApiSpec(name="a", slo_us=100000)],
            ),
        )
        (in_dir / "experiment.yaml").write_text(
            yaml.safe_dump(exp_v2.to_dict(), sort_keys=False),
            encoding="utf-8",
        )
        (in_dir / "config.docker.json").write_text("{}", encoding="utf-8")

        cfg = ExperimentConfig.load(
            experiment_name="v2_exp",
            app_name="synthetic",
            repo_root=repo_root,
            app_plugin=app,
        )

        assert cfg.config_format == "v2"
        assert cfg.policies == ["fifo", "prio_global"]
        assert cfg.gen_config["Rps"] == [100]


def test_load_legacy_requires_compat() -> None:
    app = get_app_plugin("synthetic")

    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir) / "repo"
        in_dir = _write_minimal_layout(repo_root, "synthetic", "legacy_exp")
        (in_dir / "gen_config.json").write_text(
            json.dumps(
                {
                    "Repeats": 1,
                    "Rps": [100],
                    "Apis": ["a"],
                    "Slos": [100000],
                    "Timeouts_ms": [1000],
                    "WarmupSecs": 0,
                    "DurationSecs": 1,
                    "Addr": "http://synthetic-frontend-service:8000",
                }
            ),
            encoding="utf-8",
        )
        (in_dir / "policies").write_text("fifo\n", encoding="utf-8")
        (in_dir / "config.docker.json").write_text("{}", encoding="utf-8")

        try:
            ExperimentConfig.load(
                experiment_name="legacy_exp",
                app_name="synthetic",
                repo_root=repo_root,
                app_plugin=app,
            )
            assert False, "Expected FileNotFoundError"
        except FileNotFoundError as exc:
            assert "--compat" in str(exc)

        cfg = ExperimentConfig.load(
            experiment_name="legacy_exp",
            app_name="synthetic",
            repo_root=repo_root,
            app_plugin=app,
            compat=True,
        )
        assert cfg.config_format == "legacy"
        assert cfg.policies == ["fifo"]
