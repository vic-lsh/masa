"""Tests for legacy configuration converters."""

import json
import tempfile
from pathlib import Path

from exp_runner.runner.legacy import (
    convert_experiment_config_to_gen_config,
    convert_legacy_to_experiment_config,
    load_legacy_gen_config,
)


def test_load_legacy_gen_config():
    """Test loading legacy gen_config.json file."""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_file = Path(tmpdir) / "gen_config.json"
        data = {
            "Repeats": 3,
            "RPSList": [100, 200],
            "Addr": "frontend:8080",
        }
        with open(config_file, "w") as f:
            json.dump(data, f)

        loaded = load_legacy_gen_config(config_file)
        assert loaded["Repeats"] == 3
        assert loaded["RPSList"] == [100, 200]


def test_convert_legacy_hotel_format():
    """Test converting legacy hotel gen_config.json to new format."""
    gen_config = {
        "Repeats": 3,
        "RPSList": [100, 200, 300],
        "Addr": "frontend:8660",
        "Warmup": 10,
        "Duration": 60,
        "APIs": [
            {
                "Name": "Search",
                "ReqWeight": 0.6,
                "SLO": 200000,
                "Timeout": 1000,
            },
            {
                "Name": "Reservation",
                "ReqWeight": 0.4,
                "SLO": 300000,
                "Timeout": 2000,
            },
        ],
    }
    policies = ["fifo", "prio_global"]

    config = convert_legacy_to_experiment_config(
        gen_config, policies, "baseline", "hotel"
    )

    assert config.name == "baseline"
    assert config.app == "hotel"
    assert config.execution.repeats == 3
    assert config.execution.policies == ["fifo", "prio_global"]
    assert config.execution.warmup_secs == 10
    assert config.execution.duration_secs == 60
    assert config.loadgen.rps == [100, 200, 300]
    assert config.loadgen.default_timeout_ms == 1000
    assert len(config.loadgen.apis) == 2
    assert config.loadgen.apis[0].name == "Search"
    assert config.loadgen.apis[0].slo_us == 200000
    assert config.loadgen.apis[0].weight == 0.6
    assert config.loadgen.apis[0].timeout_ms is None  # Same as default
    assert config.loadgen.apis[1].name == "Reservation"
    assert config.loadgen.apis[1].timeout_ms == 2000  # Differs from default


def test_convert_legacy_synthetic_format():
    """Test converting legacy synthetic gen_config.json to new format."""
    gen_config = {
        "Repeats": 1,
        "RPSList": [50, 100, 150],
        "Addr": "frontend:8080",
        "Warmup": 5,
        "Duration": 30,
        "APIs": [
            {
                "Name": "api_a",
                "ReqWeight": 1.0,
                "SLO": 100000,
                "Timeout": 500,
            }
        ],
    }
    policies = ["fifo", "prio_global", "prio_local,early"]

    config = convert_legacy_to_experiment_config(
        gen_config, policies, "fanout-test", "synthetic"
    )

    assert config.name == "fanout-test"
    assert config.app == "synthetic"
    assert config.execution.repeats == 1
    assert config.execution.warmup_secs == 5
    assert config.execution.duration_secs == 30
    assert config.loadgen.rps == [50, 100, 150]
    assert config.loadgen.default_timeout_ms == 500
    assert len(config.loadgen.apis) == 1
    assert config.loadgen.apis[0].name == "api_a"


def test_convert_experiment_config_to_gen_config():
    """Test converting new ExperimentConfigV2 to legacy gen_config.json format."""
    from exp_runner.runner.experiment_config_v2 import (
        ApiSpec,
        ExperimentConfigV2,
        ExecutionSpec,
        LoadGenSpec,
    )

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
        execution=execution,
        loadgen=loadgen,
    )

    gen_config = convert_experiment_config_to_gen_config(config, "frontend:8660")

    assert gen_config["Repeats"] == 3
    assert gen_config["RPSList"] == [100, 200, 300]
    assert gen_config["Addr"] == "frontend:8660"
    assert gen_config["Warmup"] == 10
    assert gen_config["Duration"] == 60
    assert len(gen_config["APIs"]) == 2
    assert gen_config["APIs"][0]["Name"] == "Search"
    assert gen_config["APIs"][0]["SLO"] == 200000
    assert gen_config["APIs"][0]["ReqWeight"] == 0.6
    assert gen_config["APIs"][0]["Timeout"] == 1000  # Default timeout
    assert gen_config["APIs"][1]["Timeout"] == 2000  # Custom timeout


def test_convert_legacy_with_defaults():
    """Test converting legacy config with missing optional fields uses defaults."""
    gen_config = {
        "Repeats": 1,
        "RPSList": [100],
        "Addr": "frontend:8080",
        "APIs": [
            {
                "Name": "api_a",
                "SLO": 100000,
            }
        ],
    }
    policies = ["fifo"]

    config = convert_legacy_to_experiment_config(gen_config, policies, "test", "test")

    # Should use defaults for missing fields
    assert config.execution.warmup_secs == 10  # Default
    assert config.execution.duration_secs == 60  # Default
    assert config.loadgen.apis[0].weight == 1.0  # Default


def test_roundtrip_conversion():
    """Test that legacy -> new -> legacy conversion preserves data."""
    original_gen_config = {
        "Repeats": 2,
        "RPSList": [50, 100],
        "Addr": "frontend:8080",
        "Warmup": 15,
        "Duration": 45,
        "APIs": [
            {
                "Name": "Search",
                "ReqWeight": 1.0,
                "SLO": 150000,
                "Timeout": 800,
            }
        ],
    }
    policies = ["fifo"]

    # Convert to new format
    new_config = convert_legacy_to_experiment_config(
        original_gen_config, policies, "test", "test"
    )

    # Convert back to legacy format
    roundtrip_gen_config = convert_experiment_config_to_gen_config(
        new_config, "frontend:8080"
    )

    # Compare (excluding Addr which is passed separately)
    assert roundtrip_gen_config["Repeats"] == original_gen_config["Repeats"]
    assert roundtrip_gen_config["RPSList"] == original_gen_config["RPSList"]
    assert roundtrip_gen_config["Warmup"] == original_gen_config["Warmup"]
    assert roundtrip_gen_config["Duration"] == original_gen_config["Duration"]
    assert len(roundtrip_gen_config["APIs"]) == len(original_gen_config["APIs"])
    assert (
        roundtrip_gen_config["APIs"][0]["Name"]
        == original_gen_config["APIs"][0]["Name"]
    )
    assert (
        roundtrip_gen_config["APIs"][0]["SLO"] == original_gen_config["APIs"][0]["SLO"]
    )
