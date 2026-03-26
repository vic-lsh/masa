# DRIFT — fifo,rajomon parameter optimization

## Key questions
- Can `fifo,rajomon` approach `fifo,early,adctl,est_mean_var` goodput by tuning the rajomon constants?
- Which constant has the biggest impact: `LATENCY_THRESHOLD_US` (congestion detection sensitivity), token bucket parameters (`MAX_TOKEN`, `TOKEN_UPDATE_STEP`), or price update rate?
- Does rajomon's queue-latency-based congestion signal provide enough resolution to compete with adctl's utilization-based signal?

## Experiment series: drift_1, drift_2, ... (mssim)
Traces: S_14677443 (est01) and S_32048416 (est02), run as separate experiments per iteration.
RPS sweep: 800, 1400, 1800 only.
Policies: `fifo,early,adctl,est_mean_var` (baseline) and `fifo,rajomon` (target).

---

## Phase 0: Baseline (est01, est02 — existing results)

### Observed Symptoms

**est01 (trace S_14677443) — goodput in req/s:**

| RPS  | fifo,early,adctl,est_mean_var | fifo,rajomon |
|------|-------------------------------|--------------|
| 800  | 684.6 (85.6%)                 | 166.2 (20.8%) |
| 1400 | 886.6 (63.3%)                 | 187.2 (13.4%) |
| 1800 | 1031.4 (57.3%)                | 167.8 (9.3%)  |

**est02 (trace S_32048416) — goodput in req/s:**

| RPS  | fifo,early,adctl,est_mean_var | fifo,rajomon |
|------|-------------------------------|--------------|
| 800  | 338.2 (42.3%)                 | 0.0 (0%)     |
| 1400 | 321.8 (23.0%)                 | 1.3 (0.1%)   |
| 1800 | 303.9 (16.9%)                 | 0.8 (0%)     |

**Diagnosis:**
- `fifo,rajomon` is catastrophically bad in est02 (near-zero) and severely underperforms in est01.
- The root cause is clear from the code: `LATENCY_THRESHOLD_US = 150_000` (150ms) means the congestion signal fires only when scheduler queue latency itself exceeds 150ms — far beyond normal operating conditions. As a result, `own_price` stays at 0 indefinitely, no load is shed at the client, and the system is overwhelmed without any back-pressure from rajomon.
- With price=0, `check_inbound` admits every request (since `tokens < 0` is always false for u64), so rajomon is effectively a no-op admission controller.
- Additionally, `PRICE_UPDATE_RATE_MS = 25` is 2.5× slower than the original Go (10ms), further delaying any response even if the signal did fire.
- `TOKEN_UPDATE_STEP = 5` and `MAX_TOKEN = 100` are 5× and 10× more permissive than Go defaults, making the client bucket very loose.

**Current constants (for reference):**
```rust
PRICE_UPDATE_RATE_MS  = 25      // Go: 10
LATENCY_THRESHOLD_US  = 150_000 // Go: 0 (any queue latency = congestion)
PRICE_STEP            = 1       // Go: 1
INIT_PRICE            = 0       // Go: 0
PRICE_FREQ            = 1       // Go: 5
TOKENS_LEFT_INIT      = 10      // Go: 10
TOKEN_UPDATE_RATE_MS  = 10      // Go: 10
TOKEN_UPDATE_STEP     = 5       // Go: 1
MAX_TOKEN             = 100     // Go: 10
```

---

## Iteration 1: Lower latency threshold to activate congestion signal (drift_1_a, drift_1_b)

**Status:** Complete

### Change
- `LATENCY_THRESHOLD_US`: 150_000 → 1_000 (1ms)
- `PRICE_UPDATE_RATE_MS`: 25 → 10 (match original Go speed)

### Hypothesis
The congestion signal is broken because the threshold is 150ms — scheduler queue latency almost never reaches that under any realistic workload with a 200ms SLO. Lowering it to 1ms means the server will detect queueing far earlier and start raising its price. Once `own_price > 0`, `check_inbound` starts rejecting requests where `tokens < price`, providing actual load shedding. Faster update rate (10ms) tightens the feedback loop.

