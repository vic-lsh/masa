# prio_local,est_mean_var,early — Analysis and Improvement Recommendations

Based on experiment results in `exp/mssim/plots/s1467_3`, `prio_local,est_mean_var,early` performs
significantly worse than `prio_oldest,early`. This document identifies the root causes and concrete
fixes.

---

## Observed Symptoms (s1467_3)

From `exp/mssim/plots/s1467_3/0/early_return_breakdown.csv`:

- `prio_local,est_mean_var,early` early-returns **130 requests/s at 200 RPS** from ms-73106 — nearly
  the entire offered load, at the lowest RPS tested. Every other policy shows zero or minimal early
  returns below 800–1000 RPS.

From `exp/mssim/plots/s1467_3/cpu_summary.csv`:

| Service   | prio_local,early (mean CPU) | prio_local,est_mean_var,early | prio_oldest,early |
|-----------|-----------------------------|-------------------------------|-------------------|
| ms-73106  | 76.6 ms                     | **22.6 ms**                   | 83.9 ms           |
| ms-11639  | 16.9 ms                     | **0.13 ms**                   | 25.2 ms           |

The low CPU is not efficient load shedding — almost all requests are rejected immediately, so no
real work is done.

---

## Root Cause 1: Dimensional bug in `LatencyMeanVar` (critical)

**File:** `libs/masa-core/src/latency_estimator/mean_var.rs:42`

```rust
// Current — WRONG
let variance = self.m2 / self.count as f64;
let raw_estimate = self.mean + self.k * variance;
```

All tracked latency values are in **microseconds**. Therefore:
- `mean` has units µs
- `variance = M2 / count` has units **µs²** (squared)

Adding them is dimensionally incoherent. For ms-73106 with real latencies (mean ≈ 75,000 µs,
stddev ≈ 20,000 µs):

```
variance    = 20,000² = 400,000,000 µs²
estimate    = 75,000 + 1.0 × 400,000,000 ≈ 400,000,000 µs  ≈ 400 seconds
```

This inflated `est_remaining` is then used in `local.rs:280`:

```rust
let deadline = self.ctx.deadline() - est_remaining;
if EARLY_RETURN && time_now() > deadline {
    return Err(self.early_return.issue_error());
}
```

`time_now()` is microseconds since UNIX epoch (~1.741 × 10¹⁵ µs). With `est_remaining ≈ 400 billion
µs`, `deadline` becomes a timestamp 400,000 seconds in the past. `time_now() > deadline` is
**always true** — every request early-returns, at every load level.

**Fix:** Use standard deviation (sqrt of variance), not variance itself. The intended formula is the
standard sigma-bound estimator `mean + k·σ`.

```rust
// libs/masa-core/src/latency_estimator/mean_var.rs
fn update_estimate(&mut self) {
    if self.count == 0 {
        self.estimate = 0;
        return;
    }
    let variance = self.m2 / self.count as f64;
    let stddev = variance.sqrt();                    // FIX: take sqrt
    let raw_estimate = self.mean + self.k * stddev;  // FIX: use stddev, not variance
    self.estimate = if raw_estimate.is_finite() && raw_estimate > 0.0 {
        raw_estimate.min(u64::MAX as f64) as u64
    } else {
        0
    };
}
```

With the fix: `estimate = 75,000 + 1.0 × 20,000 = 95,000 µs = 95ms` — a reasonable estimate for a
100ms SLO.

---

## Root Cause 2: No underflow protection in deadline computation

**File:** `libs/tonic/tonic/src/masa/context/local/local.rs:280`

```rust
let deadline = self.ctx.deadline() - est_remaining;  // u64 subtraction, can underflow
```

If `est_remaining` ever exceeds `deadline` (even after fixing Root Cause 1, this can happen
transiently during warmup or with outlier observations), the subtraction silently wraps in release
mode to a value near `u64::MAX`. The subsequent `time_now() > deadline` check then becomes false
(no early return), but the child receives an absurd deadline timestamp, breaking downstream
scheduling.

**Fix:** Use saturating arithmetic and cap `est_remaining` to the remaining SLO budget:

```rust
// libs/tonic/tonic/src/masa/context/local/local.rs
let time_left = self.ctx.deadline().saturating_sub(masa_core::time_now());
let est_remaining = self
    .server
    .est_after_child_latency
    .get_estimate(key)
    .unwrap_or(0)
    .min(time_left);                                     // never exceed remaining budget
let deadline = self.ctx.deadline().saturating_sub(est_remaining);
```

