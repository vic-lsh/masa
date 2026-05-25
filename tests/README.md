# Repository Tests

This directory contains cross-cutting repository invariant tests that span
multiple subsystems or languages.

Use `tests/` for checks that validate repository-wide contracts, such as
feature matrices, shared configuration, scripts, or metadata that must stay in
sync across Rust, Python, and shell code.

Vendored-library boundary tests should document their intentional scope. For
example, the tonic/Masa boundary check covers `libs/tonic/tonic/Cargo.toml` and
`libs/tonic/tonic/src/**`, while leaving vendored tonic tests, examples, interop
crates, and integration fixtures available for external Masa integration
coverage.

Use `exp_runner/tests/` for Python experiment runner behavior. Runtime,
crate-specific, and application-specific behavior should stay with the relevant
Rust crate or application tests.