### Expected outcomes if hypothesis is correct:
1. `fifo,rajomon` goodput in est01 rises substantially above the ~167 baseline, ideally closer to `fifo,early,adctl,est_mean_var`
2. est02 goodput rises from near-zero to measurable values
3. Price should oscillate around some non-zero equilibrium under 1400–1800 RPS

### Experiment design
Run on both traces to check whether the fix is general. Use only RPS 800/1400/1800 as required. The congestion detection fix should be visible even at 800 RPS if the queue latency there is consistently below 1ms — if goodput still drops at 800, that's a sign the threshold is still too high or something else is broken.

### Results (drift_1_a, drift_1_b)

**drift_1_a (trace S_14677443) — goodput in req/s:**

| RPS  | fifo,early,adctl,est_mean_var | fifo,rajomon |
|------|-------------------------------|--------------|
| 800  | 698.6 (87.3%)                 | 202.8 (25.4%) |
| 1400 | 905.8 (64.7%)                 | 348.3 (24.9%) |
| 1800 | 1000.8 (55.6%)                | 459.7 (25.5%) |

**drift_1_b (trace S_32048416) — goodput in req/s:**

| RPS  | fifo,early,adctl,est_mean_var | fifo,rajomon |
|------|-------------------------------|--------------|
| 800  | 337.2 (42.2%)                 | 27.0 (3.4%)  |
| 1400 | 321.8 (23.0%)                 | 0.0 (0%)     |
| 1800 | 315.4 (17.5%)                 | 0.0 (0%)     |

### Analysis

**Hypothesis was only partially correct.** The threshold fix helped est01 (goodput ratio improved from ~10-21% to ~25%), but est02 remained catastrophic (0% at 1400/1800 RPS, vs 0.1% baseline). This reveals additional bugs beyond the threshold:

- The queue latency measurement was accumulating across all polls per request (total sum vs threshold), causing spurious congestion signals — price rises too aggressively, shedding all load.
- Token spending was broken: tokens were set per-channel (not deducted), TOKEN_UPDATE_STEP was 5× too large (5 vs Go's 1), MAX_TOKEN was 10× too large (100 vs Go's 10).
- Price dynamics were symmetric (step=1 both ways), causing slow response to congestion.

All of these were fixed in drift_2 commits (see DRIFT_CHANGES.md items 2-5).

---

## Iteration 2: Fix token bucket, price dynamics, and queue latency measurement (drift_2_a, drift_2_b)

**Status:** Complete

### Changes (on top of drift_1)
- **Queue latency fix**: Per-poll `fetch_max` instead of accumulated sum across polls
- **Price dynamics**: `PRICE_STEP_UP=8`, `PRICE_STEP_DOWN=1`, hysteresis band
- **Token replenishment**: Poisson inter-arrival distribution (mean 10ms), matching Go
- **Token spending**: Uniform random in [0, balance-1], deduct immediately per request (CAS loop)

### Results (drift_2_a, drift_2_b)

| RPS  | est01 adctl | est01 rajomon | est02 adctl | est02 rajomon |
|------|-------------|---------------|-------------|---------------|
| 800  | 699.6       | 208.9 (30%)   | 335.1       | 3.0 (0.9%)   |
| 1400 | 868.4       | 344.6 (40%)   | 320.0       | 0.0 (0%)      |
| 1800 | 1035.9      | 456.7 (44%)   | 336.9       | 0.0 (0%)      |

### Analysis
est01 improved (25%→40%) but est02 still crashes. Root cause: three cascading bugs remained:
1. **Double-counting**: `check_inbound` deducted `accumulated_price` (own + downstream max) but `check_outbound` checked `remaining >= child_price` again → effectively needed `tok >= 2×price` to pass.
2. **Token depletion**: Loadgen deducted `tok` from bucket per request. Bucket equilibrated at ~1 token (spending >> replenishment at high RPS) → `tok=0` always → all inbound gates failed.
3. **Stale cache deadlock**: A transient congestion spike raised downstream price (observed: ms-20664 price=44 propagated to ms-9570→root). Once root blocked all traffic to ms-9570, no responses returned → cached price never updated → permanent 0% goodput.

