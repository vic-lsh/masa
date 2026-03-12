# QUARTZ — Progressive Cost-Aware Admission Control

## Problem

Masa's current admission control mechanism (`emp_admission`) fails on mixed-SLO workloads. It works well when one API type is clearly infeasible (coral_4: both APIs have 50ms SLO, but Search takes ~110ms → always infeasible, Reservation takes ~20ms → always feasible). It fails when both API types are feasible but have different SLOs and costs (coral_ext_2: Search=200ms SLO/110ms cost, Reservation=50ms SLO/20ms cost).

**The failure mode is a congestion-feedback monopoly:**
1. emp_admission tracks `P(complete | api, time_left_bucket)` independently per API
2. The API with the higher P monopolizes admission (Search: easy 200ms SLO → p≈1.0)
3. The tight-SLO API's P collapses (Reservation: tight 50ms SLO → transient failures drive p→0)
4. Once p collapses, only 5% probe floor admits the tight-SLO API. Recovery requires positive observations, but the few probes complete at only ~33% rate — insufficient to pull p back up.
5. The monopoly is self-reinforcing: the expensive API fills capacity, the cheap API starves.

This is not a tuning problem. 11 iterations of CORAL experimentation (adjusting probe floors, EMA asymmetry, update semantics, single-tier admission flags, floor-check bypass) all failed because the root cause is structural: **per-API completion rate tracking creates winner-take-all dynamics when APIs share backend resources.**

The result: at 800 RPS (well below system saturation of ~1400 RPS), emp_admission achieves only 403 goodput vs 799 for `prio_local,early`. It is shedding half the traffic unnecessarily.

**Goal:** Design an admission control mechanism that:
- Maximizes goodput across N API types with different SLOs and costs
- Has no per-API feedback loops that can collapse
- Works without static call graph descriptions
- Scales to arbitrary numbers of API types
- Sheds load as early as possible to minimize wasted work

## Context

### How Masa works today

**Scheduling:** `prio_local` assigns each request a local deadline based on its e2e deadline minus estimated remaining work. The modified single-threaded tokio runtime dequeues tasks by deadline (tighter deadline = higher priority). This naturally favors cheap requests — a request with 20ms of remaining work gets a tighter local deadline than one with 110ms, so it runs first.

**Early return (floor-based):** At each hop, before spawning child RPCs, the system checks `est_remaining > time_left`. If true, the request is shed immediately (early return with DeadlineExceeded). This becomes more precise at later hops as `est_remaining` covers less of the call graph.

**Empirical admission (emp_admission):** On top of the floor check, tracks `P(complete | api, time_left_bucket)` via asymmetric EMA (ALPHA_FALL=0.1, ALPHA_RISE=0.05). Requests in buckets 0–4 are admitted with probability `max(p, PROBE_FLOOR=0.05)`. This is the mechanism that fails on mixed SLO.

**Latency estimation:** `LatencyMeanVar` tracks per-method mean and variance of observed latencies via EMA (α=0.1). Used for computing `est_remaining` and local deadlines.

### Experimental evidence

**coral_4** (homogeneous SLO=50ms, both APIs):

| RPS  | prio_local,early | emv+emp | Delta |
|------|-----------------|---------|-------|
| 800  | 400.0           | 401.9   | +2    |
| 1400 | 460.9           | 700.2   | +239  |
| 2000 | 268.3           | 1002.3  | +734  |
| 4000 | 612.7           | 1997.5  | +1385 |

emp_admission achieves ~50% fraction (theoretical max: Search is infeasible at 50ms SLO, so admitting all Reservation = 50% of 50/50 mix).

**coral_ext_2** (mixed SLO: Search=200ms, Reservation=50ms):

| RPS  | prio_local,early | emv+emp | prio_oldest,early |
|------|-----------------|---------|-------------------|
| 800  | 798.5           | 402.8   | 798.5             |
| 1200 | 1188.6          | 607.6   | 1176.5            |
| 1400 | 1267.4          | 659.5   | 1201.4            |
| 1600 | 1411.2          | 15.3    | 1313.5            |
| 2000 | 214.8           | 19.6    | 549.8             |

emp_admission is strictly worse than both baselines at every load point above 400 RPS.

### What CORAL iterations tried and why they failed

| Iter | Approach | Result | Root cause of failure |
|------|----------|--------|-----------------------|
| 5 | Remove bucket-5 bypass | ❌ | Didn't break per-API feedback loop |
| 6 | Floor-first hybrid admission | Mixed | Fixed ext_2 but destroyed coral_4 win |
| 7 | Decaying cold-start exploration floor | ❌ | Still converges to one-class monopoly |
| 8 | Forced-probe flag (probes bypass all hops) | ❌ | Increased backend load, poisoned ALL estimates |
| 9 | Update only on ER outcomes (not deadline miss) | ❌ | Wrong diagnosis — requests shed at ingress before update step |
| 10 | emp_admitted single-tier (floor check active) | ❌ | Floor check killed Search at downstream hops |
| 11 | emp_admitted full bypass (skip all downstream checks) | ❌ | Congestion feedback unchanged; Reservation still ~7/s at 800 RPS |

**Core lesson:** Per-API P(complete) tracking is fundamentally incompatible with shared backends under mixed SLOs. Any mechanism that independently tracks per-API completion rates creates winner-take-all dynamics. The fix must eliminate per-API outcome tracking entirely.

## Proposed design: Progressive Cost-Aware Admission Control (PAC)

### Design principles

1. **Admission is a resource allocation problem, not a classification problem.** The question isn't "will this request complete?" — it's "does admitting this request maximize total goodput?" This requires reasoning about both feasibility and cost.

2. **Prefer cheap requests under overload.** A request that consumes 20ms of backend compute yields the same +1 goodput as one consuming 110ms. Under capacity constraints, admitting cheap requests maximizes goodput per resource unit. This extends prio_local's scheduling principle (prioritize tight-deadline requests) to admission control.

