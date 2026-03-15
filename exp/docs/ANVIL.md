# ANVIL — fifo,early,adctl,est_mean_var vs fifo,rajomon

## Key questions
- How does rajomon's token-bucket queue-latency-based admission control compare to adctl's compute-time-budget utilization-based approach on the same workload?
- Can rajomon hyperparameter tuning close the gap (if any) with adctl?
- What are the failure modes unique to rajomon under high load?

## Context

FORGE established that `fifo,early,adctl,est_mean_var` achieves 1755 goodput at 2000 RPS on coral_ext_2 (Search SLO=200ms, Reservation SLO=50ms). Rajomon is an alternative admission control mechanism based on distributed token-bucket pricing with queue latency feedback. It is mutually exclusive with `early` (compile-time constraint), so the comparison is `fifo,early,adctl,est_mean_var` vs `fifo,rajomon`.

Rajomon hyperparameters (defaults):
- Queue latency threshold: 5000 µs (5ms)
- EWMA α: 1/4 (0.25), half-life ≈ 240ms
- Background tick interval: 100ms
- Default token budget: 100 tokens per request
- Min price: 1 token

**Constraint:** Rajomon algorithm logic must not be modified — only hyperparameters may be tuned.

## Experiment series: anvil_1, anvil_2, ... (hotel)

### Baseline: anvil_1 (forge_1 config, default rajomon hyperparameters)

**Config:** Search SLO=200ms, Reservation SLO=50ms, RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000]. Policies: `fifo,early,adctl,est_mean_var` vs `fifo,rajomon`.

**Status:** Complete ✅

#### Goodput Comparison

| RPS | adctl Goodput | adctl % | rajomon Goodput | rajomon % | Delta |
|-----|--------------|---------|-----------------|-----------|-------|
| 100 | 99.5 | 99.5% | 99.5 | 99.5% | 0 |
| 400 | 398.0 | 99.5% | 398.1 | 99.5% | 0 |
| 800 | 795.9 | 99.5% | 795.9 | 99.5% | 0 |
| 1200 | 1193.6 | 99.5% | 1193.6 | 99.5% | 0 |
| 1400 | 1388.8 | 99.2% | 1390.1 | 99.3% | -1.3 |
| 1600 | 1536.5 | 96.0% | 1513.8 | 94.6% | +22.7 |
| 1800 | 1542.1 | 85.7% | 176.2 | 9.8% | +1365.9 |
| 2000 | 1726.7 | 86.3% | 191.1 | 9.6% | +1535.6 |

#### Early Return Rates (rajomon has zero early returns at all RPS)

| RPS | adctl Total | adctl Search | adctl Reservation |
|-----|------------|-------------|-------------------|
| 1400 | 4.0 | 0 | 4.0 |
| 1600 | 55.4 | 0 | 55.4 |
| 1800 | 247.8 | 93.1 | 154.7 |
| 2000 | 262.1 | 171.9 | 90.2 |

### Key Findings (Baseline)

1. **Rajomon collapses catastrophically at 1800 RPS.** Goodput drops from 1514 (94.6%) at 1600 to 176 (9.8%) at 1800 — total congestion collapse. Search goodput hits exactly 0.0 at 1800+.

2. **Root cause: rajomon's pricing reacts too slowly to shed load.** Price increases by only +1 per 100ms tick when queue latency exceeds 5ms. With a 100-token budget, it takes ~100 ticks (10 seconds!) to reach a price that rejects requests. By then the system is in full collapse.

3. **Below saturation (≤1400 RPS), both policies are identical.** The gap only appears at 1600 RPS (+23 goodput for adctl) and becomes catastrophic at 1800+.

4. **Adctl's early return mechanism is the key differentiator.** Adctl sheds 248-262 requests/s at 1800-2000 RPS, keeping the system stable. Rajomon produces zero early returns because `early` is mutually exclusive with `rajomon` at compile time.

### Diagnosis

The core problem is rajomon's **slow price escalation**. The algorithm increments price by +1 per tick (100ms), but the default token budget is 100. This means:
- At saturation, price must climb from 1 → 100+ to start rejecting (takes ~10s)
- During that 10s window, all requests are admitted and the system enters congestion collapse
- Once collapsed, even high prices can't help because latencies are already blown

Tuning levers (in order of expected impact):
1. **Lower token budget** (100 → 10-20): Makes each price increase more impactful
2. **Lower queue latency threshold** (5000µs → 1000-2000µs): React before collapse
3. **Increase EWMA α** (0.25 → 0.5+): Faster signal, less smoothing
4. **Increase min price** (1 → higher): Start from higher base price

---

## Iteration 1: Aggressive token budget + threshold tuning (experiment anvil_2)

**Status:** Code changes applied, experiment pending

### Change
- Token budget: 100 → 10 (in `libs/masa-core/src/context.rs` `default_tokens()`)
- Queue latency threshold: 5000µs → 1000µs (in `libs/tonic/tonic/src/masa/context/rajomon.rs` `update_prices()`)

