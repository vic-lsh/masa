# Masa Patch Surface After Tonic Support Extraction

This document describes the intended crate graph after extracting Masa support
out of vendored Tonic where practical, and it records the remaining vendored
patches that still carry Masa-specific behavior. Use it when planning Tokio,
Hyper, Tonic, or Tower updates.

The goal is to keep policy code in Masa-owned crates, keep compatibility imports
stable for applications, and make the remaining vendored patches explicit.

## Current Crate Graph

```text
Applications
  |
  +-- masa                         facade and feature propagation
  |     |
  |     +-- masa-core              Context, PriorityHint, timing, estimators, fixed-list balance
  |     +-- masa-policy            PolicyHooks, layers, metadata helpers
  |     |     |
  |     |     +-- masa-core
  |     |     +-- masa-tonic-core  hook traits and runtime support
  |     |     +-- tonic-core       Request, Response, Status, metadata
  |     |     +-- tokio            optional, for runtime/queue metrics features
  |     |
  |     +-- tonic                  compatibility exports and transport glue
  |
  +-- tonic
        |
        +-- tonic-core             upstream-like tonic core types
        +-- masa-tonic-core        hook traits, no-op hooks, futures, thread-local runtime bridge
        +-- masa-policy            default policy implementation and context helpers
        +-- masa-core              context serialization and priorities
        +-- hyper                  H2 stream priority extraction before spawn
        +-- tokio                  priority task spawn and current-thread scheduler
        +-- tower                  crates.io transport buffering

tonic-build
  |
  +-- generated clients/servers use tonic::masa_ext compatibility paths
```

`tonic-core` remains an upstream-shaped crate for common Tonic types. Masa hook
contracts live in `masa-tonic-core`; scheduling and admission policy logic lives
in `masa-policy`; core wire and priority types live in `masa-core`.

## Compatibility Surface

The extraction keeps these application-facing paths stable:

- `tonic::masa_ext::{Hooks, ServerHooks, ParentHooks, ClientHooks}` reexports
  the hook traits from `masa-tonic-core`.
- `tonic::masa_ext::{noop, client, server, runtime}` reexports no-op hooks,
  thread-local access, and runtime hook helpers from `masa-tonic-core`.
- `tonic::masa_ext::{MasaRequestExt, MasaResponseExt, MasaStatusExt,
  get_masa_context_from_metadata, set_masa_context_in_metadata, read_context}`
  reexports metadata helpers from `masa-policy::context_ext`.
- `tonic::masa_ext::DefaultHooks` remains the compile-time policy selector:
  no scheduling features select `masa_tonic_core::noop::NoopHooks`; scheduling
  features select `masa_policy::PolicyHooks`.
- `tonic::transport::Server::serve_with_masa` remains the server entry point
  that selects Hyper's `Exec::Masa` executor.
- `tonic::transport::masa_channel::{Channel, LoadBalancedChannel}` remains the
  client-side replica channel used by applications and experiment deployment
  scripts.

Generated code from `tonic-build` still names `tonic::masa_ext` rather than the
new lower-level crates. That is intentional: generated applications should not
need to know whether a hook implementation came from `masa-tonic-core`,
`masa-policy`, or a compatibility reexport.

## Moved Out Of Vendored Tonic And Tower

The following Masa logic has already moved out of vendored Tonic or Tower:

- Hook traits (`Hooks`, `ServerHooks`, `ParentHooks`, `ClientHooks`) moved to
  `libs/masa-tonic-core/src/lib.rs`.
- No-op hook implementations moved to `libs/masa-tonic-core/src/noop.rs`.
- The exact-boundary future wrapper (`AbortableFuture` and builder helpers)
  moved to `libs/masa-tonic-core/src/future.rs`; Tonic reexports it from
  `tonic::util` for its server internals.
- Parent-context thread-local storage moved to
  `libs/masa-tonic-core/src/thread_local.rs`.
- The Tokio poll-hook bridge moved to `libs/masa-tonic-core/src/runtime.rs`.
- Metadata context helpers and `MasaRequestExt`/`MasaResponseExt`/
  `MasaStatusExt` moved to `libs/masa-policy/src/context_ext.rs`.
- Scheduling, estimation, deadline guard, queue latency, and admission control
  layers moved to `libs/masa-policy/src/layer/`.
- The fixed-list round-robin balance helper moved out of vendored Tower into
  `libs/masa-core/src/balance.rs`; `LoadBalancedChannel` now imports
  `masa_core::balance::Balance`.
- Tower crates now come from crates.io rather than the root `[patch.crates-io]`
  section, so Tower no longer appears in the remaining vendored patch table.

## Remaining Vendored Patch Surface