3. **No per-API state that can collapse.** Replace per-API P(complete) with per-API cost estimates (from the latency estimator, which updates from all observed requests) and a global overload signal (from piggybacked downstream utilization). Neither creates per-API feedback loops.

4. **Shed early, refine progressively.** The cheapest place to shed is the ingress (zero sunk cost). Later hops refine the decision with higher confidence (less remaining work to estimate). Each hop's floor check (`est_remaining > time_left`) becomes more precise as the request progresses.

5. **Estimates should reflect compute cost, not queue delay.** The priority scheduler assigns different queue delays to different requests based on their priority. Including queue delay in cost estimates conflates the request's intrinsic cost with the system's scheduling decisions. Two requests with identical compute time should have identical cost estimates and identical priority (given equal remaining time). Queue delay is a system property that changes with load; compute cost is a request property that's relatively stable.

6. **Downstream services self-report their state.** Instead of the ingress inferring downstream overload from latency inflation, each service reports its utilization in response metadata. This is more accurate (local knowledge), faster (no EMA lag), and disentangled from priority-mediated queue delay.

### Architecture

```
                        ┌─────────────────────────────┐
 Request ──►  Layer 1: Feasibility check (every hop)  │
              est_compute_remaining > time_left → SHED │
              Increasingly precise at later hops       │
                        └──────────┬──────────────────┘
                                   │ pass
                        ┌──────────▼──────────────────┐
              Layer 2: Capacity allocation (ingress)   │
              score = P(feasible) / est_compute        │
              Uses downstream utilization signals       │
              Shed low-score requests when overloaded   │
                        └──────────┬──────────────────┘
                                   │ admit
                        ┌──────────▼──────────────────┐
              Priority scheduling (prio_local)         │
              local_deadline = e2e_deadline             │
                             - est_compute_remaining   │
              Lower deadline = higher priority          │
                        └─────────────────────────────┘
```

### Layer 1: Feasibility check

Runs at **every hop** in the call chain. Unchanged from current floor-based early return:

```
shed if: est_compute_remaining > time_left
```

Where `est_compute_remaining` is the sum of estimated compute times for the remaining call chain, **excluding queue delay** (see "Compute time estimation" below). This check becomes progressively more precise at later hops because there's less remaining work to estimate.

### Layer 2: Capacity allocation

Runs at the **ingress** (first hop). Makes cost-aware shedding decisions when the system is overloaded.

**Overload detection:** Each downstream service reports its `utilization` in response metadata (piggybacked on every response). Each response also carries `max_downstream_utilization` — the highest utilization observed transitively in the call chain below. The ingress tracks the most recent `max_downstream_utilization` for each API type from its responses.

```
bottleneck_util[api] = last observed max_downstream_util from api's responses
```

When `bottleneck_util[api]` exceeds a threshold (e.g., 0.85), that API's call chain is under stress and shedding may be needed.

**Efficiency scoring:** For each incoming request:

```
est_compute = est_compute_remaining[api]
P_feasible  = P(request completes within time_left)
            = Φ((time_left - est_compute_mean) / est_compute_stddev)
score       = P_feasible / est_compute
```

`P_feasible` is computed from the latency estimator's mean and variance for the remaining call chain. Unlike emp_admission's P(complete), this is **computed from the distribution**, not tracked via EMA of outcomes. There is no per-API state that can collapse.

`est_compute` uses compute time only (excluding queue delay), making it stable across load levels. The priority scheduler naturally gives cheap requests less queue delay, but the cost estimate reflects the request's intrinsic resource consumption, not its historical scheduling luck.

**Admission decision:**

```
if bottleneck_util[api] < 0.85:
    ADMIT

// System stressed. Admit based on efficiency score.
if score > threshold:
    ADMIT
else:
    SHED
```

The `threshold` is feedback-controlled: if downstream utilization remains high, raise threshold (shed more); if utilization drops, lower threshold (admit more). This is a simple proportional controller that converges to the admission rate that keeps the system at capacity.

**Why this handles coral_ext_2 correctly:**
- At 800 RPS (below saturation): downstream utilization < 0.85 → admit everything → no Reservation starvation
- At 1400 RPS (overloaded): Reservation score = ~0.98/20ms = 0.049; Search score = ~0.91/110ms = 0.008. Reservation is 6× more efficient → threshold set between → Search shed first, Reservation admitted. Correct.

**Why this preserves coral_4:**
- Search with 50ms SLO: P_feasible ≈ 0 (est_compute=110ms >> time_left=50ms). Shed by Layer 1 floor check before Layer 2 even runs. Same behavior as current.

### Compute time estimation

#### Task lifecycle and types of delay

A task in Masa's single-threaded tokio runtime cycles through three phases:

```
spawn ──► [QUEUE WAIT] ──► poll ──► [I/O WAIT] ──► [QUEUE WAIT] ──► poll ──► ... ──► complete
           (BinaryHeap)    (CPU)    (child RPC)     (BinaryHeap)    (CPU)
```

1. **Queue wait**: task sitting in the BinaryHeap, waiting to be dequeued by the scheduler. Occurs both on initial spawn and on every re-enqueue after I/O completes. Duration depends on how many higher-priority tasks are ahead.

2. **Poll execution**: the runtime is actively polling the task's future — local CPU work (processing data, preparing child RPC calls, handling responses). This is the actual resource consumed by this service.

3. **I/O wait**: task returned `Pending`, waiting for a waker (typically a child RPC response). The task is NOT in the queue and NOT consuming CPU. This time is accounted for in the *child service's* total time, not the parent's compute.

**Compute time** at a service = sum of all poll execution durations for this task. This is the resource the task consumes at this service, independent of queue depth or scheduling decisions.

**Total handler time** = spawn to complete = sum of queue waits + sum of poll durations + sum of I/O waits.

#### Existing runtime infrastructure

The modified tokio runtime already tracks per-poll queue delay via `TraceTimer` in each task's `Header` (`libs/tokio/tokio/src/runtime/task/core.rs`):

