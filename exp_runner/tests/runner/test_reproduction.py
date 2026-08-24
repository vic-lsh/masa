from dataclasses import replace
from pathlib import Path

import pytest

from exp_runner.runner.cli import create_parser
from exp_runner.runner.reproduction import (
    ReproductionManifestError,
    ReproductionSuite,
)


REPO_ROOT = Path(__file__).resolve().parents[3]
SUITE_PATH = REPO_ROOT / "artifact" / "eval" / "suite.yaml"
SEMANTIC_ID = "fixed_capacity/trace_short_sequential_branching"


def test_pilot_suite_loads_and_validates_runner_binding() -> None:
    suite = ReproductionSuite.load(SUITE_PATH, REPO_ROOT)

    assert suite.name == "masa-evaluation"
    assert len(suite.experiments) == 1
    experiment = suite.experiments[0]
    assert experiment.id == SEMANTIC_ID
    assert experiment.study_id == "fixed_capacity_goodput"
    assert experiment.runner.app == "tracebench"
    assert experiment.runner.experiment == "s1467_acCmp"
    assert experiment.policy_parameters["pred"] == {"tau_er": 2, "estimator_k": 0}
    assert [policy.id for policy in experiment.policies] == [
        "rajomon_fifo",
        "rajomon_tailclipper",
        "masa",
    ]


def test_select_uses_semantic_id() -> None:
    suite = ReproductionSuite.load(SUITE_PATH, REPO_ROOT)

    assert suite.select(SEMANTIC_ID) == suite.experiments
    with pytest.raises(ReproductionManifestError, match="Unknown experiment"):
        suite.select("fixed_capacity/not_present")


def test_runner_drift_is_rejected() -> None:
    suite = ReproductionSuite.load(SUITE_PATH, REPO_ROOT)
    drifted = replace(suite.experiments[0], offered_rps=(1, 2, 3))

    with pytest.raises(ReproductionManifestError, match="runner Rps"):
        drifted.validate_runner_binding(REPO_ROOT)


def test_policy_parameter_drift_is_rejected() -> None:
    suite = ReproductionSuite.load(SUITE_PATH, REPO_ROOT)
    drifted = replace(suite.experiments[0], policy_parameters={"pred": {}})

    with pytest.raises(ReproductionManifestError, match="policy parameters"):
        drifted.validate_runner_binding(REPO_ROOT)


def test_reproduce_cli_parses_semantic_selection() -> None:
    parser = create_parser()
    args = parser.parse_args(
        [
            "reproduce",
            "plan",
            "artifact/eval/suite.yaml",
            "--select",
            SEMANTIC_ID,
        ]
    )

    assert args.reproduce_command == "plan"
    assert args.select == SEMANTIC_ID