---

## Iteration 3: Fix double-counting, token depletion, and stale cache deadlock (drift_3_a, drift_3_b)

**Status:** Complete

### Changes
- `check_inbound`: gate uses `accumulated_price` but deducts only `own_price`
- Loadgen: `tok = random(0..=MAX_TOKEN)` always, no bucket deduction
- Worker: clear `downstream_prices` every 1s (100 ticks) to break stale-cache deadlock

### Results (drift_3_a, drift_3_b)

| RPS  | est01 adctl | est01 rajomon | est02 adctl | est02 rajomon |
|------|-------------|---------------|-------------|---------------|
| 800  | 699.6       | 243.5 (35%)   | 335.1       | 0.0 (0%)     |
| 1400 | 868.4       | 371.7 (43%)   | 320.0       | 41.8 (13%)   |
| 1800 | 1035.9      | 263.3 (25%)   | 336.9       | 52.3 (16%)   |

### Analysis
est02 first shows non-zero results at 1400/1800 RPS. But two new issues:
- est02 800 RPS: `/ClientMiss` errors — system overwhelmed before admission control kicks in at low load
- est01 1800 RPS drops: price oscillates wildly (logs show 36→66→4→8→55) because 8/1 asymmetry causes divergence when congested ticks exceed 11% equilibrium

---

## Iterations 4–5: Tuning price step dynamics (drift_4, drift_5)

**Iteration 4** (drift_4_a, drift_4_b) tried symmetric 2/2 steps + 5ms threshold — WORSE (all prices stay 0 with 5ms threshold, no signal fires at all in mssim).

**Iteration 5** (drift_5_a, drift_5_b): reverted to 1ms threshold with 8/2 (up/down) steps for faster recovery than drift_3.

### Results (drift_5_a, drift_5_b)

| RPS  | est01 adctl | est01 rajomon | est02 adctl | est02 rajomon |
|------|-------------|---------------|-------------|---------------|
| 800  | 685.4       | 285.4 (42%)   | 304.6       | 73.5 (24%)   |
| 1400 | 893.9       | 409.2 (46%)   | 289.7       | 75.8 (26%)   |
| 1800 | 1015.4      | 219.0 (22%)   | 286.8       | 72.0 (25%)   |

est02 fixed (73-76 consistently). est01 800/1400 improved but 1800 still drops.

---

## Iteration 6: Add PRICE_CAP to prevent overshoot (drift_6_a, drift_6_b)

**Status:** Complete

### Change
- `PRICE_CAP = MAX_TOKEN * 6/10 = 60`: clamp price on the way up; ensures ≥40% admission even at peak congestion

### Hypothesis
At 1800 RPS est01, own_price observed spiking to 90+ (90% rejection → queue empties → traffic flood → spike again). Binary oscillation instead of stable admission. Cap at 60 maintains minimum throughput.

### Results (drift_6_a, drift_6_b)

| RPS  | est01 adctl | est01 rajomon | est02 adctl | est02 rajomon |
|------|-------------|---------------|-------------|---------------|
| 800  | 703.3       | 305.0 (43%)   | 285.4       | 91.9 (32%)   |
| 1400 | 880.4       | 503.9 (57%)   | 321.5       | 90.7 (28%)   |
| 1800 | 1041.5      | 637.1 (61%)   | 281.2       | 97.4 (35%)   |

est01 1800 jumped from 219 to 637 — the price cap fixed the lockout oscillation.

---

## Iteration 7: Raise threshold to 2ms (drift_7_a, drift_7_b)

**Status:** Complete

### Change
- `LATENCY_THRESHOLD_US`: 1ms → 2ms

### Hypothesis
At 800 RPS, est01 goodput is 43% vs adctl 88%. System is not heavily overloaded (only 12% fail SLO with adctl), but 1ms threshold fires spuriously at moderate queue depth. Raising to 2ms reduces over-triggering at light load while still firing under true overload.

### Results (drift_7_a, drift_7_b)