---

## Recommendation 3: Sliding window or EMA decay in estimators

**Files:** `libs/masa-core/src/latency_estimator/mean_var.rs`, `rms.rs`

Both estimators accumulate all-time statistics. If the system was heavily loaded at some point,
the estimate remains elevated long after load drops (and vice versa). Under varying load, the
distribution of latencies shifts significantly; a stale estimate from 30 seconds ago can be
actively harmful.

Options (in increasing implementation complexity):

1. **Reset on interval**: periodically discard history and restart, accepting a cold-start
   period but ensuring freshness.
2. **Sliding window**: keep only the last N observations; evict the oldest on each track call.
3. **Exponential moving average (EMA)**: weight recent observations more heavily.

A simple EMA for `mean_var`:
```rust
// weight recent observations α-strongly; α ≈ 0.01–0.05 works well
self.mean = α * value + (1.0 - α) * self.mean;
```

This removes the need for `update_interval` entirely (every track updates the estimate) and
naturally decays stale data.

---

## Recommendation 4: Tune `k` (default 1.0 may be too conservative)

**File:** `libs/masa-core/src/latency_estimator/mean_var.rs:93`

With the formula fixed (`mean + k·σ`), `k=1.0` places the estimate at the **84th percentile** of
observed latency. For deadline scheduling, consider:

- `k=0`: pure mean — optimistic, may under-allocate budget to slow paths
- `k=0.5`: mean + 0.5·σ — ~69th percentile, less aggressive early shedding
- `k=1.0`: mean + 1·σ — ~84th percentile (current default, reasonable starting point)
- `k=2.0`: mean + 2·σ — ~97th percentile, very conservative

For the mssim workload, the high variance of ms-73106 means even `k=0.5` may provide enough
margin. The right value should be validated empirically against goodput after fixing Root Cause 1.

---

## Recommendation 5: Smoother estimator warmup

**Files:** `libs/masa-core/src/latency_estimator/mean_var.rs`, `rms.rs`

Both estimators use `update_interval=512`, meaning the cached estimate stays at 0 until 512
observations have been collected. After the 512th observation, the estimate jumps from 0 to the
full computed value. This step function is especially harmful when the first batch of observations
contains outliers (e.g., cold JIT, cache miss, or load spike during startup).

**Fix:** Use a doubling schedule: update at count = 1, 2, 4, 8, 16, ..., stabilizing to the
normal `update_interval` once sufficient data exists. This gives a useful-but-uncertain estimate
immediately, refining it as more data arrives:

```rust
fn track(&mut self, value: u64) {
    // ... Welford update ...
    self.since_last_update += 1;
    let threshold = if self.count < self.update_interval {
        self.count.next_power_of_two().min(self.update_interval)
    } else {
        self.update_interval
    };
    if self.since_last_update >= threshold {
        self.update_estimate();
        self.since_last_update = 0;
    }
}
```

---

## Why `prio_local` can outperform `prio_oldest` with correct estimates

`prio_oldest` (`prio_oldest.rs:76-82`) propagates priority unchanged through the call graph:

```rust
let deadline = self.ctx.deadline();
let prio_hint = self.ctx.prio_hint();
```

It is blind to call-graph structure. Two requests that arrived at the same time get the same
priority regardless of how much downstream work remains. This leads to:

- **Wasted work on doomed requests**: a request 90ms into a 100ms SLO that still needs 60ms of
  downstream processing will miss its SLO regardless; serving it over a request needing only 5ms
  strictly harms goodput.
- **No path awareness**: calling ms-73106 (mean 75ms) vs ms-6190 (mean 0.09ms) from the same
  parent leaves vastly different amounts of slack, but `prio_oldest` treats these identically.
- **Late shedding**: early-return only triggers at the absolute deadline, after the expensive
  child call has already been dispatched.

`prio_local` (`local.rs:274-302`) computes slack per call-graph edge:

```rust
// priority = parent_deadline − est_remaining_after_child − est_child_duration
let deadline = self.ctx.deadline() - est_remaining;
let prio_hint = deadline - est_child;
```

With correct estimates this enables:

1. **Predictive early shedding**: if `time_now() > deadline` (the adjusted deadline accounting
   for remaining work after this child), the request cannot complete even if the child responds
   instantly — shed it before wasting the child's capacity.
