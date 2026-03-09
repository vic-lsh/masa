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

### Actual Outcomes (s1467_4)

**✓ Fixes worked — no longer catastrophically broken:**
- Zero early-returns at 200 and 400 RPS (hypothesis 1 confirmed)
- CPU on ms-73106: 69.96ms mean (vs 22.65ms broken, vs 82.11ms `prio_oldest,early`)
- CPU on ms-11639: 16.50ms mean (vs 0.126ms broken, vs 24.05ms `prio_oldest,early`)

**Early return comparison (root-level only):**
| RPS | prio_local,est_mean_var,early | prio_oldest,early | prio_local,early |
|-----|-------------------------------|-------------------|-----------------|
| 200 | 0 | 0 | 0 |
| 400 | 0 | 0 | 0 |
| 800 | **60.3/s** | **6.8/s** | 132.3/s |
| 1000 | **54.5/s** | **211.8/s** | 172.7/s |
| 1200 | **59.6/s** | **445.1/s** | 203.3/s |
| 1400 | **78.4/s** | **627.5/s** | 252.7/s |
| 1800 | **106.6/s** | **989.8/s** | 387.4/s |

At ≥1000 RPS, `est_mean_var` sheds far fewer requests at root (54 vs 212 at 1000) = **better
goodput potential**. At 800 RPS, `est_mean_var` sheds 60/s unnecessarily while `prio_oldest`
has only 6.8/s — system is not overloaded, so shedding is wasteful.

**Remaining problem — k=1.0 too conservative:**
Large inner-service early-returns at ms-37691/y_DKOh-Gts:
- 800 RPS: 17.6/s inner + 37/s at ykccIz2fkK = 54/s from ms-37691 alone
- 1800 RPS: 470/s (y_DKOh-Gts) + 113/s (ykccIz2fkK) = 583/s inner sheds at ms-37691

These inner sheds are triggered because ms-37691 computes `est_remaining = mean + 1.0*stddev`,
which at σ≈high-variance levels overestimates the remaining budget needed. With k=1.0 the estimate
is the 84th percentile — too conservative when there's capacity to serve most requests.

**Conclusion:** The policy is now basically working. The k=1.0 default is the next thing to tune.

---

## Iteration 2: Reduce k from 1.0 to 0.5 (experiment s1467_5)

**Status:** Implemented, experiment pending

### Change

In `libs/masa-core/src/latency_estimator/mean_var.rs`, change the default k from 1.0 to 0.5:

```rust
impl Default for LatencyMeanVar {
    fn default() -> Self {
        Self::new(0.5, 512)  // was 1.0
    }
}
```

### Rationale

With k=1.0, the estimate is `mean + 1.0*σ` ≈ 84th percentile of observed latency. The
`est_remaining` budget allocated per edge is therefore quite large, causing the adjusted deadline
`d = parent_deadline - est_remaining` to be tight. Requests that arrive anywhere past that tight
deadline get shed — even at 800 RPS where the system has capacity.

With k=0.5, the estimate is `mean + 0.5*σ` ≈ 69th percentile. This:
- Reduces the deadline budget per edge by ~0.5*σ per hop
- Allows more requests to pass the early-return check at moderate load
- Still provides enough margin to shed requests that are clearly over budget

### Hypothesis

**Expected outcomes if hypothesis is correct:**
1. At 800 RPS: root early-returns should drop significantly (from 60/s toward 0–15/s), matching
   or approaching `prio_oldest,early`'s 6.8/s.
