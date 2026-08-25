"""
Tests for the Tracebench application plugin.
"""

import json
import tempfile
from pathlib import Path

from exp_runner.runner.apps import get_app_plugin
from exp_runner.runner.config import ExperimentConfig
from exp_runner.runner.plotting.tracebench import _compute_goodput

import pandas as pd


def test_get_app_plugin_tracebench() -> None:
    app = get_app_plugin("tracebench")
    assert app.get_app_name() == "tracebench"


def test_experiment_config_load_tracebench_minimal() -> None:
    app = get_app_plugin("tracebench")

    with tempfile.TemporaryDirectory() as tmpdir:
        repo_root = Path(tmpdir) / "repo"
        repo_root.mkdir()

        # Minimal repo layout required by ExperimentConfig.load
        (repo_root / "apps" / "tracebench").mkdir(parents=True)
        in_dir = repo_root / "exp" / "tracebench" / "in" / "e2e_test"
        in_dir.mkdir(parents=True)

        (in_dir / "policies").write_text("sched_fifo\n", encoding="utf-8")
        (in_dir / "gen_config.json").write_text(
            json.dumps(
                {"Repeats": 1, "Rps": [200], "DurationSecs": 1, "WarmupSecs": 0}
            ),
            encoding="utf-8",
        )
        (in_dir / "tracebench.json").write_text(
            json.dumps(
                {
                    "trace_dir": "trace-analysis/golden/S_86516878",
                    "config_dir": "apps/tracebench/tracebench/example_config/",
                    "slo_ms": 100,
                    "extra_env": {},
                }
            ),
            encoding="utf-8",
        )

        cfg = ExperimentConfig.load(
            experiment_name="e2e_test",
            app_name="tracebench",
            repo_root=repo_root,
            app_plugin=app,
        )

        assert cfg.app_name == "tracebench"
        assert cfg.experiment_name == "e2e_test"
        assert cfg.policies == ["sched_fifo"]
        assert cfg.gen_config["Rps"] == [200]
        assert isinstance(cfg.app_config, dict)


def test_tracebench_extra_env_records_service_forwarding_keys() -> None:
    app = get_app_plugin("tracebench")

    env = app.generate_env_vars(
        {"DurationSecs": 1, "WarmupSecs": 0, "Rps": [200]},
        {
            "slo_ms": 100,
            "orchestrator": "localhost:50051",
            "extra_env": {
                "MASA_FANOUT_AWARE": "0",
                "MASA_ESTIMATOR_STATS_LOG": "1",
            },
        },
        Path("/unused"),
    )

    assert env["MASA_FANOUT_AWARE"] == "0"
    assert env["MASA_ESTIMATOR_STATS_LOG"] == "1"
    assert (
        env["TRACEBENCH_SERVICE_EXTRA_ENV_KEYS"]
        == "MASA_ESTIMATOR_STATS_LOG,MASA_FANOUT_AWARE"
    )


def test_tracebench_exports_per_graph_slos() -> None:
    app = get_app_plugin("tracebench")

    env = app.generate_env_vars(
        {"DurationSecs": 1, "WarmupSecs": 0, "Rps": [200]},
        {
            "slo_ms": 100,
            "slo_ms_by_graph": {"S_14677443": 100, "S_98493745": 75},
        },
        Path("/unused"),
    )

    assert json.loads(env["SLO_MS_BY_GRAPH"]) == {
        "S_14677443": 100,
        "S_98493745": 75,
    }


def test_tracebench_goodput_uses_per_request_slo() -> None:
    requests = pd.DataFrame(
        {
            "e2e_latency_ms": [90.0, 90.0],
            "slo_us": [100_000, 75_000],
            "error": ["/None", "/None"],
        }
    )

    assert _compute_goodput(requests, slo_ms=100, duration_sec=1) == 1.0