2. **Path-aware prioritization**: a request calling an expensive child inherits a tighter deadline,
   so the child server correctly prioritizes it relative to other in-flight requests.
3. **Correct cross-path comparison**: two same-age requests on different paths are prioritized by
   actual slack, not just arrival time.

The current poor performance is entirely due to the variance formula bug — the approach itself is
sound.

---

## Priority order of fixes

| # | Change | File | Impact |
|---|--------|------|--------|
| 1 | Fix `mean + k*variance` → `mean + k*stddev` | `libs/masa-core/src/latency_estimator/mean_var.rs:42` | **Critical** — unblocks the policy from working at all |
| 2 | Use `saturating_sub` + cap `est_remaining` at remaining budget | `libs/tonic/tonic/src/masa/context/local/local.rs:280` | **High** — prevents silent underflow corrupting deadlines |
| 3 | Add sliding window / EMA decay to estimators | `libs/masa-core/src/latency_estimator/` | Medium — needed for good performance under load variation |
| 4 | Smoother warmup (doubling interval schedule) | `libs/masa-core/src/latency_estimator/` | Medium — avoids step-function jump on first estimate |
| 5 | Tune `k` empirically after fix #1 | `libs/masa-core/src/latency_estimator/mean_var.rs:93` | Low — refinement once basic correctness is restored |

---

## Iteration 1: Fixes #2 and #4 (experiment s1467_4)

**Status:** Implemented, experiment pending

### Changes implemented

**Fix #2 — Saturating arithmetic in `local.rs`**

Before:
```rust
let est_remaining = self.server.est_after_child_latency.get_estimate(key).unwrap_or(0);
let deadline = self.ctx.deadline() - est_remaining;  // can underflow!
...
let prio_hint = deadline - est_child;  // can underflow!
```

After:
```rust
let time_left = self.ctx.deadline().saturating_sub(time_now());
let est_remaining = self.server.est_after_child_latency.get_estimate(key).unwrap_or(0).min(time_left);
let deadline = self.ctx.deadline().saturating_sub(est_remaining);
...
let prio_hint = deadline.saturating_sub(est_child);
```

**Fix #4 — Doubling warmup schedule in `mean_var.rs`**

Replaced the `since_last_update` counter with `next_update` (absolute count trigger). The estimator
now first updates after observation 1, then 2, 4, 8, ..., up to `update_interval` (512), then
switches to steady-state fixed-interval updates (`count + 512`). This eliminates the cold-start
step function: previously the estimate was stuck at 0 for the first 512 observations, then jumped
to the full computed value.

### Hypothesis

With Fix #1 already applied (correct stddev formula), Fix #2 is a safety net: even if an estimate
transiently overshoots the remaining SLO budget (e.g., cold start, outliers), the deadline will be
clamped to `time_now()` rather than wrapping to a timestamp 400 seconds in the past, which would
incorrectly trigger or suppress early returns.

Fix #4 is the more impactful change for this run: with a 30-second experiment window at 200 RPS
across ~6 services, each parent→child edge sees only a few hundred observations per RPS level.
The original 512-observation warmup means the estimator gives *zero* estimates for much of the
early-load experiment points, falling back to `deadline = parent_deadline - 0` (no adjustment) and
`prio_hint = deadline - 0`. This makes `prio_local,est_mean_var` behave identically to
`prio_local` (default RMS estimator) until warmup completes.

**Expected outcomes if hypotheses are correct:**
1. `prio_local,est_mean_var,early` should now show *no* early-returns at 200 RPS (vs 130/s before).
   Previously this was caused by Fix #1 being missing; the run verifies Fix #1 alone is sufficient.
2. At moderate loads (400–800 RPS), the policy should begin differentiating from `prio_local,early`
   because it gets useful estimates much earlier (after ~10–20 observations per edge instead of 512).
3. CPU utilization on ms-73106 and ms-11639 should be closer to `prio_oldest,early` at low RPS
   (since we're not shedding prematurely).
4. At high loads (≥1000 RPS), goodput should match or exceed `prio_oldest,early` because the
   path-aware deadline propagation allows smarter early shedding of doomed requests.
5. The fix to `prio_hint` underflow means downstream servers receive correct priority ordering
   even when `est_child` estimates are large, improving cross-server scheduling.
