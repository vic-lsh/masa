# Predictive Admission Control v2: Design Document

## Motivation

The current predictive AC (anchor_20) works well but has several algorithmic
weaknesses identified through analysis of the implementation and the OPAL
experiment track (14 iterations of alternatives, all regressed):

1. **Cost signal conflates compute and queueing.** Layer 2 uses wall-clock
   `est_child_latency` as cost. Under congestion, queueing inflates this
   estimate, creating a positive feedback loop: congestion → higher estimated
   cost → more rejection → lower goodput → even more rejection.

2. **Cost accounts for only the first hop.** Layer 2 at ingress meters the
   wall-clock latency of the *first* child call. The total downstream cost —
   fan-out, transitive children, local compute at each service — is invisible.
   This systematically under-estimates the capacity consumed per request.

3. **Layer 1 ignores child call duration.** The deadline feasibility check
   compares `now` against `e2e_deadline - est_remaining_floor`, where
   `est_remaining_floor` is the estimated time *after* the child returns. It
   does not include the child call itself, so a request can pass feasibility,
   wait 50ms for a child RPC, and only then discover it's too late.

4. **Feasibility check runs too late.** Layer 1 only fires at
   `before_child_rpc`. By that point the request has already been admitted,
   queued, scheduled, and partially processed — potentially consuming
   significant capacity before the first rejection opportunity.

## Design

### Overview

The v2 design makes four changes to the estimation and admission pipeline.
The two-mode explore/exploit structure (anchor_20) is retained — OPAL
conclusively demonstrated that alternatives regress. The 0-injection on
early-return is also retained — when `abort_slo` triggers, the entire request
chain bails immediately, making 0 an accurate remaining-time estimate.

### Change 1: Accumulate total compute cost across the call tree

**Problem:** Layer 2's cost signal sees only the first child's wall-clock
latency, missing fan-out amplification and conflating queueing with work.

**Design:** Each service adds its own `compute_time_us` (measured by the
existing `est_compute_latency` tracking in `before_poll`/`after_poll`) to a
running total in the request context. When the response propagates back,
`ResponseMeta` carries the accumulated compute cost of the entire downstream
subtree.

At ingress, the total accumulated cost is fed into `record_completion` for
goodput tracking. For admission decisions, an EMA of accumulated cost per
root API type replaces the current `est_child_latency` lookup.

**Why compute-only:** Including queueing time would reintroduce the positive
feedback loop (congestion → inflated cost → over-rejection). Layer 1's
deadline feasibility check already handles the "queueing made this request
infeasible" case using wall-clock estimates, so Layer 2 does not need to
redundantly account for queueing.

**Why total tree, not first hop:** A request that does 1ms of compute at the
ingress child but triggers 50ms of compute across 20 downstream services
consumes 50ms of system-wide capacity. Metering only the first hop
under-estimates by 50x, leading to over-admission.

### Change 2: Key Layer 2 estimates by root API type

**Problem:** Different ingress API types have very different downstream call
patterns. A `Search` request may fan out to 10 services, while a
`GetProfile` request calls 2. Using a single cost estimate (or keying by
local method) fails to capture this.

**Design:** Add a `root_method` field to `Context`. Set it at ingress
(hop_count == 0) to the ingress API's method name. Propagate it unchanged
through all downstream hops. Key the accumulated-cost EMA by `root_method`
so that each ingress API type has its own cost distribution.

This also enables the early feasibility check (Change 4) to use per-root-API
total latency estimates.

### Change 3: Include child call estimate in Layer 1 feasibility check

**Problem:** Layer 1 checks `now > e2e_deadline - est_remaining_floor` but
`est_remaining_floor` only covers post-child work. A request can pass this
check, spend most of its remaining time budget on the child call itself, and
only then discover it's infeasible.

**Design:** Tighten the feasibility check to:

```
now + est_child + est_remaining_floor > e2e_deadline
```

where `est_child` is the wall-clock child call estimate from
`est_child_latency` (the existing tracker). This is correct because the
feasibility question is "will this request finish in time?" — a wall-clock
question.

Note: Layer 1 continues to use wall-clock estimates (for deadline
feasibility), while Layer 2 uses compute-time estimates (for capacity
metering). These are answering different questions and the different units
are intentional.