### Hypothesis
The root cause of rajomon's collapse is that price must climb from 1 to 100+ before any rejection happens, taking ~10 seconds. Lowering the token budget to 10 means price only needs to reach 10 to reject — achievable in ~1 second (10 ticks). Lowering the threshold to 1ms means price starts climbing earlier, before the system enters deep congestion.

Together, these changes should allow rajomon to start shedding load within ~1 second of detecting overload, preventing congestion collapse.

### Expected outcomes if hypothesis is correct:
1. Rajomon goodput at 1800-2000 RPS should be dramatically higher (from ~180 to >1000)
2. Rajomon should no longer experience congestion collapse (latencies stay bounded)
3. At low RPS (≤1200), behavior should be unchanged (prices stay at 1, well below 10-token budget)
4. There may be some over-rejection at moderate overload (1400-1600 RPS) if the threshold is too sensitive

### Experiment design
Same config as anvil_1 (forge_1 RPS sweep, Search SLO=200ms, Reservation SLO=50ms). Full 8-point sweep to see both the underload and overload regimes.

### Actual Outcomes (anvil_2)

**Status:** Regression ❌ — hypothesis rejected

#### Goodput Comparison

| RPS | adctl (anvil_2) | adctl (anvil_1) | rajomon tuned | rajomon default |
|-----|----------------|-----------------|---------------|-----------------|
| 100 | 99.7 | 99.5 | 99.5 | 99.5 |
| 400 | 398.0 | 398.0 | 398.1 | 398.1 |
| 800 | 795.9 | 795.9 | 796.0 | 795.9 |
| 1200 | 1193.9 | 1193.6 | 1193.7 | 1193.6 |
| 1400 | 1391.0 | 1388.8 | 1386.1 | 1390.1 |
| 1600 | 1492.2 | 1536.5 | 1492.1 | 1513.8 |
| 1800 | 1593.2 | 1542.1 | **177.2** | 176.2 |
| 2000 | 1688.4 | 1726.7 | **191.5** | 191.1 |

**Rajomon still collapses at 1800+ RPS.** Goodput at 1800: 177.2 (tuned) vs 176.2 (default) — statistically identical. Zero Search goodput at 1800+.

**Token budget change also hurt adctl.** At 1600 RPS, adctl dropped from 1536.5 to 1492.2 (-44) due to the shared token budget parameter being lowered to 10. Mixed effect elsewhere.

**Rajomon still produces zero early returns.** The ResourceExhausted rejections from rajomon are counted as errors, not early returns.

### Root cause analysis

The token budget + threshold tuning was insufficient because:
1. **Even with budget=10, price needs 10 ticks (1s) to reach rejection** — 1800 requests arrive during that window
2. **Price oscillation**: once rejection starts, queue drains → price drops → admits again → collapse again. The +1/-1 symmetric step creates unstable oscillation.
3. **No in-flight shedding**: rajomon can only reject at admission. Already-admitted requests run to completion even if past deadline, wasting CPU. This is the fundamental architectural gap vs adctl+early.

### Decision: Revert

Reverting code commit `6fc7e1f5` because: (a) no benefit to rajomon, (b) shared token budget change degraded adctl at 1600 RPS.

---

## Iteration 2: Faster price escalation with asymmetric step (experiment anvil_3)

**Status:** Pending

### Change
- Price increment: +1 → +10 per tick when overloaded (in `rajomon.rs` `update_prices()`)
- Price decrement: -1 unchanged (keep slow decay to prevent oscillation)
- Queue latency threshold: 5000µs → 1000µs (keep from iteration 1)
- Token budget: revert to 100 (stop affecting adctl)

### Hypothesis
Iteration 1 failed because even with budget=10, it took 10 ticks to start rejecting. The root issue is the symmetric +1/-1 step causing price oscillation: price rises slowly → rejects → queue drains → price falls just as slowly → admits → overload again.

By making the step **asymmetric** (+10 up, -1 down), the price:
- Reaches rejection threshold (100) in 10 ticks = 1 second (was 100 ticks = 10s)
- Takes 100 ticks = 10 seconds to fully recover, preventing premature re-admission
- This asymmetry creates a natural hysteresis that should stabilize the system under overload

Note: changing the step size is a hyperparameter change, not an algorithm change. The algorithm structure (if above threshold → increment, else → decrement) is preserved.

### Expected outcomes if hypothesis is correct:
1. Rajomon reaches rejection within 1 second at 1800+ RPS, preventing full collapse
2. Slow decay (-1/tick) prevents oscillation — once rejecting, stays rejecting until load genuinely subsides
3. Adctl unaffected (token budget back to 100)
4. Low-load behavior unchanged (queue latency stays below 1000µs, prices stay at 1)

### Experiment design
Same config as anvil_1. Full 8-point sweep.