| Area | Files | Why it remains patched | Owning coverage |
| --- | --- | --- | --- |
| Tokio task and scheduler internals | `libs/tokio/tokio/src/task/spawn.rs`, `libs/tokio/tokio/src/task/local.rs`, `libs/tokio/tokio/src/runtime/task/core.rs`, `libs/tokio/tokio/src/runtime/task/poll_hook.rs`, `libs/tokio/tokio/src/runtime/context.rs`, `libs/tokio/tokio/src/runtime/scheduler/current_thread/queue/` | Masa priority is a task scheduling property. The runtime must store priority and poll hooks in task headers, inherit hooks across spawned child tasks, reprioritize running tasks, and choose ready tasks before their futures are polled. This cannot be reproduced by a Tonic or Tower layer after the task is already queued. | `libs/tokio/tokio/tests/masa_priority.rs`, queue unit tests in `libs/tokio/tokio/src/runtime/scheduler/current_thread/queue/`, `libs/masa-core/tests/priority_hint_behavior.rs`, and the `tokio (masa priority suite)` step in `scripts/test.sh`. |
| Hyper HTTP/2 stream priority before spawn | `libs/hyper/src/common/exec.rs`, `libs/hyper/src/proto/h2/server.rs` | Hyper is the layer that sees a new HTTP/2 stream and its headers before the stream future is spawned. Masa must read the `ctx` header and pass `PriorityHint` into the executor at that moment so first-poll ordering is correct. Waiting until Tonic receives the request is too late for initial scheduling. | Unit tests in `libs/hyper/src/common/exec.rs`; end-to-end priority behavior in `libs/tonic/tests/masa_integration_tests/tests/serve_behavior.rs`; feature-matrix compile coverage from `scripts/check.sh`. |
| Tonic unary handler-boundary hooks | `libs/tonic/tonic/src/server/grpc.rs`, `libs/tonic/tonic-build/src/client.rs`, `libs/tonic/tonic-build/src/server.rs`, `libs/tonic/tonic-build/src/code_gen.rs`, `libs/tonic/tonic-build/src/prost.rs` | Masa hooks must run at exact Tonic boundaries: create a server context before handler execution, set parent context before each handler poll, reset it after each poll, call child RPC hooks around generated client calls, and run finalize hooks before and after response serialization. These boundaries are not exposed as stable upstream extension points. | `libs/tonic/tests/masa_integration_tests/tests/masa_context.rs`, `libs/tonic/tests/masa_integration_tests/tests/metadata_helpers.rs`, and feature-specific policy tests in `libs/tonic/tests/masa_integration_tests/tests/policy_behavior.rs`. |
| Tonic executor glue | `libs/tonic/tonic/src/transport/server/mod.rs`, `libs/tonic/tonic/src/transport/service/executor.rs` | Applications need an explicit `serve_with_masa` entry point that selects Hyper's Masa executor without changing ordinary `serve` behavior. The shared executor trait also carries `PriorityHint` so internal transport workers can choose infrastructure priority where needed. | `libs/tonic/tests/masa_integration_tests/tests/serve_behavior.rs` verifies `serve_with_masa` priority behavior; application builds in `scripts/check.sh` verify the public API remains usable. |
| `LoadBalancedChannel` | `libs/tonic/tonic/src/transport/masa_channel/mod.rs` | Applications and Helm/experiment tooling rely on the current static replica naming, eager connection behavior, use of the Masa fixed-list round-robin balancer, and infrastructure-priority buffer worker. The channel can move later, but only after the replacement preserves these deployment contracts and Tonic `GrpcService` compatibility. | `libs/masa-core/src/balance.rs` owns unit coverage for round-robin order. Current channel coverage is indirect through application compile/e2e paths that instantiate `LoadBalancedChannel`, plus mssim service tests in `apps/mssim/generic-service/src/core.rs` that inject it through `new_from_service_name`. Before moving this code, add dedicated coverage for replica expansion, Docker Compose service-name mode, connection retry behavior, and buffer-worker priority. |

## Code That Can Move Later

The following code is intended to be movable once validation exists:

- `LoadBalancedChannel` can move out of vendored Tonic into a Masa-owned crate
  if the new type still implements the required Tonic client service traits,
  preserves replica naming and Docker Compose service-name behavior, and keeps
  the buffer worker at infrastructure priority.
- Some generated-code hook calls could shrink if upstream Tonic exposes stable
  client and server lifecycle hooks at the same boundaries.
- `serve_with_masa` could become an extension trait if Hyper and Tonic expose a
  stable way to choose a per-stream priority executor without patching their
  server transport.

The following code must remain patched unless upstream provides equivalent
extension points:

- Tokio task priority storage, current-thread run-queue selection, dynamic
  reprioritization, queue-latency timing, and poll-hook inheritance.
- Hyper H2 header inspection before stream task spawn.
- Tonic unary handler wrapping around the exact poll and serialization
  boundaries.

## Validation Required Before Moving More Code

Before moving any remaining vendored patch into a Masa-owned crate, validate:

- The `scripts/check.sh` feature matrix still compiles without warnings.
- `scripts/test.sh` still runs the Masa-specific Tokio, Tonic integration, and
  policy behavior suites.
- `serve_with_masa` still prioritizes a high-priority H2 stream before a
  lower-priority one under `sched_slo`.
- Generated clients still run `before_child_rpc`, `before_send`, `after_recv`,
  and `after_child_rpc` for sequential and spawned fanout calls.
- `finalize_before_serialization` and `finalize_after_serialization` still run
  at the documented points.
- `LoadBalancedChannel` still satisfies all application deployment contracts
  listed above.

## Explicit Non-Goals For This Epic

- Do not reintroduce Tower-layer semantic changes or replace the current
  client-side balancing policy while documenting the extraction.
- Do not rewrite `LoadBalancedChannel` in this epic; document it and move it in
  a separate change after focused tests exist.
- Do not change the scheduling policy semantics, admission-control behavior, or
  `current_thread` runtime requirement.
- Do not broaden Masa support to streaming RPCs without a separate design and
  test plan for streaming hook boundaries.