### Change 4: Early feasibility check at request arrival

**Problem:** The feasibility check only runs at `before_child_rpc`. By that
point, the request has consumed queue time, scheduling time, and local
compute. This is wasted capacity if the request was always going to be
infeasible.

**Design:** Add a new latency map, `est_method_latency`, tracking total
wall-clock time per method keyed by root API type (from request arrival to
completion). At `before_poll` (or `Layer::new`), check:

```
now + est_method_latency[root_method] > e2e_deadline
```

If the estimated total method latency exceeds the remaining time budget,
reject immediately before any compute work begins. This catches requests
that are already doomed due to queueing delays, without waiting for them to
reach their first child RPC call.

## Interaction between layers

The four changes create a layered defense:

```
Request arrives
  │
  ├─ Change 4: Early feasibility (before_poll/new)
  │   "Given typical total latency for this root API type,
  │    can this request plausibly finish in time?"
  │   Signal: est_method_latency[root_method] (wall-clock)
  │   Scope: every hop
  │
  ├─ Layer 1: Per-child feasibility (before_child_rpc)
  │   "Given est_child + est_remaining for this specific child call,
  │    can this request finish in time?"
  │   Signal: est_child + est_remaining_floor (wall-clock)
  │   Scope: every hop
  │
  └─ Layer 2: Capacity budget (before_child_rpc, ingress only)
      "Does the system have compute budget to process this request?"
      Signal: EMA of accumulated compute cost per root API type
      Scope: ingress only (hop_count == 0)
```

Each layer answers a distinct question:
- **Change 4:** coarse, early filter — "is there enough time?"
- **Layer 1:** precise, per-child filter — "is there enough time for this specific call path?"
- **Layer 2:** capacity filter — "is there enough system capacity?"

## Implementation requirements

### New context field: `root_method`

Add to `Context` and `ContextBuilder` in `libs/masa-core/src/context.rs`:

```rust
#[serde(default)]
pub root_method: Option<String>,
```

Set at ingress (hop_count == 0) in `PolicyHooks::begin`. Propagate via
`ContextBuilder::from` — the existing child context construction in
`hooks.rs` already copies most fields; `root_method` follows the same
pattern.

### Accumulated compute cost in ResponseMeta

Extend `ResponseMeta`:

```rust
pub struct ResponseMeta {
    pub compute_time_us: u64,       // existing: local compute
    pub accumulated_compute_us: u64, // new: total tree compute
    pub utilization: f32,
    pub max_downstream_util: f32,
}
```

In `finalize`, set `accumulated_compute_us` to local compute plus the sum of
`accumulated_compute_us` from all child responses.

### New latency map: `est_method_latency`

Add to `EstServerState`:

```rust
pub est_method_latency: Arc<LatencyMap<E>>,
```

Keyed by root API type (from `ctx.root_method`). Tracked at `finalize` with
total wall-clock request duration. Queried at `before_poll` or `new` for
early feasibility.

### Layer 2 cost signal change

In `admission_check`, replace:

```rust
let est_child_cost = est_server.est_child_latency.get_estimate(key).unwrap_or(0);
```

with a lookup from the accumulated-compute-cost EMA keyed by root API type.

In `record_completion`, replace `est_child_latency` lookup with the actual
`accumulated_compute_us` from the response.

## What is NOT changing

- **Two-mode explore/exploit** — OPAL (14 iterations) validated that the
  binary mode-switch with rejection_ema is the most stable design. All
  alternatives (continuous probe scaling, ER-gated switching, concurrency
  limiters, tau tuning) regressed.

- **0-injection on early-return** — When a child returns DeadlineExceeded
  with `abort_slo`, the entire request chain bails immediately. Tracking 0
  into `est_after_child_latency` is accurate and prevents estimate inflation.

- **Token bucket structure** — The goodput-tracking token bucket with EMA
  refill rate, burst cap, and dynamic floor is retained. Only the cost
  signal fed into it changes.

- **Deadline tightening (sched_pred)** — Child deadline = parent deadline -
  est_remaining. Unchanged.

- **EDF reprioritization (before_poll)** — Dynamic priority based on
  remaining time to deadline. Unchanged.