2. Inner-service early returns at ms-37691/y_DKOh-Gts should drop substantially at all RPS levels.
3. At high RPS (1400–1800): root early-returns should remain low (< `prio_oldest,early`'s 990/s),
   preserving the goodput advantage at high loads seen in s1467_4.
4. CPU on ms-73106 should remain in the 70–80ms range (vs `prio_oldest,early`'s 82ms), indicating
   actual work is being done rather than bulk shedding.
5. Overall goodput (fraction of requests meeting SLO) should beat `prio_oldest,early` at ≥800 RPS.

**Risk:** Less conservative estimates may let some doomed requests pass, wasting capacity. If
ms-73106's variance is very high (as the PRIO_LOCAL_IMPROVEMENTS.md analysis suggests: σ≈20,000µs),
then k=0.5 estimates ≈ 75,000+10,000=85,000µs which is still a reasonable budget within 100ms SLO.

### Actual Outcomes (s1467_5)

**k=0.5 is WORSE than k=1.0 across the board:**

Absolute goodput (RPS within SLO):
| RPS | k=1.0 (s1467_4) | k=0.5 (s1467_5) | prio_oldest |
|-----|-----------------|-----------------|-------------|
| 800 | **670** | 626 | 785 |
| 1000 | **714** | 649 | 783 |
| 1200 | **758** | 670 | 761 |
| 1400 | **785** | 714 | 757 |
| 1800 | **849** | 749 | 787 |

k=0.5 reduces inner-service early-returns (expected) but INCREASES root-level early-returns.
This is because with less aggressive inner shedding, more requests flow through deep chains,
take long, and finally miss the SLO at the root level. Net effect is worse.

**s1467_4 (k=1.0) already beats prio_oldest at 1400+ RPS!**
- r1400: 785 vs 757 (+28 abs RPS, +3.7%)
- r1800: 849 vs 787 (+62 abs RPS, +7.9%)

**Remaining gap at lower loads:**
- r800: 670 vs 785 (-115, -14%)
- r1000: 714 vs 783 (-69, -8.8%)

**Root cause of 800 RPS gap — compounding deadline tightening:**

Each hop subtracts its `est_remaining` from the parent deadline before passing it to the child:
```
ms-37691_deadline = root_deadline - est_remaining_root ≈ T + 100ms - 5ms = T + 95ms
grandchild_deadline = ms37691_deadline - est_remaining_ms37691 ≈ T + 95ms - 80ms = T + 15ms
```
With est_remaining values correctly capturing subsequent-child work (e.g., ms-37691 calls
ms-73106 next so est_remaining includes ~75ms for ms-73106), the grandchild sees a 15ms deadline
even on an underloaded system. This cascades, causing excessive early-returns deep in the graph.

**Fix for iteration 3:** Do NOT propagate the tightened deadline to children. The child still
receives the original parent deadline; the adjusted_deadline is only used locally for:
1. The single-hop early-return check at the current service
2. The priority hint computation (encodes path-aware slack)

---

## Iteration 3: Don't propagate tightened deadline to children (experiment s1467_6)

**Status:** Implemented, experiment pending

### Change

In `local.rs`, pass `self.ctx.deadline()` (original parent deadline) to the child instead of the
tightened `adjusted_deadline`:

```rust
// Before: child got tightened deadline (compounding problem)
let child_recv_ctx = ContextBuilder::from(&self.ctx)
    .deadline(deadline)  // tightened: parent_deadline - est_remaining
    ...

// After: child gets original parent deadline (no compounding)
let adjusted_deadline = self.ctx.deadline().saturating_sub(est_remaining);
// ... use adjusted_deadline for early-return check and prio_hint ...
let child_recv_ctx = ContextBuilder::from(&self.ctx)
    .deadline(self.ctx.deadline())  // original parent deadline
    ...
```

Also reverted k from 0.5 back to 1.0 (s1467_5 proved k=1.0 is strictly better).

### Hypothesis

By stopping the compounding, each service's early-return check is independent:
- Root sheds when `time_now() > root_deadline - est_remaining_root` (single-hop check)
- ms-37691 sheds when `time_now() > ms37691_deadline - est_remaining_ms37691`, but now
  ms37691_deadline = root_deadline (not root_deadline - est_remaining_root)

Since ms37691_deadline = root_deadline (no prior tightening), the total budget consumed across
all hops' early-return checks is NOT compounded. Each service sees the full parent SLO minus its
own est_remaining, which should be much less aggressive.

**Expected outcomes:**
1. Inner-service early-returns (ms-37691/y_DKOh-Gts) should drop dramatically at all RPS levels
2. Root early-returns at 800 RPS should drop significantly (toward prio_oldest's 6.8/s)
3. Absolute goodput at 800 RPS should approach prio_oldest's 785 RPS
4. High-load goodput (1400-1800 RPS) should remain higher than prio_oldest's 757-787 RPS
   because priority hints still encode path-aware slack correctly
5. Total goodput sum across all loads should beat prio_oldest

**Risk:** Forwarding the full parent deadline means child servers see less tight deadlines, so
the `before_poll` / `after_poll` early-return at child servers will be less aggressive. This
could allow more requests to attempt expensive downstream work and then fail the SLO. However,
the PRIORITY ORDERING should still be correct (prio_hint is unchanged), so the right requests
get served first and the waste should be minimal.

### Actual Outcomes (s1467_6)

**CATASTROPHIC FAILURE.** prio_local,early also uses local.rs; passing the original deadline
broke BOTH policies:

| RPS | s4 est_mean_var k=1.0 | s6 no-compound | s4 prio_oldest |
|-----|-----------------------|----------------|----------------|
| 800 | 670 | 553 | 785 |
| 1000 | 714 | **317** | 783 |
| 1200 | 758 | 327 | 761 |
| 1800 | 849 | 458 | 787 |

Root cause: without tightened deadlines, inner services no longer shed at all. At 1000+ RPS
(overloaded), every service attempts every request until the full 100ms elapses, causing
massive queuing and SLO violations across the board. The deadline propagation is not optional —
it is the mechanism that enables inner-service load shedding.

**Key learnings:**
1. The 800 RPS gap (670 vs 785) is STEADY-STATE, not warmup (verified by 5s time-bucket analysis)
2. The issue is that `est_after_child_latency` correctly estimates ~75ms for paths that include
   ms-73106, causing the early-return check to fire at T+5ms even at low load
3. Deadline propagation is fundamental; cannot be removed or modified without breaking the system

---

## Iteration 4: Mean-only threshold for early-return (experiment s1467_7)

**Status:** Implemented, experiment pending

### Change

Add `mean_estimate()` to the `LatencyEstimator` trait (default = `estimate()`). Override in
`LatencyMeanVar` to return just the Welford mean (k=0 effective). Add `get_mean_estimate()` to
`LatencyMap`. Use the mean estimate for the early-return threshold in `local.rs`, while keeping
`get_estimate()` (mean+k*σ) for the tightened deadline forwarded to children and for `prio_hint`.

Key distinction:
- **Early-return check**: `time_now() > parent_deadline - mean_remaining` (mean, no σ term)
- **Deadline for child**: `parent_deadline - estimate_remaining` (mean+k*σ, unchanged)
- **Priority hint**: `deadline - est_child` (unchanged)

### Hypothesis

The σ term in `est_remaining` is responsible for the conservative early-return at underloaded
conditions. With mean=75ms and σ=20ms, k=1.0 estimate=95ms fires the check at T+5ms. Using
mean=75ms fires at T+25ms — allowing 20ms more processing time before shedding.

Priority ordering and deadline propagation are fully preserved (both still use mean+k*σ).

**Expected outcomes:**
1. 800 RPS: early-return threshold shifts from T+5ms to T+25ms — fewer root early-returns
   (~20–40/s instead of 60/s), improving goodput toward prio_oldest's 785
2. 1800 RPS: threshold also shifts (mean=75ms not 95ms), so slightly less proactive shedding.
   High-load goodput may drop slightly from 849 but should remain above prio_oldest's 787
3. Net: improved 800–1000 RPS performance with modest high-load reduction → better total goodput

**Risk:** Using mean for early-return allows more σ-range requests through. At overloaded
conditions (1800 RPS), these extra requests consume capacity that could serve requests within SLO,
potentially reducing high-load goodput below prio_oldest's 787. However, priority ordering is
intact so capacity is still allocated to best candidates.

### Actual Outcomes (s1467_7)

**Marginal improvement overall; does not close the 800 RPS gap:**

| RPS  | s1467_4 (k=1.0) | s1467_7 (mean ER) | prio_oldest (s7) |
|------|-----------------|-------------------|------------------|
| 800  | 671.6           | 672.7             | 793.0            |
| 1000 | 715.2           | **741.6**         | 820.9            |
| 1200 | 759.7           | 760.2             | 789.2            |
| 1400 | 787.0           | 780.1             | 792.7            |
| 1800 | 850.6           | 845.9             | **809.7**        |

s1467_7 improves 1000 RPS by +26 RPS but loses slightly at 1400-1800 RPS. **800 RPS gap is
unchanged** (120 RPS below prio_oldest). The mean-only early-return threshold had essentially
zero effect at 800 RPS because:

1. Root early-returns (64.65/s) come from `EarlyReturnHandler::check` (time_now ≥ T+100ms),
   not from the est_after threshold check. These are requests where total latency exceeds SLO.
2. MS_37691 y_DKOh-Gts early-returns (17.73/s) come from EarlyReturnHandler at MS_37691 with
   ctx.deadline ALREADY PAST: deadline=T+77ms is set by MS_56394's deadline propagation (using
   mean+σ=23ms for est_after), but MS_56394 calls y_DKOh-Gts at T+82ms (after ~80ms own CPU).
   Changing early-return threshold (mean vs mean+σ) doesn't help — the DEADLINE is what's wrong.
3. MS_37691 ykccIz2fkK early-returns (37.68/s) come from tight timing at T+93ms call with
   T+98ms deadline — only 5ms slack with busy MS_37691 serving 1600 calls/s.

**Key finding from trace data:** MS_56394 takes 93-95ms total (p50=95ms from trace). MS_37691
calls take only 2ms each. MS_56394 has ~80ms of intrinsic CPU before calling y_DKOh-Gts. So:
- est_after[MS_56394→y_DKOh-Gts] = ~13ms (mean), σ≈10ms
- With mean+σ=23ms: deadline = T+77ms
- MS_56394 calls y_DKOh-Gts at T+82ms → immediate early-return at MS_37691 ✗
- With mean=13ms: deadline = T+87ms > T+82ms → no immediate early-return ✓

---

## Iteration 5: Use mean-only for est_remaining in deadline propagation (experiment s1467_8)

**Status:** Planned

### Change

In `local.rs`, change the deadline propagation to use `get_mean_estimate()` (mean, k=0) instead
of `get_estimate()` (mean+k*σ) for `est_remaining`:

```rust
// BEFORE (s1467_7): deadline uses mean+σ, early-return uses mean
let est_remaining = self.server.est_after_child_latency.get_estimate(key).unwrap_or(0).min(time_left);
let est_remaining_threshold = self.server.est_after_child_latency.get_mean_estimate(key).unwrap_or(0).min(time_left);

// AFTER (Iteration 5): both use mean only
let est_remaining = self.server.est_after_child_latency.get_mean_estimate(key).unwrap_or(0).min(time_left);
// est_remaining_threshold is now identical to est_remaining (simplify to one variable)
```

### Hypothesis

The y_DKOh-Gts deadline is set to T+77ms (mean+σ=23ms), but MS_56394 calls y_DKOh-Gts at
T+82ms — already 5ms past the deadline. This causes immediate early-return at MS_37691 for every
request. Using mean=13ms instead gives deadline=T+87ms, which is after T+82ms, eliminating the
false positive.

For doomed requests at high load (where time_now >> T+82ms anyway), the early-return at
MS_37691 still fires correctly because time_now ≥ ctx.deadline = T+87ms.

**Expected outcomes:**
1. y_DKOh-Gts sheds at MS_37691 should drop from 17.73/s to ~0/s at 800 RPS
2. ~18 RPS goodput improvement at 800 RPS (672 → ~690)
3. ykccIz2fkK and root sheds unchanged (different causes)
4. High-load performance unchanged or slightly better (doomed requests still shed)
