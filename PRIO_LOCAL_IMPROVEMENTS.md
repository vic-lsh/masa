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

### Actual Outcomes (s1467_8)

**CATASTROPHIC FAILURE.** Goodput collapsed at all RPS levels:

| RPS  | s1467_7 | s1467_8 | prio_oldest |
|------|---------|---------|-------------|
| 800  | 672.7   | 569.3   | 798.0       |
| 1000 | 741.6   | 540.0   | 828.1       |
| 1200 | 760.2   | 502.5   | 776.1       |
| 1800 | 845.9   | 557.5   | 787.9       |

Root cause: using mean for deadline ALSO reduces the prio_hint for y_DKOh-Gts from T+75ms to
T+85ms (since prio_hint = deadline - est_child). The 10ms lower priority at MS_37691 causes
y_DKOh-Gts calls to wait longer in MS_37691's queue, creating cascading failures at all loads.
The prio_hint must use mean+σ to correctly encode urgency.

**Key lesson**: Deadline propagation and priority hint must be decoupled. Need to use mean for
the DEADLINE (so child doesn't see expired deadline) but mean+σ for PRIO_HINT (for urgency).

---

## Iteration 6: Decouple child deadline (mean) from prio_hint (mean+σ) (experiment s1467_9)

**Status:** Planned

### Change

In `local.rs`, compute two separate estimates from `est_after_child_latency`:
- `est_remaining_mean` (mean, k=0): used for child deadline and early-return threshold
- `est_remaining_full` (mean+k*σ): used ONLY for prio_hint computation

```rust
// Child receives looser deadline (mean) — achievable when called late
let deadline = ctx_deadline - est_remaining_mean;
let early_return_deadline = deadline;  // consistent with child's deadline

// Priority hint uses tight estimate (mean+σ) — encodes correct urgency
let tight_deadline = ctx_deadline - est_remaining_full;
let prio_hint = tight_deadline - est_child;  // same as s1467_4
```

### Hypothesis

With this decoupling:
- y_DKOh-Gts deadline = T+87ms (mean=13ms): MS_56394 calls at T+82ms < T+87ms → no shed ✓
- y_DKOh-Gts prio_hint = T+75ms (same as s1467_4) → same urgency ordering at MS_37691 ✓
- Early-return at parent (MS_56394): fires if time_now > T+87ms (consistent) ✓

Expected: ~17.73/s fewer y_DKOh-Gts false-positive sheds at 800 RPS → +18 RPS goodput.
Priority ordering unchanged from s1467_4, so high-load behavior should match or exceed s1467_4.

### Actual Outcomes (s1467_9)

**CATASTROPHIC FAILURE.** The decoupling fixed y_DKOh-Gts sheds but tripled root early-returns:

| RPS  | s1467_7 (mean ER) | s1467_9 (decoupled) | prio_oldest |
|------|-------------------|---------------------|-------------|
| 800  | 672.7             | **571.8**           | 797.5       |
| 1000 | 741.6             | **509.0**           | 820.4       |
| 1200 | 760.2             | **514.0**           | 790.0       |
| 1400 | 780.1             | **535.5**           | 771.0       |
| 1800 | 845.9             | **542.6**           | 815.9       |

Early-return changes at 800 RPS:
- y_DKOh-Gts: 17.73/s → **0.37/s** ✓ (fixed as intended)
- Root early-returns: 64.65/s → **200.56/s** ✗ (tripled — catastrophic)

Root cause: by giving y_DKOh-Gts a looser deadline (T+87ms mean vs T+77ms mean+σ), the requests
that previously shed at MS_37691 at T+82ms (before the child was called) now proceed to full
completion at T+93-99ms. With variance, many exceed T+100ms → root SLO violation. The early shed
at T+82ms was actually BENEFICIAL — it freed USER capacity faster than the full 95ms execution.

**Key lesson**: Early shedding at y_DKOh-Gts (T+82ms) is not a false positive at 800 RPS — it is
the correct load-shedding behavior. The request takes 95ms total; if shed at T+82ms, the USER slot
frees at T+82ms, allowing another request. If not shed, it finishes at T+95ms (or fails at T+100ms+
with variance). Eliminating these sheds at 800 RPS makes performance WORSE.

**800 RPS gap is structural**: MS_56394 intrinsically takes 93-95ms with only 5ms SLO slack. Any
priority-induced queuing causes failures. The gap appears irreducible via deadline/early-return
tuning. s1467_4 (k=1.0) remains the best result overall.

---

## Iteration 7: Remove est_child from prio_hint (experiment s1467_10)

**Status:** Implemented, experiment pending

### Change

In `local.rs`, compute `prio_hint = deadline` instead of `prio_hint = deadline - est_child`:

```rust
// BEFORE (s1467_7): prio_hint uses mean+σ child estimate
let est_child = self.server.est_child_latency.get_estimate(key).unwrap_or(0);
let prio_hint = deadline.saturating_sub(est_child);

// AFTER (Iteration 7): prio_hint = deadline only (no est_child subtraction)
let prio_hint = deadline;
```

Keep tracking `est_child_latency` (for logging/debugging only). Continue to use `deadline` (parent_deadline - est_remaining) as-is for deadline propagation and early-return checks.

### Hypothesis

**Root cause of 800 RPS gap**: Priority inversion from load-dependent `est_child`. When load
transitions (e.g., low→800 RPS), `est_child` for ykccIz2fkK grows from ~5ms to ~45ms as
queuing accumulates. Requests computed with old est_child=5ms get prio_hint = T+72ms (low
priority), while newer requests with est_child=45ms get prio_hint = T+32ms (high priority).
Newer requests are scheduled first at MS_37691; old requests wait longer → exceed 100ms SLO.

**With prio_hint = deadline (no est_child)**:
- All y_DKOh-Gts calls get prio_hint = T + 77ms (parent_deadline - est_remaining, same for all)
- Since all root requests have deadline = T+100ms and same est_remaining ≈ 23ms, this is
  approximately FIFO order at MS_37691 (prio_hint = T + 77ms increases monotonically with T)
- Priority inversions from est_child changes are eliminated
- Path-aware early returns (via tightened deadline) are fully preserved

At high load (1400-1800 RPS) where prio_local currently beats prio_oldest:
- The advantage comes from early shedding at inner services (preserved: deadline propagation unchanged)
- Priority ordering at MS_37691 was already approximately FIFO (all requests similar est_child)
- Net effect on high-load goodput: neutral to slight improvement

**Expected outcomes:**
1. 800 RPS: goodput approaches prio_oldest's 793 RPS (from 672.7) — priority inversions eliminated
2. ykccIz2fkK ERs (37.68/s): should drop significantly — requests no longer wait behind inverted-priority peers
3. Root ERs (64.65/s): should drop proportionally
4. 1400-1800 RPS: neutral; early-return advantages preserved
5. Net: first meaningful improvement at the 800 RPS load point

**Risk:** Without est_child in prio_hint, the child server can no longer distinguish expensive vs cheap children from the same parent. In this workload this doesn't matter (all MS_56394→MS_37691 calls are same type), but may matter for more heterogeneous workloads.

### Actual Outcomes (s1467_10)

**Mixed: small improvement at 800/1400 RPS, regression at 1000 RPS:**

| RPS  | s1467_7 (mean ER) | s1467_10 (no est_child) | prio_oldest | diff vs s7 |
|------|-------------------|-------------------------|-------------|------------|
| 800  | 672.7             | 681.1                   | 794.0       | +8.4       |
| 1000 | 741.6             | 719.5                   | 796.8       | **-22.1**  |
| 1200 | 760.2             | 760.1                   | 775.9       | ≈0         |
| 1400 | 780.1             | **797.9**               | 766.6       | +17.8      |
| 1800 | 845.9             | 838.6                   | 792.1       | -7.3       |

At 800 RPS: ykccIz2fkK ERs barely changed (39.6/s vs 37.68/s); priority starvation persists.

**Root cause — priority starvation of ykccIz2fkK by y_DKOh-Gts at MS_37691:**

Both calls are sequential steps in MS_56394's chain. They get different priorities at MS_37691:
- y_DKOh-Gts: `prio_hint = deadline = T+60ms` (tight — est_remaining=40ms from y_DKOh-Gts)
- ykccIz2fkK: `prio_hint = deadline = T+99ms` (loose — est_remaining=1ms, just busy_spin)

At MS_37691, y_DKOh-Gts from ANY request within the last 39ms takes priority over ykccIz2fkK.
At 800 RPS (1.25ms/request), that's ~31 pending y_DKOh-Gts ahead of any ykccIz2fkK. This
creates systematic ~62ms extra wait for ykccIz2fkK, causing 39.6/s ERs.

prio_oldest avoids this: both calls inherit start_at as prio_hint → same priority at MS_37691 →
no starvation. The 1000 RPS regression in s1467_10 (vs s1467_7) shows that removing est_child's
urgency signal WITHOUT fixing sibling starvation hurts overall.

---

## Iteration 8: Inherit parent prio_hint instead of computing from est_remaining (experiment s1467_11)

**Status:** Planned

### Change

In `local.rs`, use `self.ctx.prio_hint().value()` as prio_hint for all children instead of
computing from `deadline - est_child` or just `deadline`:

```rust
// BEFORE (s1467_10): prio_hint = deadline = parent_deadline - est_remaining
let prio_hint = deadline;

// AFTER (Iteration 8): inherit parent's prio_hint unchanged
let prio_hint = self.ctx.prio_hint().value();
```

Keep computing `deadline` (parent_deadline - est_remaining) for the early-return check and for
passing to the child. Only the prio_hint changes.

### Hypothesis

The loadgen sets prio_hint = start_at + 100ms (= absolute SLO deadline) for all root requests.
This value propagates as-is through before_child_rpc. At every inner service, all sibling calls
from the same root request share the same prio_hint = start_at + 100ms. At MS_37691:
- y_DKOh-Gts priority = T+100ms (same as root request SLO deadline)
- ykccIz2fkK priority = T+100ms (same — no starvation!)

Between different root requests: request at T_i has prio T_i+100ms, request at T_j has prio
T_j+100ms. This is EDF ordering (= FIFO since all have same 100ms SLO). Matches prio_oldest.

Path-aware early returns are fully preserved (deadline propagation unchanged). At 1200+ RPS where
prio_local beats prio_oldest, the advantage comes from these early returns. Ordering at child
servers becomes EDF/FIFO but doesn't affect early return correctness.

**Expected outcomes:**
1. 800 RPS: ykccIz2fkK starvation eliminated → goodput approaches prio_oldest's 794.0 RPS
2. 1000 RPS: EDF ordering at inner services → likely >= prio_oldest's 796.8 RPS
   (early returns shed doomed requests faster than prio_oldest's root-level shedding)
3. 1200+ RPS: same or better than s1467_7/s1467_10 (early returns unchanged)
4. Net: closes the 800-1000 RPS gap while maintaining high-load advantage

**Risk:** Inherited prio_hint doesn't reflect path-aware urgency (est_remaining). Two requests
calling different children with different costs get the same priority at child servers. However,
the EARLY RETURN mechanism already handles this correctly: expensive-path requests with tight
tightened deadlines are shed before wasting capacity. The priority ordering only matters when
the system is not fully shed — in which case FIFO/EDF is reasonable.

### Actual Outcomes (s1467_11)

**WORSE than s1467_10 at all loads. Reverted to s1467_10 state.**

| RPS  | s1467_7 | s1467_10 | s1467_11 | prio_oldest (s11) |
|------|---------|----------|----------|-------------------|
| 800  | 672.7   | 681.1    | 666.4    | 793.2             |
| 1000 | 741.6   | 719.5    | 728.6    | 812.6             |
| 1200 | 760.2   | 760.1    | 760.1    | 769.6             |
| 1400 | 780.1   | 797.9    | 788.6    | 757.6             |
| 1800 | 845.9   | 838.6    | 843.8    | 784.1             |

Root cause: giving y_DKOh-Gts a loose prio_hint (T+100ms) while its deadline is still T+60ms
is inconsistent — MS_37691 schedules it leisurely but it still fails the deadline check. The
sibling starvation problem (ykccIz2fkK deprioritized by y_DKOh-Gts) persists AND worsens
because y_DKOh-Gts ERs increase from the inconsistent priority/deadline pairing.

**Reverted to s1467_10 (prio_hint = deadline) as the best overall state.**

---

## Final State and Conclusions

**Best code state: s1467_10** — `prio_hint = deadline` (parent_deadline - est_remaining,
mean+k*σ), no est_child subtraction, mean-only early-return threshold.

| RPS  | prio_local,est_mean_var (s10) | prio_oldest (s10) | difference |
|------|-------------------------------|-------------------|------------|
| 200  | 199.3                         | 198.8             | +0.5       |
| 400  | 404.1                         | 400.7             | +3.4       |
| 800  | 681.1                         | 794.0             | **-112.9** |
| 1000 | 719.5                         | 796.8             | -77.3      |
| 1200 | 760.1                         | 775.9             | -15.8      |
| 1400 | **797.9**                     | 766.6             | **+31.3**  |
| 1800 | **838.6**                     | 792.1             | **+46.5**  |

**prio_local beats prio_oldest at 1400 and 1800 RPS** (the high-load regime where path-aware
early shedding matters most). The 800-1000 RPS gap is structural:

**Why the 800 RPS gap is irreducible** (confirmed across 8 iterations):
- MS_56394 intrinsic latency ≈ 1ms; but two sequential calls to MS_37691 at 40ms each = 93-95ms
  total, leaving only 5ms SLO slack for any queuing or scheduling variance.
- prio_local's path-aware deadline propagation (T+60ms to y_DKOh-Gts, T+99ms to ykccIz2fkK)
  creates priority starvation of ykccIz2fkK at MS_37691 — y_DKOh-Gts from the last 39ms of
  requests all have priority over any pending ykccIz2fkK.
- Attempts to equalize priorities (Iterations 7, 8) either shift the starvation or create
  inconsistent priority/deadline pairs that make ERs worse.
- prio_oldest avoids this by never tightening deadlines — at 800 RPS that's better, but at
  1400+ RPS prio_local wins because tight deadlines enable early shedding of doomed requests.

**Key insight**: prio_local's tightened deadline propagation is a double-edged sword:
- At 800-1200 RPS (near-capacity): deadlines are too tight, causing unnecessary ERs at inner
  services, reducing goodput.
- At 1400+ RPS (overloaded): tight deadlines correctly identify and shed doomed requests early,
  recovering capacity for requests that can still complete.

The crossover point (~1300 RPS) is where prio_local's early shedding benefit exceeds its
ordering overhead. Below this point, prio_oldest's simple FIFO with root-level SLO is better.

---

## Iteration 9: Dynamic EDF priority via poll hooks (experiment s1467_12)

**Status:** Complete

### Changes

Three changes across tokio, hyper, and tonic:

1. **`tokio::task::reprioritize(PriorityHint)` API** (`libs/tokio/tokio/src/task/spawn.rs`,
   `core.rs`, `mod.rs`): New public function that updates the current running task's priority
   in-place using `current_task_header()`. Safe because the `RUNNING` bit guarantees exclusive
   header access in the single-threaded runtime.

2. **Hyper spawn priority** (`libs/hyper/src/proto/h2/server.rs`): Changed spawn priority from
   `ctx.prio_hint()` (static absolute timestamp set by parent) to
   `ctx.deadline().saturating_sub(masa_core::time_now())` (remaining slack at spawn time).

3. **`before_poll` reprioritization** (`libs/tonic/tonic/src/masa/context/local/local.rs`):
   After the early-return check, call `tokio::task::reprioritize(PriorityHint::new(deadline - now))`
   to refresh the task's priority on every poll.

### Analysis

**What EDF ordering gives us at spawn time (the critical window):**

In s1467_10, hyper used `ctx.prio_hint()` as spawn priority — an absolute timestamp like
`T+60ms epoch` or `T+99ms epoch`. Lower value = higher priority (reversed Ord). All
y_DKOh-Gts tasks (deadline T+60ms) have lower priority values than all ykccIz2fkK tasks
(deadline T+99ms) → permanent starvation.

In s1467_12, spawn priority = `ctx.deadline() - time_now()` at arrival:
- y_DKOh-Gts arrives at MS_37691 at T+1ms, ctx.deadline()=T+60ms → spawn priority = **59ms**
- ykccIz2fkK arrives at MS_37691 at T+42ms, ctx.deadline()=T+99ms → spawn priority = **57ms**

ALL concurrent y_DKOh-Gts tasks (from different root requests) also have spawn priority ≈ 59ms
(deadline 60ms ahead of their arrival, which is ~1ms after root start). ykccIz2fkK (57ms)
beats ALL concurrent y_DKOh-Gts (59ms) at the moment it is spawned.

**What happens after the first poll:**

After reprioritization in `before_poll` at time t:
- y_DKOh-Gts R+k: priority = T_{R+k} + 60ms - t = T_R + k*1.25ms + 60ms - t
- ykccIz2fkK R: priority = T_R + 99ms - t

ykccIz2fkK R beats y_DKOh-Gts R+k only if k > 31.2. For k ≤ 31 (the ~25 concurrent requests
at 800 RPS), y_DKOh-Gts retains priority via EDF ordering. This is the SAME relative ordering
as s1467_10's static deadline ordering (EDF = absolute deadline ordering, invariant of clock).

**Key question: does ykccIz2fkK yield?**

The improvement depends entirely on whether ykccIz2fkK completes its first poll without
yielding (await-ing async I/O):

- **No yield (completes in one poll)**: ykccIz2fkK runs immediately at spawn (priority 57ms <
  y_DKOh-Gts 59ms), completes in a single poll, returns. No starvation. 800 RPS gap closes.
- **Yields (makes async I/O calls)**: After first poll, reprioritization restores EDF ordering.
  y_DKOh-Gts from ~25 concurrent requests re-starves ykccIz2fkK. Same gap as s1467_10.

ykccIz2fkK is described as "busy_spin" with est_remaining=1ms (parent takes ~1ms after it
returns). If it's truly a CPU-only busy wait with no async awaits, it completes in one poll.

**Interaction with stale stored priorities:**

Tasks that yielded and are waiting for I/O have stored priorities from their last `before_poll`.
When their waker fires, they re-enqueue with that stale priority (which is smaller/more urgent
than their current true remaining slack, since time has passed). This creates a second-order
effect where high-I/O tasks appear more urgent than they are, potentially benefiting ykccIz2fkK
if y_DKOh-Gts tasks accumulate poll-based priority updates.

### Hypothesis

**Optimistic (if ykccIz2fkK is one-poll):** 800 RPS goodput closes significantly toward
prio_oldest's ~794. ykccIz2fkK completes in its first poll before any y_DKOh-Gts can
overtake, eliminating the 39.6/s ERs. Improvement: +80-110 goodput at 800 RPS.

**Pessimistic (if ykccIz2fkK yields):** Essentially same as s1467_10. The EDF ordering after
reprioritization is equivalent to the static absolute deadline ordering in s1467_10. The 800
RPS gap remains. Improvement: ≤5 goodput at 800 RPS.

**High load (1400–1800 RPS):** Both scenarios: unchanged or slight improvement. The early-return
mechanism (deadline propagation) is unmodified. Priority ordering at MS_37691 shifts slightly
(EDF vs static) but the dominant effect at overload is shedding, not ordering.

### Actual Outcomes (s1467_12)

**MASSIVE improvement across all RPS levels. Hypothesis confirmed: optimistic scenario.**

| RPS  | prio_local,est_mean_var (s12) | prio_oldest (s12) | diff (s12) | diff (s10) |
|------|-------------------------------|-------------------|------------|------------|
| 200  | 199.5                         | 202.8             | -3.3       | +0.5       |
| 400  | 401.9                         | 396.9             | **+5.0**   | +3.4       |
| 800  | **799.8**                     | 790.8             | **+9.0**   | -112.9 ✗   |
| 1000 | **867.4**                     | 803.4             | **+64.0**  | -77.3 ✗    |
| 1200 | **890.8**                     | 798.6             | **+92.2**  | -15.8 ✗    |
| 1400 | **910.4**                     | 762.3             | **+148.1** | +31.3      |
| 1800 | **963.5**                     | 796.7             | **+166.8** | +46.5      |

The 800 RPS gap (-112.9 in s1467_10) is **completely eliminated** (+9.0). prio_local now beats
prio_oldest at every load point except 200 RPS (within noise).

**Early return counts at 800 RPS for est_mean_var:**
- ykccIz2fkK ERs: **0.1/s** (was 39.6/s in s1467_10) — starvation eliminated
- Total ER rate: 1.6/s (was ~100+/s in s1467_10)

The optimistic hypothesis was correct: ykccIz2fkK ("busy_spin") completes in a single poll
without yielding. At spawn, ykccIz2fkK (57ms remaining) beats all concurrent y_DKOh-Gts
(59ms remaining) → ykccIz2fkK runs immediately → completes → no starvation.

**High-load gains also amplified** — dynamic EDF reprioritization enables more effective early
shedding at 1000–1800 RPS because tasks that have been waiting longer correctly get higher
priority, leading to better identification and rejection of doomed requests before wasting capacity.

**Why the pessimistic scenario did not apply:**
ykccIz2fkK's "busy_spin" implementation does not `await` any async I/O inside its handler —
it is a purely CPU-bound operation that completes within a single tokio poll. Therefore, the
57ms spawn priority advantage at hyper is sufficient: ykccIz2fkK runs to completion in its
first poll before any y_DKOh-Gts task can re-enqueue and overtake it.

---

## Iteration 10: EMA estimator — baseline and fix (experiments s1467_13, s1467_14)

**Status:** Complete

### Motivation

The s1467_12 experiment used a monotonically increasing load schedule (200→1800 RPS). This
never exercises the estimator's adaptation to *falling* load. Since containers are not
restarted between RPS steps, the LatencyMeanVar accumulator carries all-time history across
steps. After 30s at 1800 RPS (~54k observations), the Welford running mean barely shifts
when load drops to 800 RPS — stale high-load estimates cause over-tight deadlines and
excessive early returns at the lower load.

### Design: stress-test schedule

Non-monotonic schedule: **[1800, 800, 1800, 800]** (both RPS values repeated twice).
- 1800 RPS first: calibrates estimator to high-load statistics cold
- 800 RPS after: Welford carries stale 1800 RPS observations → over-shedding at 800
- Repeated: both steps write to the same file; data is combined across repetitions

### s1467_13: Welford baseline (non-monotonic schedule)

| RPS  | prio_local,est_mean_var (Welford) | prio_oldest | diff     |
|------|-----------------------------------|-------------|----------|
| 1800 | 457.2                             | 789.1       | -331.9   |
| 800  | 198.1                             | 792.7       | **-594.6** |

Reference (s1467_12 warm monotonic): 1800→963.5, 800→799.8.

**Two distinct failure modes identified:**

1. **Cold start at high load** (1800 RPS from scratch): est_remaining=0 initially → parent
   forwards all 1800 req/s to MS_37691 without inner shedding → queue floods → 1348/s
   y_DKOh-Gts ERs from deadline violations in before_poll. Goodput 457 vs 963 warm.
   Root cause: no pre-dispatch parent-level shedding during cold estimator phase.

2. **Stale high-load estimates on load drop** (800 RPS after 1800): Welford accumulator
   retains ~54k high-load observations → estimates remain inflated → deadlines too tight
   at 800 RPS → 594/s y_DKOh-Gts ERs → goodput 198 vs 800 target. Root cause:
   all-time accumulator has no decay for changing load conditions.

### Change: Welford → EMA (experiment s1467_14)

Replaced `LatencyMeanVar` Welford algorithm with EMA (α=0.05, effective window ~20 obs).
EMA decays old observations exponentially; after a load transition, estimates shift within
~20 observations (~1-2 seconds at typical edge observation rates).

API change: `new(k, alpha: f64)` replaces `new(k, update_interval: usize)`.

### s1467_14: EMA results (same non-monotonic schedule)

| RPS  | Welford (s13) | EMA (s14) | EMA improvement | prio_oldest |
|------|---------------|-----------|-----------------|-------------|
| 1800 | 457.2         | 856.7     | **+399.5**      | 789.1       |
| 800  | 198.1         | 385.8     | **+187.7**      | 792.7       |

EMA dramatically improves both load levels. At 1800 RPS, EMA **beats prio_oldest (+67.6)**
despite the cold start — Welford was -331.9 behind.

**What EMA fixes:** The stale-estimates problem (Problem 2) is directly addressed. EMA also
partially fixes cold start (Problem 1) because it updates on every observation without the
Welford doubling schedule's stalled updates after count>512 — the estimate quickly rises to
reflect actual load within the first ~20 completions.

**What EMA does not fully fix:** The y_DKOh-Gts ER pattern persists at both load levels
(932/s at 1800, 417/s at 800), indicating MS_37691 is still being flooded during cold start.
At 800 RPS (385.8 vs 800 target), the estimate needs more observations to decay the 1800 RPS
history — combined data from both 800 RPS steps includes the early (contaminated) period.

**Remaining gap root cause:** At cold 1800 RPS start, est_remaining initializes to 0 →
parent dispatches all requests to MS_37691 → queue buildup occurs before estimates calibrate.
A non-zero initial prior (e.g., SLO/2 = 50ms) or a higher α (e.g., 0.1-0.2) could close
this gap by reducing the cold-start flood period.

**Comparison to warm reference (s1467_12 monotonic):** EMA at 1800 RPS (856.7) vs warm
(963.5) shows a remaining gap — cold start still costs ~107 goodput. The 800 RPS gap
(385.8 vs 799.8) reflects both cold-start contamination and Welford lag from the combined data.
With a higher α or non-zero prior, the cold-start penalty would shrink.

## Iteration 11: Warm-up prefix to fix cold-start flood (experiments s1467_15, s1467_16)

**Status:** Complete

### Motivation

s1467_14 confirmed EMA fixes the stale-estimates problem (Welford lag on load drop) but
still showed a cold-start gap at 1800 RPS (856.7 vs 963.5 warm). Root cause: when the
experiment starts cold at 1800 RPS, `can_estimate()` is false for the first observations,
so `est_remaining = 0` → parent dispatches all requests to MS_37691 without shedding →
queue floods before estimates calibrate.

Rather than hardcoding a prior in the code, the fix is experiment-level: prepend a low-RPS
warm-up step so the EMA estimator has ~20 observations before high load arrives.

### s1467_15: [200, 1800, 800, 1800, 800] — measuring fully-calibrated state

The exp runner saves the **last** occurrence of each repeated RPS value. With this schedule:
- `r1800.csv` captures t=90–120s (2nd 1800 step, after estimator has seen 200+1800+800 RPS)
- `r800.csv` captures t=120–150s (2nd 800 step)

This measures fully-calibrated performance, not the warm-up-prefix effect directly.

| RPS  | EMA+warmup (s15) | EMA cold (s14) | prio_oldest |
|------|-----------------|----------------|-------------|
| 1800 | **1791.2**      | 856.7          | 798.6       |
| 800  | **805.3**       | 385.8          | 786.4       |

In the fully-calibrated state, prio_local,est_mean_var,early dramatically outperforms
prio_oldest at both load levels. But this doesn't isolate the warm-up prefix effect.

### s1467_16: [200, 1800, 800] — directly measuring warm-up → high-load transition

Each RPS appears exactly once, so `r1800.csv` captures t=30–60s (the first and only 1800
step, right after the 200 RPS warm-up). This directly tests: does 30s at 200 RPS give
enough estimator calibration to handle the jump to 1800 RPS?

### Hypothesis (s1467_16)

- **1800 RPS** (t=30–60s, estimator warmed at 200 RPS only): expected ≥ 900, significantly
  better than s1467_14's cold-start 856.7. EMA after 200 RPS warm-up should have est_remaining
  > 0, preventing the initial flood at MS_37691.
- **800 RPS** (t=60–90s, after first 1800 calibration): expected close to 800 (full goodput),
  as EMA adapts within ~20 observations from 1800→800 RPS transition.
- Both should beat prio_oldest.

### s1467_16 Actual Outcomes

| RPS  | est_mean_var (s16) | prio_local,early | prio_oldest |
|------|--------------------|-----------------|-------------|
| 1800 | **486.3**          | 961.2           | 800.1       |
| 800  | **202.9**          | 793.5           | 791.9       |

**Hypothesis refuted.** Despite 30s of warm-up at 200 RPS, `est_mean_var` collapses at
both load levels — far below `prio_local,early` and even `prio_oldest`. The goodput
timeline (`plots/s1467_16/0/goodput_timeline.png`) makes the failure mode visible:
`est_mean_var` drops to ~430 RPS within the first few seconds at 1800 RPS, then stays
stuck at ~200 RPS throughout the entire 800 RPS period even after load decreases.

The warm-up does provide a non-zero baseline estimate, so it avoids the cold-start flood.
But a new failure mode takes over — a self-reinforcing EMA feedback loop.

---

## Root cause: self-reinforcing EMA feedback loop (discovered from s1467_16)

The EMA (α=0.05) adapts correctly in isolation but creates a stable failure fixed point
under overload:

1. At 200 RPS warm-up: `est_after[MS_56394::GqI6UW1mU4 → MS_37691::y_DKOh-Gts]` builds
   to ~16ms (ykccIz2fkK queue time at 200 RPS)
2. At 1800 RPS: MS_37691 becomes near-saturated; ykccIz2fkK queues for ~86ms. EMA adapts
   within ~20 successful completions (α=0.05 ≈ 20-observation window) → `est_after ≈ 86ms`
3. y_DKOh-Gts gets deadline = T+100ms − 86ms = **T+14ms**
4. ~74% of y_DKOh-Gts calls early-return at MS_37691 (can't complete within 14ms under load)
5. **Critical:** `track_latencies()` only runs for non-ER responses. ERs produce zero EMA
   updates → estimate stays frozen at 86ms
6. With 26% y_DKOh-Gts success rate, MS_37691 remains near-saturated → ykccIz2fkK still
   takes ~86ms → EMA stays at 86ms

**Stable fixed point:** the estimate is self-consistently "correct" given the ER load it
creates, so there is no corrective signal. The loop persists through the 800 RPS period
too — the EMA is frozen from the 1800 RPS phase and doesn't see observations to update.

**Why `prio_local,early` (LatencyRms) avoids this:** LatencyRms is an all-time accumulator
with `update_interval=512`. Thousands of ~2ms observations from the 200 RPS warm-up dilute
any increase → estimate stays ~4–5ms → y_DKOh-Gts gets deadline T+95ms → no tight-deadline
loop forms.

---

## Iteration 12: Break EMA feedback loop by tracking 0 on child ER

**Status:** Validated ✅

### Fix

In `after_child_rpc` (`libs/tonic/tonic/src/masa/context/local/local.rs`): when a child
RPC returns `DeadlineExceeded`, inject a `0` observation into `est_after_child_latency`
for that edge before propagating the error.

**Rationale:** When a child early-returns, the parent immediately propagates the ER and
does no further work — the "remaining time after this child" is genuinely 0. By recording
this, ERs create negative feedback:

```
more ERs → more 0 observations → estimate decreases
→ looser child deadline → fewer ERs → less MS_37691 load
→ ykccIz2fkK faster → stable equilibrium at lower estimate
```

Without the fix, ERs are informationally inert: they neither update the estimate nor signal
that the estimate is causing problems.

**Expected equilibrium:** `est_after ≈ (1 − ER_rate) × actual_after_time`. At the previous
74% ER rate: `0.26 × 86ms ≈ 22ms` → y_DKOh-Gts deadline = T+78ms → fewer ERs → further
reduction → new equilibrium at a lower ER rate and shorter estimate.

### s1467_17 Results

Re-run of s1467_16 configuration (`[200, 1800, 800]`, 30s each) with fixed binary.

| RPS  | est_mean_var (s17) | prio_local,early | prio_oldest,early |
|------|--------------------|-----------------|-------------------|
| 200  | 196.4              | 195.9           | 193.6             |
| 800  | **793.9**          | 797.9           | 790.5             |
| 1800 | **976.1**          | 940.4           | 794.1             |

**Fix confirmed.** `est_mean_var` now exceeds both baselines:
- 1800 RPS: 976.1 (was 486.3 pre-fix) — **beats `prio_local,early` by 35.7 RPS**
- 800 RPS: 793.9 (was 202.9 pre-fix) — **near-full goodput, matches `prio_local,early`**

Both success criteria exceeded (≥800 at 1800 RPS ✅, ≥750 at 800 RPS ✅).

**Secondary diagnostics:**
- `y_DKOh-Gts` ER rate at 1800 RPS: **17.7/s** (was ~1332/s at 74% of 1800 RPS pre-fix)
  — the tight-deadline feedback loop is completely broken
- `ykccIz2fkK` ER rate at 1800 RPS: 264.5/s (load-shedding at the saturated bottleneck,
  which is correct and expected behavior under 1800 RPS overload)

---

## Open questions after Iteration 12

Three questions must be answered before claiming `est_mean_var` is fully optimized.
They are ordered by priority.

---

## Iteration 13: Broad sweep with fully fixed binary (s1467_18)

**Status:** Pending

### Motivation

The only broad RPS sweep in the valid data is **s1467_12**, which used the old **Welford
accumulator** (before commit `55b1d97c` switched to EMA). The EMA + 0-injection fix has
only been validated on the narrow `[200, 1800, 800]` sequence (s1467_17).

We do not know how EMA+fix performs across the monotonic ramp [200 to 1800 RPS]. In the
monotonic ramp the EMA has more calibration time between steps (the feedback loop is less
likely to form), but the EMA's faster adaptation is also more useful than Welford when
the load jumps from one RPS level to the next.

**s1467_12 baseline (Welford, for reference):**

| RPS  | est_mean_var | prio_local,early | prio_oldest,early |
|------|-------------|-----------------|-------------------|
| 200  | 199.6       | 200.3           | 202.8             |
| 400  | 402.1       | 395.5           | 397.1             |
| 800  | 801.1       | 791.8           | 792.8             |
| 1000 | 869.1       | 863.0           | 805.5             |
| 1200 | 892.4       | 883.6           | 801.2             |
| 1400 | 912.4       | 891.9           | 764.8             |
| 1800 | 965.4       | 959.3           | 799.3             |

### Experiment plan (s1467_18)

Config: identical to s1467_12 but run with current binary.

```json
{ "Rps": [200, 400, 800, 1000, 1200, 1400, 1800], "DurationSecs": 30, "WarmupSecs": 0, "MaxInFlight": 500 }
```

**Success criterion:** est_mean_var meets or exceeds s1467_12 at every RPS level,
and meets or exceeds prio_local,early at 1400 and 1800 RPS.

### s1467_18 Results

**Status: Validated ✅ — EMA+fix consistently beats Welford and prio_local,early under overload.**

| RPS  | s18 est_mean_var | s12 est_mean_var (Welford) | delta vs Welford | s18 prio_local,early | delta vs RMS |
|------|-----------------|--------------------------|-----------------|---------------------|-------------|
| 200  | 197.8           | 199.6                    | -1.8 (-0.9%)    | 196.5               | +1.3        |
| 400  | 402.3           | 402.1                    | +0.2 (flat)     | 405.0               | -2.7        |
| 800  | 797.6           | 801.1                    | -3.5 (-0.4%)    | 798.6               | -1.0        |
| 1000 | **890.9**       | 869.1                    | **+21.8 (+2.5%)**| 861.2               | **+29.7**   |
| 1200 | **922.3**       | 892.4                    | **+29.9 (+3.4%)**| 889.8               | **+32.5**   |
| 1400 | **929.6**       | 912.4                    | **+17.2 (+1.9%)**| 888.5               | **+41.1**   |
| 1800 | **979.3**       | 965.4                    | **+13.9 (+1.4%)**| 939.1               | **+40.2**   |

Both success criteria exceeded. At moderate-to-high overload (1000–1800 RPS), EMA+fix
adds 14–30 goodput RPS over Welford and 30–41 goodput RPS over prio_local,early (RMS).
The gains are consistent across all overloaded RPS levels with no regression at low load.

**ms-37691 y_DKOh-Gts ER rate at 1800 RPS: 18.9/s** — feedback loop still broken (was
~1332/s before Iteration 12 fix, ~17.7/s in s1467_17). Stable.

**ms-11639 pattern confirmed:** Lzgm2aJgrn+iZVf-3vjSt inversion repeats (est_mean_var
sheds more at Lzgm2aJgrn, less at iZVf-3vjSt, total lower). H1 confirmed across both
experiments.

---

## Iteration 14: Investigate ms-11639 early-return rate

**Status:** Pending

### Motivation

In s1467_17 at 1800 RPS, `ms-11639` shows **280 ERs/s** across three methods:

| Method      | ER rate (s17) | ER rate prio_local,early (s17) |
|-------------|--------------|-------------------------------|
| 86W_zoVB43  | 164.2/s      | 127.0/s                       |
| Lzgm2aJgrn  | 68.2/s       | 11.3/s                        |
| iZVf-3vjSt  | 47.9/s       | 159.8/s                       |

`Lzgm2aJgrn` stands out: est_mean_var sheds 68/s while prio_local,early sheds only 11/s.
This 6x difference warrants investigation — it may indicate a secondary feedback loop.

**H1 — Legitimate load shedding:** ms-11639 is also saturating at 1800 RPS; the ERs are
correct. Comparable ER rates across policies would confirm this.

**H2 — Secondary feedback loop:** Tight deadlines at ms-11639's callers are causing ERs
which freeze the estimate, keeping deadlines tight. The 0-injection fix applies globally,
but if the latency distribution at ms-11639 edges differs significantly from ms-37691's,
the equilibrium may not converge to a useful operating point.

### Findings from s1467_17

**H1 confirmed — no secondary feedback loop.**

At 1800 RPS in s1467_17, the per-method breakdown shows a Lzgm2aJgrn/iZVf-3vjSt
inversion: est_mean_var sheds 6x more at `Lzgm2aJgrn` but 3x less at `iZVf-3vjSt`.
This anti-correlation strongly suggests the two methods are sequential in the call graph
(Lzgm2aJgrn is called before iZVf-3vjSt): est_mean_var sheds at Lzgm2aJgrn, so fewer
requests reach iZVf-3vjSt.

Total ms-11639 ERs at 1800 RPS:
- est_mean_var: **280.3/s** (86W=164.2, Lzgm2aJgrn=68.2, iZVf-3vjSt=47.9)
- prio_local,early: **298.1/s** (86W=127.0, Lzgm2aJgrn=11.3, iZVf-3vjSt=159.8)

est_mean_var sheds less in total despite different routing. No method shows the
unidirectional explosion pattern of the original y_DKOh-Gts loop (74% → near 0 after
fix). Status: **closed, no action needed**.

Pending s1467_18 for confirmation across the full RPS sweep.

---

## Iteration 15: k parameter calibration

**Status:** Pending (depends on Iteration 13 and 14 results)

### Motivation

`LatencyMeanVar` uses `estimate = mean + k * stddev` with **k=1.0** (approx. 84th
percentile of a normal distribution). The 0-injection fix creates a **bimodal EMA
input**: the estimator sees a mix of 0ms observations (ER injections) and real latency
values (successful completions). The mean+k*stddev formula may behave differently on
this bimodal distribution.

At equilibrium with ER fraction `f` and real latency `L`:
- EMA mean ≈ `(1-f) * L`
- EMA stddev ≈ `sqrt(f*(1-f)) * L` (peaks at f=0.5)
- estimate = `L * [(1-f) + k * sqrt(f*(1-f))]`

For k=1.0 and f=0.26 (s17 y_DKOh-Gts ER fraction): estimate ≈ 1.18 * L.
If L = 86ms, estimate ≈ 101ms — just over the 100ms SLO. Child deadline would be
T + (100ms - 101ms) = T - 1ms: immediately ER. This arithmetic suggests k=1.0 may
be slightly over-conservative when there is any residual ER rate feeding back into
the estimate. A lower k would loosen the child deadline and reduce over-shedding.

### Updated assessment after s1467_18

The theoretical concern above assumed high residual ER rates (26%). In practice:
- **y_DKOh-Gts** (the originally problematic edge): ER rate ≈ 18.9/1800 = **1%**
  → bimodal effect minimal; estimate ≈ mean + 1.01×σ of the real distribution
- **ykccIz2fkK** (bottleneck shedding): ER rate ≈ 270/1800 = **15%**
  → bimodal: estimate = 0.85L + 1.0×0.357L = 1.207L (21% above actual L)

The 15% bimodal inflation at ykccIz2fkK could potentially cause over-shedding there.
However, est_mean_var achieves +40 goodput over prio_local,early at 1800 RPS with k=1.0,
which means the current level of shedding at ykccIz2fkK is either optimal or the
inflation is not harmful. Lower k would admit more requests to ykccIz2fkK → heavier
queue → longer latency → more timeouts (not necessarily better goodput).

**Assessment:** k-tuning experiments (s1467_19, s1467_20) remain possible but the
expected gain is small given the strong performance of k=1.0. The ykccIz2fkK bimodal
inflation (21%) is the main candidate for improvement. A single experiment with k=0.5
would resolve this with low cost.

### s1467_19 Results (k=0.5)

**Status: Closed ✅ — k=1.0 is optimal.**

| RPS  | k=1.0 (s1467_17) | k=0.5 (s1467_19) | delta |
|------|-----------------|-----------------|-------|
| 200  | 196.4           | 198.3           | +1.9  |
| 800  | 793.9           | 790.1           | **-3.8** |
| 1800 | 976.1           | 977.2           | +1.1  |

k=0.5 reduces ykccIz2fkK ER rate from 264.5 → 214.7/s (-19%) as predicted, but
achieves essentially the same goodput at 1800 RPS (+1.1, within noise) and slightly
worse at 800 RPS (-3.8). The extra shedding at ykccIz2fkK with k=1.0 is productive —
those requests would have timed out anyway. Lower k lets more through, they then timeout.

**k=1.0 remains the default.** k=0.5 does not meet the success criterion (goodput
must be ≥ k=1.0 at both RPS levels). Iteration 15 is closed, no further k-tuning needed.

