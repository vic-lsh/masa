# AGENTS.md

This file provides guidance to coding agents (e.g., Claude Code, Codex, Gemini CLI) when working with code in this repository.

## Project Overview

Masa is an RPC system that improves goodput (throughput for requests under SLO) via dynamic RPC prioritization, built on modified versions of Tonic, Hyper, and Tokio. It detects which RPCs are "running late" at runtime and dynamically adjusts priority based on end-to-end SLO constraints.

## Repository Structure

- `libs/`: Core Rust libraries - modified `tonic`, `hyper`, `tokio`, `tower`, and Masa-specific `masa` crate
- `apps/`: Microservice applications for evaluation (`hotel`, `socialnet`, `synthbench`, `tracebench`)
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
# See docs/experiments/workflow.md for detailed instructions
uv run -m exp_runner run <app> <experiment_name> --plot
# Run on Kind (Kubernetes in Docker) - supported for synthbench
uv run -m exp_runner run synthbench <experiment_name> --kind --plot
# Run on generic Kubernetes
uv run -m exp_runner run synthbench <experiment_name> --k8s --plot
uv run -m exp_runner plot <app> <experiment_name>
```

## Scheduling Policies (Feature Flags)

Policies are selected at **compile time** via feature flags. Applications must be built with the desired policy:
```bash
cargo build -p hotel --features sched_slo --release
cargo build -p hotel --features "sched_slo,abort_slo" --release
```

Key policy flags:

**Scheduling policies** (mutually exclusive):
- `sched_fifo`: FIFO ordering (baseline)
- `sched_slo`: Priority by end-to-end SLO deadline (implies tokio priority queue)
- `sched_tailclipper`: TailClipper paper's oldest-request-first policy with round-robin fairness
- `sched_pred`: Adds deadline tightening and dynamic reprioritization using downstream work estimates. Implies `sched_slo` and `estimator`.
- `sched_mt`: Switches to a multi-threaded Tokio runtime with a single shared priority heap (`Mutex<BinaryHeap>`). Implies `sched_slo`. Workers share one lock; no per-worker queues or work-stealing. Composable with `abort_slo`, `ac_pred`, `ac_rajomon`, `sched_pred`, etc.
- `sched_mt_multiqueue`: Replaces the `sched_mt` mutex heap with a k-relaxed multi-queue (arXiv:1411.1209) of `c×threads` sub-heaps — push to random, pop samples best-of-2. Implies `sched_mt`. Composable with `abort_slo`.

**Estimation infrastructure:**
- `estimator`: Enables shared latency estimation infrastructure (estimator type selection, latency maps, estimation state). Implied by `sched_pred` and `ac_pred`. Does not require `abort_slo` on its own.

**Composable modifiers:**
- `abort_slo`: Returns early for requests past their e2e deadline, avoiding wasteful work. Composable with any scheduling policy.
- `abort_slack`: Superset of `abort_slo` — also predictively aborts when estimated remaining compute time exceeds time left. Requires `estimator`.
- `signal_slack`: Same trigger as `abort_slack`, but the request continues; emits a soft `deadline_signal_count` to `ac_pred` so the ingress AIMD shrinks `admit_p` (throttling new arrivals) without aborting in-flight work. Requires `estimator` and `ac_pred`; mutually exclusive with `abort_slack`.

**Admission control** (mutually exclusive):
- `ac_pred`: Progressive cost-aware admission control — uses compute-time estimates and downstream utilization signals. Requires `estimator`.
- `ac_rajomon`: Token-bucket rate limiting admission control.

**Policy stack override:**
- `stack_custom`: Replaces the feature-selected `MasaStack` with the agent-owned `AgentStack` in `libs/masa-policy/src/agent/`, for every app. Requires a scheduling feature. Starts equal to `MasaStack`. See `docs/POLICY_MODULES.md`.

**Run queue override:**
- `sched_custom`: Makes the current-thread Tokio runtime use `rpcstack_sched::custom::Queue` (`libs/rpcstack-sched/src/custom.rs`) instead of the queue the scheduling feature selects. Requires a scheduling feature (`compile_error!` otherwise) so that Hyper passes priorities; composes with `stack_custom` and all modifiers; ignored by `sched_mt*`. Starts as a copy of the priority-heap queue. See `docs/POLICY_MODULES.md`.

`scripts/check.sh` checks: default (no features), `sched_fifo`, `sched_fifo,abort_slo`, `sched_slo`, `sched_slo,abort_slo`, `sched_tailclipper,abort_slo`, `sched_slo,ac_rajomon`, `sched_slo,ac_pred,est_mean_var`, `sched_pred,abort_slo,ac_pred,est_mean_var`, `sched_pred,abort_slack,est_mean_var`, `sched_pred,signal_slack,ac_pred,est_mean_var`, `sched_pred,abort_slack,ac_pred,est_mean_var,deadline_equals_slack`, `sched_mt`, `sched_mt,abort_slo`, `sched_mt,ac_rajomon`, `sched_mt_multiqueue`, `sched_mt_multiqueue,abort_slo`, `sched_slo,stack_custom`, `sched_pred,abort_slack,ac_pred,est_mean_var,stack_custom`, `sched_slo,sched_custom`, `sched_pred,abort_slack,ac_pred,est_mean_var,sched_custom`.

## Architecture

### Data Flow
1. Client's `Hooks` run the module stack; Masa's budget modules write the child's `Context` (deadline/priority) as the `budget` section of the HTTP/2 `ctx` header, beside the other modules' sections
2. Server-side `hyper` parses `ctx` header, extracts `PriorityHint`
3. Under the `hyper/masa` feature, Hyper's default HTTP/2 stream executor calls `tokio::spawn_with_prio(handler_future, priority)`
4. Modified `tokio` runtime enqueues task in a priority queue (binary heap); lower `PriorityHint` value = higher priority

### Application Integration Requirements
- Use normal `.serve(addr)`; Masa-aware execution is enabled by compile-time scheduling/admission features
- Use `#[tokio::main(flavor = "current_thread")]` for normal policies; `sched_mt` and its queue backends require Tokio's multi-thread runtime
- `PriorityHint::infra()` (value 0) is reserved for infrastructure tasks and always runs first