- `set_enqueue_time()`: called every time a task enters the BinaryHeap (initial push and re-enqueue)
- `record_queue_lat()`: called every time a task is popped for polling
- `obtain_task_queue_latency()`: public API for a running task to read its most recent queue wait

**Current limitation:** `TraceTimer` only retains the most recent queue latency (overwritten on each pop). To measure total compute time, we need cumulative tracking.

Additionally, `masa-core` already defines a `QueueLatencies` struct distinguishing initial vs resume delay:
```rust
pub struct QueueLatencies {
    pub initial: u64,    // Initial queue delay (microseconds)
    pub resume: u64,     // Resume queue delay (microseconds)
}
```

#### Required changes

Extend `TraceTimer` to accumulate across polls:

```rust
pub(crate) struct TraceTimer {
    last_enqueue: Option<Instant>,
    last_poll_start: Option<Instant>,
    cumulative_queue_us: u64,    // NEW: sum of all queue waits
    cumulative_poll_us: u64,     // NEW: sum of all poll durations
}
```

- On dequeue (existing `record_queue_lat`): accumulate `cumulative_queue_us += now - last_enqueue`. Set `last_poll_start = now`.
- On yield/completion: accumulate `cumulative_poll_us += now - last_poll_start`.

Then: `compute_time = cumulative_poll_us` at task completion.

This gives each completed task a precise decomposition:
- `compute_time` = CPU work consumed at this service
- `queue_delay` = total time spent waiting in the BinaryHeap
- `io_wait` = total_handler_time − compute_time − queue_delay

#### How compute time flows through the call graph

Each service reports `compute_time` in its response metadata. The parent uses this to build `est_compute_remaining`:

```
est_child_compute[method] ← EMA(child_response.compute_time)
est_local_compute ← EMA(local_compute_time)
est_compute_remaining[api] = sum(est_child_compute[child] for child in api's remaining calls)
                           + est_local_compute
```

Note: `est_child_compute[method]` tracks the child SERVICE's compute time, not the parent's I/O wait for that child. The parent's I/O wait includes the child's queue delay + compute + the child's own I/O — but only the child's `compute_time` is reported back. This ensures the parent's `est_compute_remaining` reflects pure resource consumption across the entire remaining call chain, without any queue delay from any hop.

**Why exclude queue delay from cost estimates:**
- Queue delay depends on priority assignment, which depends on cost estimates — including it creates circularity
- Two requests with identical compute time should have identical cost and priority (given equal remaining time)
- Compute time is stable across load levels; queue delay is volatile. Cost estimates based on compute time require less frequent updates and less probing.

**Why include queue delay in P(feasible):**
P(feasible) asks "will this request meet its deadline?" — queue delay is real time consumed from the budget. However, P(feasible) doesn't need an explicit queue delay estimate. It's computed from the latency distribution which reflects the request's actual time-to-completion. The priority scheduler ensures that high-priority (cheap) requests experience less queue delay, so their completion time distribution naturally shifts left. P(feasible) captures this implicitly.

### Utilization measurement

Each service tracks its own utilization — the fraction of time the CPU is busy:

```
utilization = busy_time / (busy_time + idle_time)    // rolling window, e.g., 1 second
```

In Masa's single-threaded tokio runtime, the event loop either polls tasks (busy) or parks waiting for I/O events (idle). Utilization measures what fraction of the runtime's capacity is consumed.

Implementation: the modified tokio runtime adds two accumulators: time spent entering `poll()` (busy) and time spent in `park()` (idle). The ratio is computed over a rolling window.

Key properties:
- `utilization < 0.85`: service has spare capacity. Queues stay short.
- `utilization → 1.0`: service is at capacity. Any additional work increases queue delay.
- Independent of request mix: if a service handles requests from multiple APIs, utilization reflects total load.
- Local measurement: each service knows its own state precisely, no inference needed.

### Piggybacked probing via response metadata

Every response carries two fields:

```
MasaResponseMeta {
    compute_time_us: u64,         // this request's compute time at this service
    utilization: f32,             // this service's current utilization
    max_downstream_util: f32,     // max utilization observed in call chain below
}
```

**`compute_time_us`**: This request's actual processing time, excluding queue wait. Used by the parent to update `est_child_compute[method]`.

**`utilization`**: This service's current CPU utilization. Allows the parent (and transitively, the ingress) to know where capacity pressure exists.

**`max_downstream_util`**: The maximum of this service's own utilization and the `max_downstream_util` from all child responses. Propagated transitively up the call chain so the ingress sees the bottleneck utilization without needing to know the call graph topology.

**Cross-API observation sharing:** When multiple APIs share a backend service, ANY API's traffic to that service provides fresh observations. If Search and Reservation both call Rate, Reservation's calls to Rate keep the Rate utilization signal fresh even if Search is being shed. The ingress learns about shared backend pressure through whatever API is currently sending traffic.

**Stale signals for shed APIs:** If an API is fully shed, the ingress receives no responses for it, so `bottleneck_util[api]` goes stale. Two mechanisms handle this:

1. **Score-based readmission.** `est_compute` for the shed API is stable (compute time doesn't change when idle). If the global `threshold` drops (because admitted APIs' bottleneck signals show improving conditions), the shed API's score exceeds the threshold and it gets readmitted. Its first response immediately provides fresh `max_downstream_util`.

2. **Staleness decay.** If an API's bottleneck signal hasn't been updated for N seconds, gradually decay it toward a neutral value (e.g., 0.5). This ensures the ingress periodically reconsiders shed APIs without requiring dedicated probe traffic. An idle downstream service is by definition unsaturated; the first readmitted request updates the signal within one RTT.

### Priority computation

Under prio_local, local deadline computation uses **compute time only**:

```
local_deadline = e2e_deadline - est_compute_remaining
```

This ensures priority reflects the request's intrinsic urgency (how much work remains vs. how much time is left), not historical queue dynamics. Two requests with identical compute remaining and identical time left get identical priority regardless of what queue delays they've historically experienced.

