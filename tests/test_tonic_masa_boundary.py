from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "scripts" / "validate_tonic_masa_boundary.py"
SPEC = importlib.util.spec_from_file_location("validate_tonic_masa_boundary", SCRIPT_PATH)
assert SPEC is not None
assert SPEC.loader is not None
validate_tonic_masa_boundary = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validate_tonic_masa_boundary)


def test_vendored_tonic_library_does_not_depend_on_masa_crates() -> None:
    assert validate_tonic_masa_boundary.validate_tonic_masa_boundary() == []


def test_boundary_rejects_tonic_library_masa_dependencies(tmp_path: Path) -> None:
    manifest = tmp_path / "libs/tonic/tonic/Cargo.toml"
    source = tmp_path / "libs/tonic/tonic/src/lib.rs"
    manifest.parent.mkdir(parents=True)
    source.parent.mkdir(parents=True)
    manifest.write_text(
        """
[package]
name = "tonic"

[dependencies]
masa-core = { path = "../../../masa-core" }
renamed-policy = { package = "masa-policy", path = "../../../masa-policy" }
"""
    )
    source.write_text("pub mod masa;\n")

    errors = validate_tonic_masa_boundary.validate_tonic_masa_boundary(tmp_path)

    assert any("dependency 'masa-core'" in error for error in errors)
    assert any("package 'masa-policy'" in error for error in errors)


def test_boundary_rejects_tonic_library_masa_crate_paths(tmp_path: Path) -> None:
    manifest = tmp_path / "libs/tonic/tonic/Cargo.toml"
    source = tmp_path / "libs/tonic/tonic/src/lib.rs"
    manifest.parent.mkdir(parents=True)
    source.parent.mkdir(parents=True)
    manifest.write_text(
        """
[package]
name = "tonic"

[dependencies]
bytes = "1"
"""
    )
    source.write_text(
        """
use masa_core::Context;

fn helper() {
    let _ = masa_policy::PolicyHooks::default();
}
"""
    )

    errors = validate_tonic_masa_boundary.validate_tonic_masa_boundary(tmp_path)

    assert any("masa_core::" in error for error in errors)
    assert any("masa_policy::" in error for error in errors)


def test_boundary_scope_excludes_vendored_tonic_tests(tmp_path: Path) -> None:
    manifest = tmp_path / "libs/tonic/tonic/Cargo.toml"
    source = tmp_path / "libs/tonic/tonic/src/lib.rs"
    test_source = tmp_path / "libs/tonic/tests/masa_integration_tests/src/lib.rs"
    manifest.parent.mkdir(parents=True)
    source.parent.mkdir(parents=True)
    test_source.parent.mkdir(parents=True)
    manifest.write_text(
        """
[package]
name = "tonic"

[dependencies]
bytes = "1"
"""
    )
    source.write_text("pub mod masa;\n")
    test_source.write_text("use masa_core::Context;\n")

    assert validate_tonic_masa_boundary.validate_tonic_masa_boundary(tmp_path) == []
