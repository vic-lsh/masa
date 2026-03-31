# EMBER — 2-layer admission control refactor validation (socialnet)

## Key questions
- Does the 2-layer admission control refactor (commit 4fefbd98) regress SocialNet goodput compared to the prior 3-layer implementation?
- Is `est_compute_latency[parent→child key]` a suitable token bucket cost for CPU-bound workloads like SocialNet, where compute ≈ wall-clock?
- Are there parameter tuning issues with the refactored code that need adjustment?

## Experiment series: ember_1, ember_2, ... (socialnet)

## Pre-refactor baseline: surge_8

surge_8 was run on the old 3-layer code (before commit 4fefbd98). Results:

| RPS | FIFO | pred (no ac) | **pred+ac_pred** | TailClipper |
|-----|------|-------------|-----------------|-------------|
| 800 | 800 | 800 | **800** | 800 |
| 1000 | 1000 | 1000 | **1000** | 1000 |
| 1200 | 1191 | 1172 | **1192** | 1184 |
| 1400 | 1346 | 1156 | **1332** | 1246 |
| 1600 | 1056 | 1066 | **1406** | 1040 |
| 1800 | 994 | 1003 | **1452** | 999 |
| 2000 | 1085 | 1048 | **1507** | 1087 |
| 2500 | 988 | 1063 | **1554** | 996 |
| 3000 | 124 | 1012 | **1520** | 942 |

This is the reference point. The refactored code must match or exceed these numbers on SocialNet.

---

## Iteration 0: Post-refactor baseline (ember_1)

**Status:** Running

### Change
No code changes — this tests the 2-layer refactor (commit 4fefbd98) as-is.

### Hypothesis
SocialNet is CPU-bound, so `est_compute_latency[parent→child key]` should be similar to the old `est_child` cost. The refactored 2-layer admission control should perform comparably to surge_8. Any significant regression would indicate that the compute cost estimate diverges from wall-clock more than expected, or that removing Layer 2 (compute feasibility) leaves a gap.

### Expected outcomes if hypothesis is correct:
1. pred+ac_pred goodput within ±50 of surge_8 at all RPS levels
2. No collapse at any load point
3. ac_pred still actively rejecting at 1600+ RPS (non-zero early returns with service names)

### Experiment design
Same config as surge_8 (800-3000 RPS, 30s per step, 50ms SLO, 4 policies).

### Actual Outcomes (ember_1)

**Status:** Regression ❌

| RPS | surge_8 pred+ac | ember_1 pred+ac | Delta | ember_1 FIFO | ember_1 TC | ember_1 pred (no ac) |
|-----|----------------|----------------|-------|-------------|-----------|---------------------|
| 800 | 800 | 800 | 0 | 800 | 800 | 800 |
| 1000 | 1000 | 1000 | 0 | 1000 | 1000 | 1000 |
| 1200 | 1192 | 1177 | -15 | 1193 | 1185 | 1188 |
| 1400 | 1332 | 1251 | **-81** | 1176 | 1300 | 1287 |
| 1600 | 1406 | 1060 | **-346** | 1054 | 1074 | 1063 |
| 1800 | 1452 | 1013 | **-439** | 1016 | 1034 | 1371 |
| 2000 | 1507 | 1741 | +234 | 1037 | 1148 | 1345 |
| 2500 | 1554 | 1065 | **-489** | 931 | 1040 | 1147 |
| 3000 | 1520 | 1018 | **-502** | 212 | 1080 | 1030 |

**Severe regression at 1400-3000 RPS.** At 1600-1800, pred+ac_pred performs no better than FIFO (~1060 vs ~1054). The 2000 RPS result (+234) is likely bistable noise (known SocialNet phenomenon).

**ac_pred is actively rejecting** (ER rates: 1400=147/s, 1600=539/s, 1800=783/s) but rejections are not translating into goodput benefit — the token bucket cost is too low.

### Root cause analysis