The priority scheduler handles queue dynamics at runtime: high-priority tasks are dequeued first, naturally experiencing less queue delay. This is a runtime consequence of correct priority assignment, not something the priority formula should pre-compensate for.

## Design alternatives considered

### A1: Per-API P(complete) with tuned parameters (CORAL iterations 1–11)

The current emp_admission design: track `P(complete | api, bucket)` via EMA of outcomes, admit probabilistically.

**Why rejected:** Structurally incompatible with mixed-SLO workloads. Per-API completion rates create winner-take-all dynamics when APIs share backends. 11 iterations of CORAL experimentation (varying probe floors, EMA asymmetry, update semantics, single-tier admission, floor-check bypass) all failed because the feedback loop between admission decisions and completion observations is fundamentally destabilizing. When one API monopolizes capacity, the other's completion rate collapses with no recovery path.

### A2: Cost-weighted P(complete) (initial QUARTZ hypothesis H5)

Keep per-API P(complete) tracking but weight admission by `P(complete) / est_cost`. Higher efficiency → higher admission probability.

**Why rejected:** Still relies on per-API P(complete) as the feasibility signal. The cost weighting improves the ADMISSION PRIORITY (cheap requests admitted first), but the underlying P(complete) tracking still creates feedback loops. If P(Reservation) collapses, the cost weighting can't fix it — `0.0 / 20ms = 0` is still zero regardless of how cheap Reservation is. The new design eliminates per-API outcome tracking entirely, computing P(feasible) from the latency distribution instead.

### A3: Higher probe floor / reversed EMA asymmetry (QUARTZ H1, H2)

Increase PROBE_FLOOR from 0.05 to 0.15–0.25, or flip EMA asymmetry so recovery is faster than collapse.

**Why rejected:** These are parameter tweaks to a structurally broken mechanism. Higher probe floor risks coral_4 regression (more wasted work on infeasible Search probes: 20% × 2000 Search/s × 110ms = 44 CPU-seconds/s). Reversed asymmetry slows collapse when entering overload. Neither eliminates the per-API feedback loop — they just shift the equilibrium point, which may not be stable across diverse workloads.

### A4: Load-gated activation (QUARTZ H3)

Only activate emp_admission when `goodput_fraction < threshold`. Below threshold, admit everything.

**Why rejected as standalone:** Fixes the underload case (800 RPS) but doesn't fix the mixed-SLO problem at true overload (1400+ RPS). Once activated, the same monopoly dynamics take over. Also introduces a tuning parameter (threshold) and hysteresis risk (oscillation near the threshold). However, the idea of overload gating is incorporated into the proposed design via downstream utilization signals — which are more precise (per-service, not global) and avoid the threshold discontinuity.

### A5: Estimates including queue delay (no decomposition)

Use the latency estimator as-is, with queue delay included in all estimates. The priority scheduler gives cheap requests less queue delay, so their estimates are naturally lower — providing implicit cost differentiation.

**Why rejected for cost estimates and priority:** Queue delay in cost estimates creates circularity — priority depends on est_compute_remaining, which includes queue delay, which depends on priority. Two requests with identical compute time would get different priorities based on historical queue dynamics, not their intrinsic properties. Additionally, queue delay is volatile (changes with load), so cost estimates including queue delay require more frequent probing to stay current. Compute-only estimates are stable and need less probing.

**Partially accepted for P(feasible):** The completion time distribution naturally reflects priority-mediated queue dynamics. P(feasible) doesn't need explicit queue delay estimates — the latency distribution captures the actual time-to-completion including whatever queue delay the priority scheduler assigns.

### A6: Explicit call graph knowledge for bottleneck attribution

Require the ingress to know which APIs call which downstream services (Hotel already has this for prio_local). When a specific service reports high utilization, shed APIs that use that service.

**Why rejected:** Requires static call graph descriptions that must be maintained as the application evolves. Doesn't generalize to dynamic or polymorphic call graphs. The proposed design avoids this by transitively propagating `max_downstream_util` through response metadata — the ingress learns each API's bottleneck utilization from its own responses, without needing to know the call graph topology.

## Experiment series: quartz_1, quartz_2, ... (hotel)

### Baseline: quartz_1 (mixed-SLO eval, coral_ext_2 config)

**Config:** Based on coral_ext_2 — Search SLO=200ms, Reservation SLO=50ms, RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000].

**Policies:**
- `prio_oldest,early` — external baseline (TailClipper)
- `prio_local,early` — scheduling-only baseline
- `prio_local,est_mean_var,early` — EMV without admission control
- `prio_local,est_mean_var,early,adctl` — QUARTZ admission control

**Code state:** commit cf3c1eb0 (feat(adctl): replace emp_admission with progressive cost-aware admission control)

**Goal:** Verify that adctl does NOT cause the emp_admission collapse on mixed-SLO workloads. Compare against EMV-only and baselines across the full RPS sweep.

### Results: quartz_1

**Status:** Complete ✅ — adctl eliminates emp_admission collapse and is best policy at deep overload.

| RPS  | prio_oldest,early | prio_local,early | emv (no adctl) | emv+adctl | adctl vs oldest | adctl vs emv |
|------|------------------:|-----------------:|---------------:|----------:|----------------:|-------------:|
| 100  | 99.8              | 99.9             | 99.8           | 99.8      | 0.0             | 0.0          |
| 400  | 399.3             | 399.3            | 399.3          | 399.2     | 0.0             | 0.0          |
| 800  | 798.6             | 798.6            | 798.6          | 798.6     | 0.0             | 0.0          |
| 1200 | 1180.9            | 1184.2           | 1185.4         | 1179.9    | -1.0            | -5.5         |
| 1400 | 1192.8            | 1271.5           | 1279.8         | 1276.9    | +84.1           | -2.9         |
| 1600 | 1266.9            | 1428.7           | 1426.7         | **1461.9**| **+195.0**      | **+35.2**    |
| 1800 | 945.9             | 612.1            | 969.2          | **1073.2**| **+127.3**      | **+104.0**   |
| 2000 | 705.1             | 650.5            | 214.4          | **1005.8**| **+300.6**      | **+791.4**   |