| RPS  | est01 adctl | est01 rajomon | est02 adctl | est02 rajomon |
|------|-------------|---------------|-------------|---------------|
| 800  | 700.4       | 321.9 (46%)   | 323.0       | 144.2 (45%)  |
| 1400 | 910.1       | 525.7 (58%)   | 290.2       | 127.3 (44%)  |
| 1800 | 1013.2      | 650.8 (64%)   | 290.4       | 127.7 (44%)  |

### Summary (best constants so far)
```rust
LATENCY_THRESHOLD_US  = 2_000  // 2ms
PRICE_STEP_UP         = 8
PRICE_STEP_DOWN       = 2
PRICE_CAP             = 60     // MAX_TOKEN × 60%
MAX_TOKEN             = 100
```

| Trace | RPS  | adctl | rajomon | ratio |
|-------|------|-------|---------|-------|
| est01 | 800  | 700   | 322     | 46%   |
| est01 | 1400 | 910   | 526     | 58%   |
| est01 | 1800 | 1013  | 651     | 64%   |
| est02 | 800  | 323   | 144     | 45%   |
| est02 | 1400 | 290   | 127     | 44%   |
| est02 | 1800 | 290   | 128     | 44%   |

Significant improvement from baseline (rajomon was 0-21% of adctl). Now 44-64% across both traces and all RPS levels.

---

## Iteration 8: Faster price recovery with PRICE_STEP_DOWN=4 (drift_8_a, drift_8_b)

**Status:** Complete — reverted (regression)

### Change
- `PRICE_STEP_DOWN`: 2 → 4

### Hypothesis
At high RPS, recovery from PRICE_CAP=60 takes ~300ms with step=2 (60/2 × 10ms). If overload passes quickly, the system stays over-throttled too long. Halving recovery time to ~150ms might let through more requests during the non-overloaded window.

### Results (drift_8_a, drift_8_b)

| RPS  | est01 adctl | est01 rajomon | est02 adctl | est02 rajomon |
|------|-------------|---------------|-------------|---------------|
| 800  | 703.3       | 396.0 (56%)   | —           | 173.0         |
| 1400 | 880.4       | 292.0 (33%)   | —           | 0.0 (0%)      |
| 1800 | 1041.5      | 741.0 (71%)   | —           | 0.0 (0%)      |

### Analysis
Regression at intermediate load (est01 1400 dropped 526→292; est02 1400/1800 collapsed to 0). Faster recovery from cap means price undershoots during sustained congestion — the system briefly over-admits, queue saturates, then price slams back to cap. This oscillation is worse than drift_7's slower recovery. PRICE_STEP_DOWN=2 is retained.

**Conclusion: drift_7 config is the best found. No further parameter tuning attempted.**

---

## Final Summary

The best rajomon constants found through parameter search:

```rust
LATENCY_THRESHOLD_US  = 2_000  // 2ms
PRICE_STEP_UP         = 8
PRICE_STEP_DOWN       = 2
PRICE_CAP             = 60     // MAX_TOKEN × 60%
MAX_TOKEN             = 100
TOKEN_UPDATE_STEP     = 5
PRICE_UPDATE_RATE_MS  = 10     // 10ms
TOKEN_UPDATE_RATE_MS  = 10     // 10ms
```

Goodput ratio vs `fifo,early,adctl,est_mean_var` (from baseline 0-21% to):

| Trace | RPS  | adctl | rajomon | ratio |
|-------|------|-------|---------|-------|
| est01 | 800  | 700   | 322     | 46%   |
| est01 | 1400 | 910   | 526     | 58%   |
| est01 | 1800 | 1013  | 651     | 64%   |
| est02 | 800  | 323   | 144     | 45%   |
| est02 | 1400 | 290   | 127     | 44%   |
| est02 | 1800 | 290   | 128     | 44%   |

The remaining ~36-56% gap reflects a fundamental architectural difference: rajomon admits randomly (uniform random token bid), while adctl selects based on utilization estimates and per-request cost. Closing this gap requires changing the admission algorithm, not tuning the price dynamics.
