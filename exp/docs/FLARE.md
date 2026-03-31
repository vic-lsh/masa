# FLARE — Revised admission control algorithm design

## Context

The admission control algorithm went through two experiment series that
identified fundamental design tensions:

- **SURGE** (socialnet): Tuned the token bucket parameters
  (UTIL_TARGET, ADJUST_RATE, MAX_BURST_SECS) from inactive to effective.
  Achieved monotonically increasing goodput under overload (+422 vs baseline
  at 2000 RPS). Discovered a 2-second oscillation that rate-tuning cannot fix
  (iterations 3-6 all failed). Also discovered that `est_child` (wall-clock)
  as token bucket cost crashes Hotel (I/O-heavy calls exceed max budget).

- **EMBER** (socialnet): Validated the 2-layer refactor. Proved that
  `est_compute_latency` (single-hop CPU) is ~10-20x too low for fanout
  services (ember_1 regression). Restored `est_child_latency` as cost
  (ember_2 recovered surge_8 performance). Added dynamic burst floor
  (ember_3) to prevent the Hotel crash.

This document specifies the revised algorithm that consolidates learnings
from both series and addresses the remaining open problems.

## Assumptions

- **Databases are well-provisioned.** MongoDB and other backing stores are
  never the bottleneck. They may incur high latency due to processing costs,
  but they don't become overloaded. This means bottlenecks are always within
  the microservice layer (CPU).
- **DB latency is roughly constant under load.** Because DBs are
  well-provisioned, their response times don't inflate with system load.
  I/O wait in `est_child` is therefore a fixed offset that doesn't change
  relative request rankings.

## Design goal

Maximize goodput under overload. When shedding is necessary, preferentially
reject expensive requests and admit cheap ones — serving 10 cheap requests
yields more goodput than 1 expensive one.

---

## Algorithm

### Layer 1: Deadline feasibility (every hop, every child RPC)

```
if est_remaining_floor(parent→child key) > time_left(e2e_deadline):
    shed
```

Unchanged from current implementation. Sheds doomed requests before they
waste resources. Runs at every hop (not just ingress) because shedding a
doomed request deep in the call graph still saves the compute of that hop
and its children. Uses `e2e_deadline` (original, not tightened) because this
is a physical feasibility check.

### Layer 2: Token bucket admission (ingress only, hop_count==0)

#### Cost metric: `est_child_latency` (wall-clock child time)

The per-request cost is `est_child_latency(parent→child key)` — the
wall-clock time of the child RPC as observed by the caller.

**Why wall-clock, not compute?**

- `est_compute_latency` (single-hop CPU) is ~10-20x too low for fanout
  services. compose-post's local CPU is a few K µs, but the full call takes
  ~45K µs. ember_1 proved this: the token bucket became a no-op.
- Cumulative downstream compute doesn't work because resources aren't
  fungible across microservices. A request with 50K µs aggregate compute
  spread across 10 cheap services is less harmful than one with 5K µs
  hitting a single service at 95% utilization. Aggregate compute gives the
  wrong shedding ranking when bottlenecks are localized.
- `est_child` (wall-clock) is a **consistent monotonic proxy** for
  bottleneck impact under the well-provisioned-DB assumption:
  1. When a microservice CPU is bottlenecked, its queue builds up, inflating
     wall-clock response times for requests that traverse it.
  2. Requests traversing the bottleneck see inflated `est_child`; requests
     avoiding it don't. This naturally partitions requests into
     "bottleneck-touching" (high cost, shed first) and "bottleneck-avoiding"
     (low cost, admitted).
  3. Among bottleneck-touching requests, cost scales with impact — 3 serial
     calls through the bottleneck accumulate 3x the queueing delay.
  4. The I/O wait component (DB latency) is roughly constant under load
     (well-provisioned DBs), so it shifts all costs by a fixed offset
     without changing relative rankings.
  5. The rate signal (utilization) controls *how many* requests are admitted;
     the cost controls *which ones* get shed. A constant offset in cost
     doesn't affect the ranking.

#### Budget cap (max_budget)

```
max_budget = max(rate × MAX_BURST_SECS, largest_recent_cost × 2.0)
```

The budget (in µs) is clamped to `max_budget` after every refill. This
serves two purposes that are in tension:

1. **Prevent accumulation-driven oscillation.** Without a cap, budget
   surplus accumulates during low-load or "good" periods. When overload
   arrives (or during the next oscillation cycle), the stored surplus lets
   the controller admit a burst that overwhelms downstream, causing a "bad"
   period. SURGE iterations 3-6 proved that rate-tuning (AIMD, rate freeze,
   additive increase) cannot fix this oscillation — the root cause is
   surplus accumulation, not rate dynamics. surge_7 confirmed:
   MAX_BURST_SECS=0.005 reduced oscillation amplitude and improved goodput
   by +24 to +59 at overload.

2. **Ensure no request is permanently inadmissible.** A fixed cap
   `rate × MAX_BURST_SECS` can be smaller than a single request's cost.
   For example, Hotel Reservation: `est_child=115K µs` but
   `max_budget=75K µs` at max rate. The budget can *never* accumulate
   enough to cover the cost, making the request permanently inadmissible.
   This caused the Hotel crash in SURGE. The dynamic floor
   `largest_recent_cost × 2` ensures the cap always accommodates the most
   expensive request recently observed. The ×2 margin allows one expensive
   + one cheap request in the same burst window.

`largest_recent_cost` is tracked as an EMA over admitted request costs,
providing a stable floor that doesn't jump per-request.

#### Rate signal

The token bucket refill rate adjusts based on downstream stress:

```
effective_util = max(max_cpu_util_across_all_apis, er_pseudo_util)
if effective_util > UTIL_TARGET:
    rate *= 1.0 - ADJUST_RATE × elapsed
else:
    rate *= 1.0 + ADJUST_RATE × elapsed
```

Two input signals:

**1. CPU utilization: `max(util across all APIs)`**

`max_downstream_util` is propagated per-API via ResponseMeta, tracked in
`BottleneckTracker` with staleness decay. The rate signal uses the maximum
across all APIs (`get_max()` method), not the calling API's individual util.

Why max across APIs: with a shared bucket, all APIs compete for the same
budget. If any path is stressed, the rate should decrease. The
over-throttling concern (unstressed APIs getting throttled) is mitigated by
the cost metric — requests on uncongested paths have low `est_child`
(no queueing inflation), so they draw little budget and still get admitted
even at reduced rate.

Per-API `get(api)` is preserved for diagnostics and future use (per-API
bucket merging).

**2. ER-rate backstop: `er_pseudo_util`**

CPU utilization alone can miss overload when requests are shed before
consuming CPU (e.g., Layer 1 deadline feasibility or abort_slo shed
requests that never execute, keeping CPU idle-looking while goodput is
low).

```
er_pseudo_util = min(1.0, er_fraction / ER_THRESHOLD)
```

`er_fraction` is an EMA (α=0.05) over binary observations:
- 1.0 for each Layer 1 or abort_slo early return
- 0.0 for each successful completion

**Layer 2 (token bucket) rejections are excluded** from the ER count to
avoid a positive feedback loop (reject → ER rate up → throttle harder →
more rejections).

**α=0.05 (slow EMA) is intentional.** The system exhibits a ~2-second
oscillation cycle. A fast EMA would track the oscillation itself
(amplifying it). A slow EMA averages over multiple cycles, reflecting the
sustained overload state rather than the instantaneous phase. This provides
damping rather than amplification.

#### Admission decision: probabilistic smoothing

```
if budget >= cost:
    admit, debit cost
else:
    p = (budget / cost) ^ PROB_SMOOTH
    admit with probability p; debit cost if admitted
```

- `PROB_SMOOTH=1.0`: linear probabilistic (full smoothing)
- `PROB_SMOOTH→∞`: approaches binary reject (current behavior)
- `PROB_SMOOTH=0.0`: never rejects (opt-out for experiments)

**Why probabilistic?** The current binary admit/reject creates sharp phase
transitions that drive the 2-second oscillation. When budget is sufficient,
100% of requests are admitted (burst). When budget depletes, 0% are admitted
(collapse). Probabilistic admission smooths the transition: as budget
depletes, admission probability decreases gradually rather than falling off
a cliff.

Probabilistic behavior only applies in the `budget < cost` regime. When
budget is sufficient, admission is deterministic — no noise on the happy
path.

#### Bucket topology: shared

Single shared bucket across all APIs. Cheap requests naturally outcompete
expensive ones for admission.

Known limitation: if two APIs traverse completely disjoint service paths
with independent bottlenecks, overload on one path can drain budget that
starves the other. This is mitigated by the cost metric — requests on the
uncongested path have low `est_child` and draw little budget.

---

## Constants

All compile-time `const` values:

| Parameter | Value | Origin |
|-----------|-------|--------|
| `UTIL_TARGET` | 0.80 | surge_2: lowered from 0.92 to activate admission control before queue collapse |
| `ADJUST_RATE` | 2.0 | surge_3: 4x faster rate response, eliminated 1600 RPS dip |
| `MAX_BURST_SECS` | 0.005 | surge_7: minimal burst prevents accumulation-driven oscillation |
| `INITIAL_BUDGET_RATE` | 5M µs/s | surge_2: halved from 10M to reduce warmup inflation |
| `PROB_SMOOTH` | TBD | 1.0=linear probabilistic, high=binary, 0.0=disabled |
| `ER_THRESHOLD` | TBD | ER fraction at which pseudo-util saturates to 1.0 |
| `ER_ALPHA` | 0.05 | Slow EMA to average over oscillation cycles, not track them |
| `STALENESS_SECS` | 2.0 | Decay window for stale utilization reports |
| `STALENESS_DEFAULT` | 0.5 | Default util when no data available |

---

## Future work

### Per-API buckets with dynamic merging (deferred, important)

The shared bucket causes false coupling when APIs traverse disjoint service
paths. The ideal solution: each API gets its own bucket by default. When the
system detects that two APIs report high utilization from the *same*
downstream service, it merges their buckets so they compete fairly for the
shared bottleneck.

This requires propagating bottleneck *identity* (which service is hot), not
just magnitude (how hot). Current ResponseMeta only carries
`max_downstream_util` (a float). The identity signal could be a service ID
or hash of the bottleneck service name.

Design criteria:
- Disjoint paths: independent buckets, no false coupling
- Overlapping paths (shared bottleneck): merged bucket, cross-API
  prioritization with cost-based shedding
- Dynamic: merging/splitting responds to changing bottleneck topology at
  runtime

### Probabilistic admission tuning

`PROB_SMOOTH` needs calibration via experiments. Start with 1.0 (linear),
measure oscillation amplitude vs binary baseline. If oscillation is
sufficiently damped, keep it. If goodput regresses due to noisy individual
decisions (cheap request rejected, expensive admitted by luck), increase
toward 2.0-3.0 for a steeper probability curve.

### ER_THRESHOLD calibration

`ER_THRESHOLD` defines when the ER-rate backstop saturates. Too low
(e.g., 0.05): backstop triggers under normal shedding at moderate overload,
over-throttling. Too high (e.g., 0.5): backstop only triggers at extreme
overload, defeating the purpose. Start with 0.2 (20% ER rate →
pseudo-util=1.0) and calibrate empirically.
