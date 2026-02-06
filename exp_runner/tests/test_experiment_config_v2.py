"""Tests for experiment_config_v2 module."""

import tempfile
from pathlib import Path

from exp_runner.runner.experiment_config_v2 import (
    ApiSpec,
    ExperimentConfigV2,
    ExecutionSpec,
    LoadGenSpec,
)


def test_api_spec_basic():
    """Test basic ApiSpec creation."""
    api = ApiSpec(name="Search", slo_us=200000, weight=0.6)
    assert api.name == "Search"
    assert api.slo_us == 200000
    assert api.weight == 0.6
    assert api.timeout_ms is None


def test_api_spec_with_timeout():
    """Test ApiSpec with timeout override."""
    api = ApiSpec(name="Reservation", slo_us=300000, timeout_ms=2000)
    assert api.timeout_ms == 2000


def test_loadgen_spec():
    """Test LoadGenSpec creation."""
    apis = [
        ApiSpec(name="Search", slo_us=200000, weight=0.6),
        ApiSpec(name="Reservation", slo_us=300000, weight=0.4),
    ]
    loadgen = LoadGenSpec(
        rps=[100, 200, 300],
        default_timeout_ms=1000,
        apis=apis,
    )
    assert loadgen.rps == [100, 200, 300]
    assert loadgen.default_timeout_ms == 1000
    assert len(loadgen.apis) == 2


def test_execution_spec():
    """Test ExecutionSpec creation."""
    execution = ExecutionSpec(
        repeats=3,
        policies=["fifo", "prio_global"],
        warmup_secs=10,
        duration_secs=60,
    )
    assert execution.repeats == 3
    assert execution.policies == ["fifo", "prio_global"]
    assert execution.warmup_secs == 10
    assert execution.duration_secs == 60


def test_experiment_config_from_dict():
    """Test ExperimentConfigV2 creation from dictionary."""
    data = {
        "kind": "Experiment",
        "metadata": {
            "name": "baseline",
            "app": "hotel",
        },
        "spec": {
            "topology_ref": "default",
            "replica_overrides": {
                "rate": 2,
                "profile": 2,
            },
            "execution": {
                "repeats": 3,
                "policies": ["fifo", "prio_global"],
                "warmup_secs": 10,
                "duration_secs": 60,
            },
            "loadgen": {
                "rps": [100, 200, 300],
                "default_timeout_ms": 1000,
                "apis": [
                    {
                        "name": "Search",
                        "slo_us": 200000,
                        "weight": 0.6,
                    },
                    {
                        "name": "Reservation",
                        "slo_us": 300000,
                        "weight": 0.4,
                        "timeout_ms": 2000,
                    },
                ],
            },
        },
    }

    config = ExperimentConfigV2.from_dict(data)
    assert config.name == "baseline"
    assert config.app == "hotel"
    assert config.topology_ref == "default"
    assert config.replica_overrides == {"rate": 2, "profile": 2}
    assert config.execution.repeats == 3
    assert config.execution.policies == ["fifo", "prio_global"]
    assert config.loadgen.rps == [100, 200, 300]
    assert len(config.loadgen.apis) == 2
    assert config.loadgen.apis[0].name == "Search"
    assert config.loadgen.apis[1].timeout_ms == 2000


def test_experiment_config_to_dict():
    """Test ExperimentConfigV2 serialization to dictionary."""
    apis = [
        ApiSpec(name="Search", slo_us=200000, weight=0.6),
        ApiSpec(name="Reservation", slo_us=300000, weight=0.4, timeout_ms=2000),
    ]
    loadgen = LoadGenSpec(
        rps=[100, 200, 300],
        default_timeout_ms=1000,
        apis=apis,
    )
    execution = ExecutionSpec(
        repeats=3,
        policies=["fifo", "prio_global"],
        warmup_secs=10,
        duration_secs=60,
    )
    config = ExperimentConfigV2(
        name="baseline",
        app="hotel",
        topology_ref="default",
        replica_overrides={"rate": 2},
        execution=execution,
        loadgen=loadgen,
    )

    data = config.to_dict()
    assert data["kind"] == "Experiment"
    assert data["metadata"]["name"] == "baseline"
    assert data["metadata"]["app"] == "hotel"
    assert data["spec"]["topology_ref"] == "default"
    assert data["spec"]["replica_overrides"] == {"rate": 2}
    assert data["spec"]["execution"]["repeats"] == 3
    assert data["spec"]["loadgen"]["rps"] == [100, 200, 300]
    assert len(data["spec"]["loadgen"]["apis"]) == 2


def test_experiment_config_from_yaml():
    """Test ExperimentConfigV2 loading from YAML file."""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_file = Path(tmpdir) / "experiment.yaml"
        config_file.write_text(
            """
kind: Experiment
metadata:
  name: baseline
  app: hotel
spec:
  topology_ref: default
  execution:
    repeats: 1
    policies: [fifo]
    warmup_secs: 5
    duration_secs: 30
  loadgen:
    rps: [100]
    default_timeout_ms: 1000
    apis:
      - name: Search
        slo_us: 200000
"""
        )

        config = ExperimentConfigV2.from_yaml(config_file)
        assert config.name == "baseline"
        assert config.app == "hotel"
        assert config.execution.repeats == 1
        assert config.loadgen.rps == [100]


def test_experiment_config_minimal():
    """Test ExperimentConfigV2 with minimal required fields."""
    data = {
        "metadata": {
            "name": "test",
            "app": "synthetic",
        },
        "spec": {
            "execution": {
                "policies": ["fifo"],
            },
            "loadgen": {
                "rps": [50],
                "apis": [
                    {
                        "name": "api_a",
                        "slo_us": 100000,
                    }
                ],
            },
        },
    }

    config = ExperimentConfigV2.from_dict(data)
    assert config.name == "test"
    assert config.topology_ref is None  # Optional, defaults to None
    assert config.execution.repeats == 1  # Default value
    assert config.loadgen.default_timeout_ms == 1000  # Default value