**Key findings:**

1. **emp_admission collapse eliminated.** adctl achieves 798.6 at 800 RPS (old emp_admission: 403). No per-API feedback loop starvation.

2. **Best policy at all RPS >= 1600.** Massive wins at deep overload: +195 vs TailClipper at 1600, +301 at 2000. EMV-only collapses to 214.4 at 2000 but adctl sustains 1005.8.

3. **Higher-quality shedding.** adctl has 839 ER/s at 2000 vs EMV's 1724 ER/s, yet 5x the goodput. Ingress admission prevents wasted downstream work, breaking the EMA cascade.

4. **No regression** at any load point (1200 delta of -5.5 is within noise).

5. **CPU profiles identical** across policies — adctl overhead is negligible. reservation-service confirmed as bottleneck (~100% CPU).

**Comparison vs old emp_admission (from coral_ext_2):**

| RPS  | emp_admission | adctl   | Delta    |
|------|-------------:|--------:|---------:|
| 800  | 402.8        | 798.6   | **+395.8** |
| 1200 | 607.6        | 1179.9  | **+572.3** |
| 1400 | 659.5        | 1276.9  | **+617.4** |
| 1600 | 15.3         | 1461.9  | **+1446.6**|
| 2000 | 19.6         | 1005.8  | **+986.2** |

### Next experiments needed

1. **quartz_2: coral_4 regression test** — both APIs SLO=50ms. Verify adctl preserves infeasible-API shedding (where emp_admission excelled: +734 at 2000 RPS). Design predicts Layer 1 handles this.
2. **quartz_3: Socialnet** — parallel fanout call graph. PINE showed divergent behavior between serial/parallel.
3. **quartz_4: Non-monotonic load** — test robustness to load swings and EMA staleness.

## Iteration 1: Time-based threshold decay (experiment quartz_2)

**Status:** Regression ❌ — reverted (git revert 7b345bf2 of code commit 33afd731)

### Change
Replace per-call threshold decay (×0.99 per `should_admit()` call) with time-based exponential decay. The current implementation decays threshold by 0.99 on every request, which at 2000 RPS means 0.99^2000 ≈ 0 within one second — the threshold collapses to zero the instant utilization drops below 0.85, causing immediate re-overload and the observed oscillation pattern.

The fix: track `last_update_time` in the AdmissionController. On each call, compute `elapsed_secs` since last update and apply `threshold *= decay_rate.powf(elapsed_secs)` where `decay_rate` is calibrated for a ~1-2 second half-life. Similarly, make the raise proportional to elapsed time: `threshold += raise_rate * elapsed_secs`. This ensures the controller behaves identically regardless of request volume.

### Hypothesis
The oscillation at 2000 RPS (alternating between ~1500 and ~250 goodput in ~10-15s cycles) is caused by the threshold controller's per-call decay being volume-dependent. At high RPS, decay is so fast that the threshold reaches zero within milliseconds of utilization dropping below 0.85, causing instant re-overload. Time-based decay will create a smooth, predictable descent that allows the system to find a stable equilibrium admission rate.

### Expected outcomes if hypothesis is correct:
1. The bistable oscillation at 2000 RPS should be eliminated or significantly dampened, yielding sustained goodput closer to the healthy-phase peak (~1500) rather than the current average (~1006).
2. Performance at 1600-1800 RPS should improve moderately (less micro-oscillation even if not visible in averages).
3. No regression at 800-1400 RPS (threshold controller is dormant when util < 0.85).

### Experiment design
Same config as quartz_1 (coral_ext_2 based: Search SLO=200ms, Reservation SLO=50ms) with RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000]. Only test `prio_oldest,early` and `prio_local,est_mean_var,early,adctl` to halve experiment time (we established EMV-only and prio_local,early baselines in quartz_1). Direct comparison to quartz_1's adctl numbers tells us if the fix helped.

### Actual Outcomes (quartz_2)

**Status:** Regression ❌

| RPS | adctl q2 | adctl q1 | delta | prio_oldest q2 | adctl q2 vs oldest |
|-----|------:|------:|------:|------:|------:|
| 100 | 99.8 | 99.8 | 0.0 | 99.8 | 0.0 |
| 400 | 399.3 | 399.2 | +0.1 | 399.3 | 0.0 |
| 800 | 798.6 | 798.6 | 0.0 | 798.6 | 0.0 |
| 1200 | 1185.2 | 1179.9 | +5.3 | 1180.9 | +4.3 |
| 1400 | 1262.9 | 1276.9 | -14.0 | 1192.8 | +70.1 |
| 1600 | 1417.0 | 1461.9 | -44.9 | 1266.9 | +150.1 |
| 1800 | 1001.2 | 1073.2 | -72.0 | 945.9 | +55.3 |
| 2000 | 624.7 | 1005.8 | -381.1 | 705.1 | -80.4 |

**Key findings:**

1. **Time-based decay worsens oscillation at 2000 RPS.** Goodput drops from 1006 → 625 (−381). Per-second trace shows boom-bust cycles: 17s collapse → 14s recovery (~1700 good) → 28s re-collapse. System spends ~75% of time in collapse.

2. **Regression at all overload RPS levels.** 1400 (−14), 1600 (−45), 1800 (−72), 2000 (−381). Pre-overload (≤1200) unaffected.

3. **Recovery proves mechanism CAN work.** During secs 38-51 at 2000 RPS, system achieves ~1700 goodput — exceptional if sustained. Problem is instability: RAISE_RATE=0.5/s too slow to re-raise threshold before flood overwhelms system.

4. **Root cause:** Boom-bust cycle. Threshold decays (HALF_LIFE=2s) → admission opens wide → system floods → RAISE_RATE=0.5/s too slow to close the gate → re-collapse. At moderate overload (1400-1600), decay also loosens admission unnecessarily.