**The token bucket cost is ~10-20x too low.** The refactor changed the cost from `est_child_latency[key]` (wall-clock child RPC time, ~45K µs for compose-post) to `est_compute_latency[key]` (child's self-reported local compute time).

For SocialNet's parallel fanout architecture:
- **Old cost (est_child):** Wall-clock time of the compose-post call ≈ 45,000 µs. This includes all downstream work (compose-post + text-service + user-mention + url-shorten + ...).
- **New cost (est_compute):** Only compose-post's own local poll/compute time ≈ a few thousand µs. Does NOT include any child service compute.

The `compute_time_us` in `ResponseMeta` (set in `inject_response_meta`, state.rs:116) is `self.poll_compute_us` — just the local service's CPU time. For a fanout service that delegates most work to children, this is a tiny fraction of the true resource cost.

**Why the SURGE.md analysis was wrong:** The "Revised algorithm" section assumed `est_compute_latency[parent→child key]` would be "the child's REPORTED compute time" and said "For CPU-bound services, child compute ≈ child wall-clock." But this conflates two things:
1. The child service IS CPU-bound (its local compute ≈ its local wall-clock)
2. BUT the child's local compute ≠ the total downstream compute chain

For a leaf service (no children), compute ≈ wall-clock holds. For a fanout service (compose-post), local compute << wall-clock because most wall-clock time is waiting for parallel child RPCs.

### Fix direction

The correct approach: use `est_child_latency[key]` as the token bucket cost (reverting to the old cost signal), but keep the 2-layer structure (removing the redundant Layer 2). This restores the correct cost magnitude for SocialNet while keeping the architectural cleanup.

For Hotel's I/O-heavy case (where est_child includes MongoDB wait), a separate fix is needed — perhaps using `max(est_compute, est_child * cpu_fraction)` or using the ER-rate backstop signal described in SURGE.md.

---

## Iteration 1: Restore est_child_latency as token bucket cost (ember_2)

**Status:** Pending

### Change
In `admission_check()` Layer 2, change the token bucket cost from `est_compute_latency.get_estimate(key)` back to `est_child_latency.get_estimate(key)` (with fallback to est_compute if unavailable).

### Hypothesis
The regression is caused by the token bucket cost being ~10-20x too low. Restoring `est_child_latency` (wall-clock child time) should restore surge_8-level goodput for SocialNet. The 2-layer structure (removing redundant Layer 2 compute feasibility) is kept.

### Expected outcomes if hypothesis is correct:
1. pred+ac_pred goodput recovers to within ±50 of surge_8 at all RPS levels
2. ac_pred rejection rates similar to surge_8
3. No regression at underload (800-1200 RPS)

### Experiment design
Same config as surge_8/ember_1 (800-3000 RPS, 30s per step, 50ms SLO, 4 policies).

### Actual Outcomes (ember_2)

**Status:** Complete ✅ — Keep

**Code commit:** e4eefa4c

| RPS | surge_8 | ember_1 (broken) | ember_2 (fix) | vs surge_8 |
|-----|---------|-----------------|--------------|------------|
| 800 | 800 | 800 | 800 | 0 |
| 1000 | 1000 | 1000 | 1000 | 0 |
| 1200 | 1192 | 1177 | 1170 | -22 |
| 1400 | 1332 | 1251 | 1319 | -13 |
| 1600 | 1406 | 1060 | **1428** | +22 |
| 1800 | 1452 | 1013 | **1439** | -13 |
| 2000 | 1507 | 1741 | **1550** | +43 |
| 2500 | 1554 | 1065 | **1541** | -13 |
| 3000 | 1520 | 1018 | **1508** | -12 |

**Fix fully recovers surge_8 performance.** All deltas within ±43 RPS (run-to-run noise ~±20-50).

**ac_pred ER rates scale progressively:** 30/s at 1200, 81/s at 1400, 172/s at 1600, 360/s at 1800, 449/s at 2000, 958/s at 2500, 1492/s at 3000. Smooth, cost-aware admission control.

**All expected outcomes confirmed:**
1. ✅ Goodput recovered to within ±43 of surge_8 at all RPS
2. ✅ ac_pred rejection rates healthy and progressive
3. ✅ No regression at underload

### Root cause summary

The 2-layer refactor (commit 4fefbd98) changed the token bucket cost from `est_child_latency` (wall-clock child RPC time, ~45K µs for compose-post) to `est_compute_latency` (child's local compute time, ~few K µs). For fanout services, local compute << wall-clock because most wall-clock time is spent waiting for parallel child RPCs. The ~10-20x cost underestimate made the token bucket nearly a no-op.

**Fix:** Restore `est_child_latency` as the token bucket cost (commit e4eefa4c). The 2-layer structure is correct and kept.

**Open question for Hotel:** `est_child_latency` includes I/O wait (MongoDB ~115K µs), which caused the original Hotel crash with MAX_BURST_SECS=0.005 (budget can never cover 115K cost). This needs a separate solution — possibly `min(est_child, MAX_BURST_BUDGET * 0.5)` clamping, or the ER-rate backstop signal.

---

## Admission Control Design Analysis

Post-experiment design discussion exploring what the token bucket should represent and how to handle multi-API workloads.

### Optimization goal

Maximize goodput under overload. When shedding is necessary, preferentially reject expensive requests and admit cheap ones — serving 10 cheap requests yields more goodput than 1 expensive one.

### What signal tells you to throttle?

**Downstream utilization** (`max_downstream_util` propagated via ResponseMeta). This is an observation, not an estimate — it directly measures how stressed the bottleneck is.

**Gap:** For I/O-bound bottlenecks (Hotel/MongoDB), CPU utilization stays low even at overload. An ER-rate backstop signal would cover this but isn't implemented yet.

### What should the per-request cost be?

The token bucket is fundamentally a rate limiter with adaptive rate. The utilization signal does the heavy lifting (tells you *when* to throttle). The per-request cost determines *what* to shed preferentially.

| Cost metric | Pros | Cons |
|-------------|------|------|
| `est_child` (wall-clock child time) | Right magnitude, enables cross-API prioritization, captures both CPU and I/O occupancy | Includes queue wait (noisy under load) |
| `est_compute` (child's local CPU) | Clean resource signal | Only one hop deep — misses fanout children. ~10-20x too small for fanout services (ember_1 proved this) |
| Cumulative downstream compute | Theoretically principled for CPU | Resources aren't fungible across microservices. 3 services × 1 CPU ≠ 3 CPUs of shared capacity. The bottleneck is `max(per_service_util)`, not `sum(compute)`. |
| cost=1 (count requests) | Simplest, no estimation needed | Can't differentiate expensive vs cheap APIs — critical requirement for goodput maximization |

**Recommendation: `est_child` (wall-clock).** It's the right proxy for "how long does this request occupy downstream capacity." Enables cross-API prioritization (cheap requests outcompete expensive ones). The fact that it includes I/O wait is correct — an I/O-heavy request really does occupy the path for that duration.

### Why compute-only tokens don't work

The initial motivation for switching to `est_compute_latency` was to correctly handle I/O-heavy workloads (Hotel) where wall-clock includes MongoDB wait that doesn't consume CPU. But this reasoning has two flaws:

1. **`compute_time_us` in ResponseMeta is only the immediate child's local CPU time** — it doesn't include grandchildren. For fanout services (compose-post), local compute is a tiny fraction of total downstream work. To make compute-only tokens work, you'd need cumulative `total_downstream_compute` propagated through ResponseMeta.

2. **Even with cumulative compute, resources aren't fungible across services.** Each microservice has its own independent CPU. A call graph traversing 3 services has 3 independent CPU pools, not a shared pool of 3 CPUs. One service at 95% utilization bottlenecks the system regardless of whether the other two are at 10%. Summing compute across services doesn't reflect the actual constraint.

### Per-API or shared bucket?

| Approach | Good for | Bad for |
|----------|----------|---------|
| Per-API buckets | Disjoint paths (no false coupling) | Can't prioritize cheap over expensive across APIs |
| Shared bucket | Cross-API prioritization, overlapping paths | False coupling on disjoint paths |

**Recommendation: shared bucket.** The goal of preferentially admitting cheap requests requires a shared budget where cheap and expensive requests compete. Disjoint-path false coupling is a real concern but second-order.

**Overlapping subgraph nuance:** Per-API buckets work well when each API traverses completely different parts of the service graph. But when APIs share bottleneck services, independent per-API rate limiters can oscillate against each other (both fighting over the same shared resource). A shared bucket naturally coordinates competing demand on shared bottlenecks.

**Deferred:** dynamically discovering whether APIs share bottlenecks to choose per-API vs shared automatically.

### Rate signal with multiple APIs

Current code uses the calling API's util to adjust the shared rate. With two APIs at different utilization levels (e.g., Search util=0.9, Reservation util=0.3), the rate ping-pongs on every request — never converging. Using `max(util across all APIs)` avoids ping-pong but over-throttles APIs on unsaturated paths. The correct solution depends on bottleneck sharing detection (deferred).

### Burst cap (MAX_BURST_SECS)

**Why it exists:** Prevents accumulation-driven oscillation. Without it, budget surplus accumulates during "good" seconds, enabling a burst that overloads downstream and triggers a "bad" second. SURGE iterations 3-6 proved that rate-tuning approaches (AIMD, rate freeze, additive increase) don't fix this — the root cause is surplus accumulation, not rate dynamics.

**Problem:** A fixed `MAX_BURST_SECS=0.005` doesn't generalize. For SocialNet (est_child≈45K µs), max budget=75K µs admits ~1.7 requests per burst — tight enough. For Hotel (est_child≈115K µs), max budget=75K < cost, making requests permanently inadmissible (the crash).

**Recommendation: dynamic floor.** `max_budget = max(rate × MAX_BURST_SECS, largest_recent_cost × 2)`. Preserves anti-oscillation while ensuring no request is permanently inadmissible. The ×2 margin allows one expensive + one cheap request in the same burst window.

### Recommended algorithm

```
Layer 1 (every hop): Deadline feasibility
  - est_remaining_floor > time_left → shed
  - Triage: shed doomed requests before they waste resources
  - Uses downstream work estimation (genuinely valuable here)

Layer 2 (ingress only): Token bucket admission
  - Cost:  est_child_latency (wall-clock child time)
  - Rate:  adjusts based on downstream utilization signal
  - Burst: max(rate × MAX_BURST_SECS, largest_recent_cost × 2)
  - Shared across APIs for cross-API prioritization
```

**Where estimation helps and where it doesn't:**
- Layer 1 (deadline feasibility): estimation is critical — knowing est_remaining lets you shed doomed requests that are technically alive but can't finish in time
- Layer 2 (token bucket): estimation provides the cost metric for cross-API prioritization, but the utilization signal does the real work of controlling the admission rate
- Scheduling (sched_pred, outside AC): estimation drives deadline tightening and EDF reprioritization — major goodput driver

### Concrete code changes needed

1. **Dynamic burst floor** — fix Hotel crash without regressing SocialNet
2. **Rate signal consolidation** — address ping-pong with multiple APIs (conservative: use max util; ideal: bottleneck-aware grouping, deferred)
