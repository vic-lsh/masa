# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Masa is an RPC system that improves goodput (throughput for requests under SLO) via dynamic RPC prioritization, built on modified versions of Tonic, Hyper, and Tokio. It detects which RPCs are "running late" at runtime and dynamically adjusts priority based on end-to-end SLO constraints.

## Repository Structure

- `libs/`: Core Rust libraries - modified `tonic`, `hyper`, `tokio`, `tower`, and Masa-specific `masa` crate
- `apps/`: Microservice applications for evaluation (`hotel`, `socialnet`, `synthetic`, `mssim`)
- `exp/`: Python-based experiment orchestration and analysis tools
- `scripts/`: Shell scripts for CI/CD (test, check, format)

## Build and Test Commands

### Rust Development (run from repo root)

```bash
# Format code
./scripts/format.sh   # or: cargo fmt

# Check all feature flag combinations
./scripts/check.sh

# Run all tests
./scripts/test.sh

# Test single package
cargo test -p <package_name>

# Test specific function
cargo test -p <package_name> -- <test_function_name>
```

### Python Development

```bash
# Setup (one-time)
curl -LsSf https://astral.sh/uv/install.sh | sh
uv sync
source .venv/bin/activate

# Run Python tests
pytest

# Run experiments (they run for a long time; don't run unless the user asks you to)
python3 -m exp.runner run <app> <experiment_name> --plot
python3 -m exp.runner plot <app> <experiment_name>
```

## Scheduling Policies (Feature Flags)

The codebase uses feature flags for scheduling policies. Key combinations checked by CI:
- `fifo`: FIFO ordering
- `prio_global`: Priority by end-to-end SLO end time
- `prio_oldest`: Oldest request first (from the TailClipper paper)
- `prio_global,early`: Global priority with early return
- `prio_local,early`: Local deadline-based priority
- `prio_oldest,early`: Oldest request first with early return (from the TailClipper paper)

Early return is a feature that allows the server to return a response early if the request is past its (e2e) deadline. This is useful for avoiding wasteful work that cannot be counted as goodput.

## Architecture

### libs/masa
Core Masa types and utilities:
- `Context`/`ContextBuilder`: RPC context with deadline/priority info
- `Prioritize`, `PriorityHint`: Priority calculation traits
- `LatencyEstimator`: Latency distribution tracking

### libs/tonic/tonic/src/masa/
Masa integration into Tonic gRPC:
- `context/`: Multiple context implementations (fifo, global, local, tracing variants)
- `transport/masa_channel/`: Masa-aware channel transport

### Patched Libraries
The workspace patches crates.io dependencies with local modified versions (see `Cargo.toml` `[patch.crates-io]`):
- `tokio`, `tokio-util`, `tokio-stream`, `tokio-test`, `tokio-macros`
- `hyper`
- `tower`, `tower-service`, `tower-layer`

## Conventions

### General
- Prefer simple designs; question if a feature is needed when it adds complexity
- Remove unused code immediately - do not comment it out
- Document *why* complex logic exists, not *what* it does
- Update `README.md` in relevant directories if CLI interfaces change

### Rust
- Use `Result` and `Option` idiomatically; avoid `unwrap()` except in tests
- The codebase relies heavily on `tokio`; use standard async patterns
- After changes: run `./scripts/check.sh`

### Python
- Use type hints for function arguments and return values
- Use `uv add <package>` for dependencies, not pip directly
- Experiment logic goes in `exp/runner`, tests in `exp/tests`
- After changes in `exp/`: run `pytest`

### Testing
- New features must include tests
- Behavior changes require updating existing tests or adding new ones
- `scripts/test.sh` is the source of truth for which packages are currently passing tests
- Use `scripts/test_e2e_hotel.sh` or similar scripts for end-to-end verification

### Working in this Codebase
- Before editing, understand call sites and dependencies
- Keep changes scoped to the request unless necessary for correctness

## Applications

Experiment apps in `apps/` with experiment configs in `exp/<app>/data/in/<experiment>/`:
- `hotel`: Rust port of Deathstarbench Hotel application
- `socialnet`: Social network microservice benchmark
- `synthetic`: Configurable synthetic workload
- `mssim`: Trace-driven microservice simulator