**Decision: REVERT.** Code reverted via `git revert 33afd731` → commit 7b345bf2.

**Lessons for next iteration:**
- The per-call ×0.99 decay in the original code is actually functioning as a very aggressive time-based decay (effective half-life of milliseconds at high RPS). This is what creates the quartz_1 oscillation.
- Making decay slower (HALF_LIFE=2s) didn't help because the RAISE was also made proportional to time, making it too slow.
- The right fix may not be about decay rate at all — it may be about preventing the threshold from dropping too low (minimum threshold floor), or using integral/PID control instead of simple proportional.

## Iteration 2: Threshold floor to prevent full admission collapse (experiment quartz_3)

**Status:** Regression ❌ — reverted (git revert 859712b0 of code commit d9bc47b6)

### Change
Add a minimum threshold floor to the admission controller. Currently when `bottleneck_util` drops below 0.85, the threshold decays via ×0.99 per call to zero, opening admission fully and causing re-overload. The fix: clamp threshold to `max(threshold, MIN_THRESHOLD)` where MIN_THRESHOLD is a small positive value (e.g., 0.002). This keeps some selectivity even during the "recovery" phase, preventing the boom-bust cycle.

Additionally, keep the per-call raise/decay mechanics unchanged (they're fast and responsive). The floor just prevents the threshold from reaching zero.

### Hypothesis
The quartz_1 oscillation at 2000 RPS is caused by threshold→0 during recovery phases, which admits everything and re-floods the system. A floor prevents this: when utilization drops below 0.85, the threshold decays toward MIN_THRESHOLD instead of zero, maintaining selective admission. Requests with `score < MIN_THRESHOLD` (i.e., expensive + low feasibility) remain shed even during recovery, preventing the system from re-overloading.

The floor value should be small enough to not interfere with admission at moderate overload (1400-1600 RPS where all requests have high scores) but large enough to maintain selectivity at deep overload (2000 RPS where the system can't handle all traffic).

### Expected outcomes if hypothesis is correct:
1. At 2000 RPS: goodput should stabilize near the current healthy-phase peak (~1500) or at minimum improve over quartz_1's 1006 average.
2. At 1400-1600 RPS: no regression (floor is below normal scores at these load levels).
3. At ≤1200 RPS: no change (threshold controller dormant).

### Experiment design
Same config as quartz_1/2 (coral_ext_2 based). Two policies: prio_oldest,early + adctl.

### Actual Outcomes (quartz_3)

**Status:** Regression ❌ — catastrophic Search starvation

| RPS | adctl q3 (floor) | adctl q1 (original) | delta |
|-----|------:|------:|------:|
| 100 | 50.5 | 99.8 | **-49.3** |
| 400 | 198.7 | 399.2 | **-200.5** |
| 800 | 396.2 | 798.6 | **-402.4** |
| 1200 | 601.7 | 1179.9 | **-578.2** |
| 1600 | 799.6 | 1461.9 | **-662.3** |
| 2000 | 998.6 | 1005.8 | **-7.2** |

**Root cause:** MIN_THRESHOLD=0.002 prevents the threshold from ever dropping to zero. Once Search gets restricted (its efficiency score is lower due to higher compute cost), the floor prevents readmission — even at 100 RPS with zero contention. Result: 0% Search goodput, 100% Reservation goodput at ALL RPS levels. The ~50% fraction matches the 50/50 Search/Reservation traffic mix.

Ironically, oscillation IS eliminated at 2000 RPS (stddev=128 vs 689 in quartz_1), but only because Search is permanently dead.

**Decision: REVERT.** Code reverted via `git revert d9bc47b6` → commit 859712b0.

**Lessons:**
- The threshold is a REJECTION gate, not an admission probability. A floor prevents it from opening, not from closing.
- The threshold and efficiency score semantics are: score >= threshold → admit. Search score ≈ p_feasible / est_compute. With est_compute=110ms, Search score ≈ p/110000. For MIN_THRESHOLD=0.002, Search needs p ≥ 220 to be admitted — impossible since p ∈ [0,1].
- **Fundamental insight:** The score = p_feasible / est_compute has units of 1/microseconds. Search (110ms) scores ~5000x lower than Reservation (20ms) for equal feasibility. A single threshold can never treat them equitably unless the score accounts for this scale difference.

## Iteration 3: Normalize efficiency score to remove compute-cost scale bias (experiment quartz_4)

**Status:** Regression ❌ — reverted (git revert bb69c985 of code commit c81cc87d)

### Change
The efficiency score `p_feasible / est_compute` has a massive scale bias: Search (est_compute=110ms) gets scores ~5000x lower than Reservation (est_compute=20ms) for the same feasibility. This means ANY non-zero threshold will reject Search before Reservation. The fix: normalize the score so different API types are on a comparable scale.

Option A: Use `p_feasible` alone (drop the cost weighting). The priority scheduler already favors cheap requests via tighter local deadlines — double-counting cost in both priority AND admission may be why Search gets permanently rejected.

Option B: Use `p_feasible * log(1/est_compute)` or similar compression to reduce the 5000x gap to something manageable.

Going with **Option A** first: simplest change, and the priority scheduler's cost-awareness may be sufficient.

### Hypothesis
The oscillation at 2000 RPS is NOT caused by the threshold controller dynamics (Iterations 1-2 proved this). The real problem is that the efficiency score's compute-cost weighting creates such extreme score differences that the threshold can never find a stable equilibrium: when it's high enough to reject Search, it rejects ALL Search; when it's low enough to admit Search, it admits everything. Removing the cost weighting from the score (using p_feasible alone) eliminates this bimodality. The priority scheduler still handles cost-aware scheduling at runtime.

### Expected outcomes if hypothesis is correct:
1. Search and Reservation should have comparable admission scores, enabling the threshold to find a balanced equilibrium.
2. The oscillation at 2000 RPS should be reduced (threshold can rise/fall without creating all-or-nothing Search rejection).
3. Goodput should be at least as good as quartz_1 at all RPS levels, potentially better at 2000 RPS.
4. No regression at ≤1200 RPS.

### Experiment design
Same config as quartz_1/2/3 (coral_ext_2 based). Two policies: prio_oldest,early + adctl.

### Actual Outcomes (quartz_4)

**Status:** Regression ❌ — collapse at deep overload

| RPS | adctl q4 (p_feasible only) | adctl q1 (original) | prio_oldest q4 | q4 vs q1 |
|-----|------:|------:|------:|------:|
| 100 | 99.8 | 99.8 | 99.8 | 0.0 |
| 800 | 798.5 | 798.6 | 798.5 | 0.0 |
| 1200 | 1183.9 | 1179.9 | 1181.7 | +4.0 |
| 1400 | 1259.6 | 1276.9 | 1197.4 | -17.3 |
| 1600 | 1379.8 | 1461.9 | 1267.1 | -82.1 |
| 1800 | 1022.7 | 1073.2 | 1174.5 | -50.5 |
| 2000 | 214.1 | 1005.8 | 281.2 | **-791.7** |

**Key findings:**
1. Fixes Search starvation — 99.8% goodput at low loads (vs 50% in quartz_3). The p_feasible score is in [0,1] range, compatible with threshold_raise=0.01.
2. Catastrophic collapse at 2000 RPS (214 vs 1006). Without cost differentiation, system admits too many expensive Search requests → compute wasted → everyone suffers.
3. At 2000 RPS, Search goodput = 11.8 (vs 387.3 in quartz_1). System can't serve Search because there's no cost-aware shedding.

**Root cause analysis across all iterations:**

The REAL bug in the original code is the **scale mismatch** between scores and threshold dynamics:
- `score = p_feasible / est_compute_us` gives values ~10^-5 (e.g., 1.0/110000 ≈ 0.000009 for Search)
- `THRESHOLD_RAISE = 0.01` per call — 1000x larger than any possible score
- After ONE call with `util > 0.85`, threshold = 0.01, instantly rejecting everything
- `× 0.99` decay takes ~530 calls to return to score range (~0.26s of total rejection)
- This creates the boom-bust oscillation: overshoot → total rejection → util drops → slow decay → admit everything → re-overload

**The fix must normalize scores to [0,1] while preserving cost differentiation.**

**Decision: REVERT.** Code reverted via `git revert c81cc87d` → commit bb69c985.

## Iteration 4: Bounded cost-normalized scoring (experiment quartz_5)

**Status:** Pending

### Change
Replace `score = p_feasible / est_compute` with:
```
score = p_feasible / (1.0 + est_compute as f64 / REFERENCE_COMPUTE)
```
where `REFERENCE_COMPUTE = 50000.0` (50ms in μs).

This normalizes scores to [0, 1] while preserving bounded cost differentiation:
- Reservation (20ms): factor = 1/(1 + 20000/50000) = 0.71 → score ≈ 0.71 * p_feasible
- Search (110ms): factor = 1/(1 + 110000/50000) = 0.31 → score ≈ 0.31 * p_feasible
- Ratio: 2.3x (bounded), not 5500x (unbounded)

Scores are now in [0, 0.71] range, compatible with THRESHOLD_RAISE=0.01 per call. At 2000 RPS with util > 0.85 on ~50% of calls, threshold rises ~10/s → reaches 0.71 in ~70ms. With ×0.99 decay on the other ~1000 calls, threshold decays to ~0 in ~700 calls (~0.35s). This is faster cycling but with bounded overshoot — the threshold stays within the meaningful score range.

### Hypothesis
The oscillation in quartz_1 is caused by score/threshold scale mismatch: scores ~10^-5 vs threshold steps of 0.01. By normalizing scores to [0,1] with bounded cost weighting, the threshold controller operates within the actual score range. Cost differentiation is preserved (Search ~2.3x harder to admit than Reservation) but is bounded, so the threshold can find equilibrium between admitting both vs shedding both.

### Expected outcomes if hypothesis is correct:
1. No oscillation at 2000 RPS — threshold operates within score range, not overshooting by 1000x.
2. Cost-aware shedding preserved — Search shed before Reservation under overload (unlike quartz_4).
3. Goodput ≥ quartz_1 at all RPS levels, potentially much better at 2000 RPS (stable ~1500 vs oscillating average of 1006).
4. No regression at ≤1200 RPS.

### Experiment design
Same config. Two policies: prio_oldest,early + adctl.

### Actual Outcomes (quartz_5)

**Status:** Regression ❌ — still oscillating, worse at 1600/2000

| RPS | adctl q5 | adctl q1 | delta |
|-----|------:|------:|------:|
| 1200 | 1180.5 | 1179.9 | +0.6 |
| 1400 | 1256.6 | 1276.9 | -20.3 |
| 1600 | 1125.6 | 1461.9 | **-336.3** |
| 1800 | 1144.0 | 1073.2 | +70.8 |
| 2000 | 742.9 | 1005.8 | **-262.9** |

Score normalization brought scores to [0, 0.71] but per-call threshold dynamics STILL overshoot: 0.01 × 1000 calls/s = 10/s rise, vastly exceeding 0.71 max score. Per-second trace at 2000 shows 17s healthy phase (~1650 goodput) then permanent collapse — the healthy phase is better than quartz_1, but recovery fails.

**Critical insight across Iterations 1-4:** The fix requires BOTH:
1. Bounded scores (Iteration 4) — so threshold and scores are in the same range
2. Time-based dynamics (Iteration 1) — so controller is volume-independent

Iteration 1 failed because scores were ~10^-5 with time-based params calibrated for [0,1]. Iteration 4 fixed scores but kept per-call dynamics. Combining them should work.

**Decision: REVERT.** Code reverted via `git revert 266bfaff` → commit 48d846ae.

## Iteration 5: Bounded scores + time-based controller (experiment quartz_6)

**Status:** Pending

### Change
Combine Iteration 1 (time-based threshold) with Iteration 4 (bounded cost-normalized scoring). Two changes:

1. **Score normalization:** `score = p_feasible / (1 + est_compute / 50000.0)` — bounded [0, 0.71]
2. **Time-based threshold controller:** Track `last_update: Instant`. On each call:
   - `elapsed_secs = now.duration_since(last_update).as_secs_f64()`
   - If `bottleneck_util > 0.85`: `threshold += RAISE_RATE * elapsed_secs` (RAISE_RATE=1.0/s)
   - Else: `threshold *= 0.5_f64.powf(elapsed_secs / HALF_LIFE)` (HALF_LIFE=1.0s)
   - `threshold = threshold.clamp(0.0, 1.0)`
   - Update `last_update = now`

Parameters calibrated for score range [0, 0.71]:
- RAISE_RATE=1.0/s: reaches max score in ~0.7s under sustained overload (fast enough to respond)
- HALF_LIFE=1.0s: threshold halves every second during recovery (slow enough to prevent re-flood)

### Hypothesis
Per-call raise/decay is volume-dependent and creates overshoot at high RPS. Time-based dynamics are volume-independent but failed in Iteration 1 because scores were ~10^-5 (time-based params tuned for [0,1] range missed entirely). With bounded scores in [0, 0.71], time-based params can be correctly calibrated: RAISE_RATE=1.0/s ramps to full rejection in ~1s, HALF_LIFE=1.0s decays gradually over seconds. This prevents both the instant-overshoot problem (Iterations 1,4,5) and the too-slow-raise problem (Iteration 1).

### Expected outcomes if hypothesis is correct:
1. Stable goodput at 2000 RPS near the ~1650 seen in quartz_5's healthy phase.
2. No regression at 1400-1600 (threshold doesn't overshoot into the score range).
3. Cost-aware shedding: Search shed before Reservation under overload.
4. No regression at ≤1200 RPS.

### Experiment design
Same config. Two policies: prio_oldest,early + adctl.

### Actual Outcomes (quartz_6)

**Status:** Regression ❌ — permanent latch-up at deep overload

| RPS | adctl q6 | adctl q1 | delta |
|-----|------:|------:|------:|
| 1400 | 1286.0 | 1276.9 | +9.1 |
| 1600 | 1421.6 | 1461.9 | -40.3 |
| 1800 | 605.0 | 1073.2 | **-468.2** |
| 2000 | 294.3 | 1005.8 | **-711.5** |

Time-based controller creates permanent latch-up: once threshold tightens, sheds ~85% of traffic → almost no completions → controller never sees improvement → never relaxes. quartz_1's per-call oscillation at least recovers periodically (avg 1006 vs 294).

**Decision: REVERT.** Commit 13f8e766.

**Pattern across Iterations 1-5:**
- Stateful threshold controllers are fundamentally unstable for admission control
- Per-call: too fast → oscillation, but self-correcting
- Time-based: too slow recovery → permanent latch-up
- Floor: prevents recovery entirely
- Score normalization helps but doesn't fix the controller instability

## Iteration 6: Stateless utilization-proportional admission (experiment quartz_7)

**Status:** Pending

### Change
Eliminate the persistent threshold entirely. Replace with stateless computation on every call:

```
excess = ((bottleneck_util - UTIL_TARGET) / (1.0 - UTIL_TARGET)).clamp(0.0, 1.0)
threshold = excess * excess  // quadratic ramp: gentle near 0.85, aggressive near 1.0
score = p_feasible / (1 + est_compute / REFERENCE_COMPUTE)  // bounded [0, 0.71]
admit if score >= threshold
```

No persistent state. No raise/decay dynamics. Threshold is derived fresh from the current `bottleneck_util` on every call.

Remove `self.threshold: Mutex<f64>` and `last_update` from AdmissionController. Keep `BottleneckTracker` (tracks per-API utilization from responses).

### Hypothesis
All previous iterations failed because the stateful threshold controller creates feedback loops between admission decisions and utilization observations. Stateless admission eliminates this: the shedding intensity is a direct function of current utilization with no memory of past decisions.

The quadratic ramp provides:
- No shedding when util < 0.85 (all requests admitted at low load)
- Gentle shedding 0.85-0.92 (moderate overload: mostly feasible requests pass)
- Aggressive shedding 0.93-1.0 (deep overload: only high-score requests pass)

Cost-aware via bounded score normalization (Search penalized ~2.3x).

### Expected outcomes if hypothesis is correct:
1. No oscillation or latch-up — no state to oscillate or latch.
2. Smooth degradation at 1600-2000 RPS: goodput decreases gradually, not cliff-edge.
3. Cost-aware: Search shed preferentially over Reservation at moderate overload.
4. At least quartz_1-level goodput at 2000 RPS, potentially better (no wasted time in collapsed phases).

### Experiment design
Same config. Two policies: prio_oldest,early + adctl.

## Open questions

1. **Threshold controller tuning.** The feedback controller for Layer 2's `threshold` needs tuning (proportional gain, update rate). Too aggressive → oscillation. Too conservative → slow adaptation.

2. **P(feasible) computation.** Using the normal CDF approximation `Φ((time_left - mean) / stddev)` assumes normally distributed completion times. Real distributions are likely right-skewed. May need a different distributional assumption or a non-parametric approach.

3. **est_compute vs est_total for P(feasible).** The design uses compute-only estimates for cost/priority but needs total-time estimates for P(feasible) (since queue delay counts against the deadline). The P(feasible) computation may need access to both the compute distribution and an expected queue delay factor derived from utilization.

4. **Interaction with before_poll reprioritization.** The current prio_local implementation reprioritizes tasks via `before_poll`. If `local_deadline` changes to use `est_compute_remaining` (excluding queue delay), this affects the reprioritization logic. Need to verify compatibility.

5. **Staleness decay rate.** How quickly should stale bottleneck signals decay toward neutral? Too fast → unnecessary readmission of genuinely problematic APIs. Too slow → shed APIs stay shed too long when conditions improve.
