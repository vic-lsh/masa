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

**Status:** Running

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