### libs/masa & libs/masa-core
Application-facing Masa API and core types:
- `libs/masa-core`: `Context` (the budget module's wire data), `PriorityHint`, `Prioritize`, and latency distribution utilities
- `libs/masa`: `DefaultHooks` selection by feature flag, context creation helpers, load-balanced transport, and policy-facing reexports from `masa-policy`
- `DefaultHooks`: `tonic::masa::noop::NoopHooks` with no scheduling features; `masa_policy::PolicyHooks` (= `PolicyHooks<MasaStack>`) when scheduling features are enabled; `PolicyHooks<AgentStack>` with `stack_custom`

### libs/rpcstack-wire, libs/rpcstack & libs/rpcstack-tonic
The module framework. It owns every mechanism and has no policy (no deadlines, priorities or tokens; a request is known only by service and method name), and depends on no Masa crate, hyper or tokio runtime policy crate (`scripts/validate_rpcstack_boundary.py`). Each crate has a README listing its public surface:
- `libs/rpcstack-wire`: leaf crate with the `ctx` header section codec (`sections`, `find_section`, `encode_payload`, `decode_payload`, `push_section`, `describe`, `HEADER_NAME`); each section is `<name>:<base64 bincode>`; `masa-core` uses it so Hyper can read one section selectively
- `libs/rpcstack`: `Module` (with `requires`), `ModuleServer`, `ServerInit`, `build_server` (checks that `Module::NAME` is unique across the stack and dependencies are met), `Extensions`/`ChildState` (typed per-request and per-child maps, with typed decision points: `propose`/`proposals`/`resolve`), `Outcome`/`ChildOutcome`, `Stack`, `policy_stack!`, and the wire codec (`WireIn`, `WireOut`, `EncodedSection`, `peek`, `describe`). `Module::POLL_HOOKS` (default `true`) lets a module that keeps both poll hooks at their defaults opt out, so a stack with no poll hooks skips the per-poll lock; debug builds panic if a module declares `false` and overrides one. It owns hook order: pre-hooks head first and short-circuiting; `seal_child_rpc`, `after_child_rpc` and `finalize` tail first, only for modules whose pre-hook ran; `after_poll` head first
- `libs/rpcstack-tonic`: `PolicyHooks<S>` — tonic's `Hooks` for any stack; owns request plumbing and dispatches every hook through the module stack `S`. Also `RequestExt`/`ResponseExt`/`StatusExt` for module wire data and method-name overrides
- Tests of the framework use toy modules and no Masa types: `libs/rpcstack/tests`, `libs/rpcstack-tonic/tests`

### libs/rajomon
Rajomon admission control (feature `ac_rajomon`) as a module built on the framework alone: `RajomonModule`, `RajomonWire`, the process-wide `RAJOMON_STATE` (prices, price-update worker), the client-side `CLIENT_TOKEN_BUCKET`, and `RajomonParams`. It depends on `rpcstack` and generic libraries, never on a Masa crate or hyper, and no framework crate depends on it (`scripts/validate_rpcstack_boundary.py`). `libs/rajomon/tests/toy_stack.rs` runs it in a stack of the framework alone. `masa-policy` depends on it under `ac_rajomon` and re-exports its public items. See `libs/rajomon/README.md`

### libs/masa-policy/
Masa's own policy, written against the framework; it re-exports the framework under the same paths (`masa_policy::Module`, `policy_stack!`, `PolicyHooks<S = MasaStack>`):
- `hooks.rs`: `PolicyHooks<S = MasaStack>` and the contexts as aliases of the `rpcstack-tonic` types
- `masa_stack.rs`: `MasaStack`, the only place where features choose modules (disabled slots are `()`)
- `agent/`: `AgentStack`, the agent-owned stack selected by `stack_custom`; new policies for apps and experiments go here
- `module/`: Built-in modules — `budget.rs` (request facts, and the child budget as a decision the budget module resolves; `ContextBuilder` and the root priority formula), `e2e_deadline_guard.rs`, `oracle.rs`, `queue_latency.rs`, `est/` (estimation), `admission/` (predictive; Rajomon lives in `libs/rajomon`)
- To add a policy, write a new module implementing `Module` and compose a stack; do not add branches to the hook adapter. See `docs/POLICY_MODULES.md`
- `context_ext.rs`: Context (budget section) helpers and `MasaRequestExt`/`MasaResponseExt`/`MasaStatusExt` (the framework's request helpers plus the Masa context)
- Depends on `rpcstack`, `rpcstack-tonic`, `tonic` and, under `ac_rajomon`, `rajomon`; vendored tonic does not depend on `masa-policy` or on the framework crates

### libs/tonic/tonic/src/masa/
Tonic-specific glue:
- `hooks.rs`: Core hook trait definitions (`Hooks`, `ServerHooks`, `ParentHooks`, `ClientHooks`) used by tonic internals and generated code
- `noop.rs`: No-op Masa hooks implementation for builds without scheduling features
- `runtime.rs`: Bridge from tonic `ParentContext` to tokio `PollHook`
- `thread_local.rs`: Thread-local storage for parent/server context propagation
- Has no dependency on `masa-policy` or `masa-core`; concrete policy hooks are wired through `masa::DefaultHooks`
- `libs/tonic/tonic/src/transport/masa_channel/`: Lower-level Masa-aware channel internals; application-facing load-balanced transport lives in `masa::transport`

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
- When working in a worktree derived from `main`, pull the latest `main` from the remote before starting changes.
- Before editing, understand call sites and dependencies
- Keep changes scoped to the request unless necessary for correctness
- When adding a new feature flag, be sure to update the documentation and include it in all applications.

## Applications

Experiment apps in `apps/` with experiment configs in `exp/<app>/in/<experiment>/`:
- `hotel`: Rust port of Deathstarbench Hotel application
- `socialnet`: Social network microservice benchmark
- `synthbench`: Configurable synthbench workload
- `tracebench`: Trace-driven trace-driven RPC benchmark

There is also `apps/benchmark/` for measuring serialization overhead and E2E latency (`cargo bench -p masa-benchmark`).

See `docs/experiments/workflow.md` for running experiments and `docs/experiments/analysis.md` for interpreting results.
