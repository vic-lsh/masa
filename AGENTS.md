# AGENTS.md

This file provides guidance to coding agents (e.g., Claude Code, Codex, Gemini CLI) when working with code in this repository.

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

# Run all tests (see script for which packages are tested; tokio tests are skipped due to flakiness)
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

# Run Python tests
uv run pytest

# Run experiments (they run for a long time; don't run unless the user asks you to)
# See EXPERIMENT_WORKFLOW.md for detailed instructions
uv run -m exp_runner run <app> <experiment_name> --plot
# Run on Kind (Kubernetes in Docker) - supported for synthetic
uv run -m exp_runner run synthetic <experiment_name> --kind --plot
# Run on generic Kubernetes
uv run -m exp_runner run synthetic <experiment_name> --k8s --plot
uv run -m exp_runner plot <app> <experiment_name>
```

## Scheduling Policies (Feature Flags)

Policies are selected at **compile time** via feature flags. Applications must be built with the desired policy:
```bash
cargo build -p hotel --features sched_slo --release
cargo build -p hotel --features "sched_slo,slo_abort" --release
```

Key policy flags:

**Scheduling policies** (mutually exclusive):
- `sched_fifo`: FIFO ordering (baseline)
- `sched_slo`: Priority by end-to-end SLO deadline (implies tokio priority queue)
- `sched_tailclipper`: TailClipper paper's oldest-request-first policy with round-robin fairness
- `sched_pred`: Adds deadline tightening and dynamic reprioritization using downstream work estimates. Implies `sched_slo` and `estimator`.

**Estimation infrastructure:**
- `estimator`: Enables shared latency estimation infrastructure (estimator type selection, latency maps, estimation state). Implied by `sched_pred` and `ac_pred`. Does not require `slo_abort` on its own.

**Composable modifiers:**
- `slo_abort`: Returns early for requests past their e2e deadline, avoiding wasteful work. Composable with any scheduling policy.

**Admission control** (mutually exclusive):
- `ac_pred`: Progressive cost-aware admission control — uses compute-time estimates and downstream utilization signals. Requires `estimator`.
- `ac_rajomon`: Token-bucket rate limiting admission control.

`scripts/check.sh` checks: default (no features), `sched_fifo`, `sched_fifo,slo_abort`, `sched_slo`, `sched_slo,slo_abort`, `sched_tailclipper,slo_abort`, `sched_slo,ac_rajomon`, `sched_slo,ac_pred,est_mean_var`, `sched_pred,slo_abort,ac_pred,est_mean_var`.

## Architecture

### Data Flow
1. Client's `Hooks` calculates child deadline/priority, serializes `Context` to JSON in HTTP/2 header (`ctx` key)
2. Server-side `hyper` parses `ctx` header, extracts `PriorityHint`
3. `hyper` calls `tokio::spawn_with_prio(handler_future, priority)` via the `Exec::Masa` executor
4. Modified `tokio` runtime enqueues task in a priority queue (binary heap); lower `PriorityHint` value = higher priority

### Application Integration Requirements
- Use `.serve_with_masa(addr)` instead of `.serve(addr)` to enable priority-aware execution
- Use `#[tokio::main(flavor = "current_thread")]` — the priority scheduler is implemented in the single-threaded runtime
- `PriorityHint::infra()` (value 0) is reserved for infrastructure tasks and always runs first

### libs/masa & libs/masa-core
Core Masa types and utilities:
- `Context`/`ContextBuilder`: RPC context with deadline/priority info
- `PriorityHint`: Priority value (lower = higher priority; reversed `Ord` for `BinaryHeap`)
- `Prioritize`: Priority calculation trait
- `LatencyEstimator`: Latency distribution tracking

### libs/tonic/tonic-core/src/masa_ext/
Core hook trait definitions (`Hooks`, `ServerHooks`, `ParentHooks`, `ClientHooks`) and `NoopHooks`, plus shared types (`Request`, `Response`, `Status`, metadata). Has no dependency on `masa-policy` or `masa-core`.

### libs/masa-policy/
Policy implementations extracted from tonic:
- `hooks.rs`: `PolicyHooks` — unified hook implementation with `for_each_overlay!` macro dispatch
- `overlay/`: Composable overlay system — `slo_abort.rs`, `queue_lat.rs`, `predictive/` (estimation + admission), `rajomon.rs`
- `context_ext.rs`: Context serialization helpers for tonic requests/responses

### libs/tonic/tonic/src/masa_ext/
Tonic-specific glue:
- `mod.rs`: `DefaultHooks` type alias selected by feature flags; re-exports from `tonic-core` and `masa-policy`
- `runtime/mod.rs`: Bridge from tonic `ParentContext` to tokio `PollHook`
- `thread_local.rs`: Thread-local storage for parent/server context propagation
- `transport/masa_channel/`: Masa-aware channel transport

### Patched Libraries
The workspace patches crates.io dependencies with local modified versions (see `Cargo.toml` `[patch.crates-io]`). All must be built from local copies:
- `tokio`, `tokio-util`, `tokio-stream`, `tokio-test`, `tokio-macros` — priority-aware scheduler
- `hyper` — priority-aware HTTP/2 stream handling
- `tower`, `tower-service`, `tower-layer`

### Deep Dive
See `docs/MASA_POLICY_IMPL.md` for detailed implementation walkthrough covering feature flags, context serialization, transport layer changes, and runtime integration.

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
- Do not tolerate any warnings in cargo-check. Always fix them.
- Use `./scripts/format.sh` to format code consistently.
- Write idiomatic Rust code.

### Python
- Use type hints for function arguments and return values
- Use `uv add <package>` for dependencies, not pip directly
- Experiment logic goes in `exp_runner/runner`, tests in `exp_runner/tests`
- After changes in `exp/`: run `uv run pytest`

### Testing
- New features must include tests
- Behavior changes require updating existing tests or adding new ones
- `scripts/test.sh` is the source of truth for which packages are currently passing tests
- Use `scripts/test_e2e_hotel.sh` or similar scripts for end-to-end verification

### Working in this Codebase
- Before editing, understand call sites and dependencies
- Keep changes scoped to the request unless necessary for correctness
- When adding a new feature flag, be sure to update the documentation and include it in all applications.

## Applications

Experiment apps in `apps/` with experiment configs in `exp/<app>/data/in/<experiment>/`:
- `hotel`: Rust port of Deathstarbench Hotel application
- `socialnet`: Social network microservice benchmark
- `synthetic`: Configurable synthetic workload
- `mssim`: Trace-driven microservice simulator

There is also `apps/benchmark/` for measuring serialization overhead and E2E latency (`cargo bench -p masa-benchmark`).

See `EXPERIMENT_WORKFLOW.md` for running experiments and `EXPERIMENT_ANALYSIS.md` for interpreting results.
