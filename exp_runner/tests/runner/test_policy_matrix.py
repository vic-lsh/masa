from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT_PATH = (
    Path(__file__).resolve().parents[3] / "scripts" / "validate_policy_matrix.py"
)
SPEC = importlib.util.spec_from_file_location("validate_policy_matrix", SCRIPT_PATH)
assert SPEC is not None
assert SPEC.loader is not None
validate_policy_matrix = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validate_policy_matrix)


def test_policy_matrix_matches_repo() -> None:
    assert validate_policy_matrix.validate_policy_matrix() == []
