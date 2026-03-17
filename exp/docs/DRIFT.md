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

**Status:** Running

### Changes (on top of drift_1)
- **Queue latency fix**: Per-poll `fetch_max` instead of accumulated sum across polls
- **Price dynamics**: `PRICE_STEP_UP=8`, `PRICE_STEP_DOWN=1`, hysteresis band (hold between half-threshold and threshold)
- **Token replenishment**: Poisson inter-arrival distribution (mean 10ms), matching Go
- **Token spending**: Uniform random in [0, balance-1], deduct immediately per request (CAS loop)

### Hypothesis
The drift_1 failure in est02 was caused by a cascade of bugs: (1) accumulated queue latency causing runaway price increases that shed 100% of load, (2) loose token bucket (step=5, max=100) that flooded requests when price was near-zero, (3) symmetric price steps that couldn't respond fast enough to sudden congestion. The queue latency fix should prevent runaway prices, and the corrected token dynamics should produce smoother load shedding.

### Expected outcomes if hypothesis is correct:
1. est02 goodput rises from 0% to measurable values at all RPS levels
2. est01 goodput improves further above the 25% ratio from drift_1
3. own_price actually rises under load (verifiable from logs: "Rajomon own_price")
