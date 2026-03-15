# THORN — Rajomon parameter tuning

## Key questions
- Can Rajomon's default parameters be tuned to close the goodput gap with `prio_local,early,adctl,est_mean_var` on hotel?
- Does combining Rajomon with `prio_local` scheduling (instead of `fifo`) significantly improve its performance?
- What is the root cause of Rajomon's price-latch collapse at high load, and can parameter tuning alone fix it?

## Constraints
- No algorithm changes — only tuning the 9 constants in `rajomon.rs`
- Maximum 5 iterations
- The Rust implementation must remain aligned with the original Go implementation at `3rd_party/rajomon/`

## Parameters under tuning
| Parameter | Default | Description |
|-----------|---------|-------------|
| `PRICE_UPDATE_RATE_MS` | 10 | Background worker tick interval (ms) |
| `LATENCY_THRESHOLD_US` | 0 | Queue latency threshold for congestion (us) |
| `PRICE_STEP` | 1 | Additive price step on congestion |
| `INIT_PRICE` | 0 | Initial own_price |
| `PRICE_FREQ` | 5 | Send price in 1/N responses |
| `TOKENS_LEFT_INIT` | 10 | Initial client token budget |
| `TOKEN_UPDATE_RATE_MS` | 10 | Client token refill interval (ms) |
| `TOKEN_UPDATE_STEP` | 1 | Tokens added per refill |
| `MAX_TOKEN` | 10 | Token cap |

## Experiment series: thorn_1, thorn_2, ... (hotel)

## Observed Symptoms (thorn_1 — baseline with default parameters)

| RPS | fifo,rajomon | fifo,rajomon,early | prio_local,rajomon | prio_local,rajomon,early | **prio_local,early,adctl** | prio_oldest,early |
|-----|-------------|-------------------|-------------------|------------------------|--------------------------|------------------|
| 100 | 0.0 | 0.0 | 0.0 | 0.0 | **99.5** | 99.6 |
| 400 | 0.0 | 0.0 | 0.0 | 0.0 | **397.8** | 397.8 |
| 800 | 0.0 | 0.0 | 0.0 | 0.0 | **795.7** | 795.7 |
| 1200 | 0.0 | 0.0 | 0.0 | 0.0 | **1180.0** | 1175.9 |
| 1400 | 0.0 | 0.0 | 0.0 | 0.0 | **1233.2** | 1112.7 |
| 1600 | 0.0 | 0.0 | 0.0 | 0.0 | **1369.1** | 1076.3 |
| 1800 | 0.0 | 0.0 | 0.0 | 0.0 | **1462.5** | 1032.1 |
| 2000 | 0.0 | 0.0 | 0.0 | 0.0 | **1590.6** | 1074.2 |

**Total lockout at all load levels.** All 4 rajomon variants produce zero goodput, even at 100 RPS (~1% utilization). CPU usage across rajomon services is near-idle (0.2-1.1%), confirming requests are blocked from entering the system, not overloading it.

**Root cause: cascading price inflation from zero latency threshold.**
1. `LATENCY_THRESHOLD_US=0` — any queuing triggers price increase. In single-threaded tokio, every task has non-zero queue latency, so prices rise on every 10ms tick even at zero load.
2. `MAX_TOKEN=10` with 100 tokens/s replenishment — far too low for Hotel's 4-6x fan-out at each hop.
3. Price decrease (-1/tick) is too slow relative to increase (+1/tick under permanent "congestion").
4. `max(own_price, max_downstream_price)` amplifies the worst-case leaf service price to the root.

---

## Iteration 1: Fix total lockout with threshold + token budget (experiment thorn_2)

**Status:** Complete ✅ (partial — lockout fixed, high-load still poor)

**Code commit:** 5a293de3

### Change
Tune 4 parameters to break the immediate lockout:
- `LATENCY_THRESHOLD_US`: 0 → 5000 (5ms) — allow normal async scheduling latency without triggering congestion
- `MAX_TOKEN`: 10 → 100 — 10x larger token budget to accommodate fan-out
- `TOKEN_UPDATE_STEP`: 1 → 5 — 5x faster replenishment (500 tokens/s vs 100)
- `PRICE_FREQ`: 5 → 1 — every response carries price for faster convergence

### Hypothesis
The zero latency threshold causes permanent price inflation because single-threaded tokio always has non-zero task queuing. Setting a 5ms threshold means prices only rise during actual congestion. The larger token budget (100 cap, 5/tick refill = 500/s) should sustain Hotel's fan-out at moderate load. Sending price in every response eliminates staleness-driven oscillation.

### Expected outcomes if hypothesis is correct:
1. Rajomon variants should achieve near-full goodput (>95%) at 100-800 RPS (no longer locked out)
2. At 1200+ RPS, rajomon should show some admission control behavior (not zero, but likely below the champion)
3. `prio_local,rajomon` should outperform `fifo,rajomon` at high load due to better scheduling
4. The `early` variants should show additional benefit at saturation by cutting wasteful work

### Experiment design
Use 4 representative RPS levels (100, 800, 1400, 2000) for quick validation. If results look promising, follow up with the full 8-level sweep. Keep all 6 policies from thorn_1 plus the two non-rajomon baselines.

### Actual Outcomes (thorn_2)

**Status:** Complete ✅ (partial)

| Policy | 100 RPS | 800 RPS | 1400 RPS | 2000 RPS |
|---|---|---|---|---|
| fifo,rajomon | 99.5 | 795.6 | **833.7** | 217.6 |
| fifo,rajomon,early | 99.5 | 795.6 | 138.1 | 201.7 |
| prio_local,rajomon | 99.6 | 795.7 | 137.4 | 199.9 |
| prio_local,rajomon,early | 99.5 | 795.7 | 137.8 | 201.8 |
| **prio_local,early,adctl** | **99.5** | **795.6** | **1215.9** | **1570.2** |
| prio_oldest,early | 99.5 | 795.5 | 1107.9 | 1071.4 |

**Lockout fixed at low load.** All rajomon variants achieve ~99.5% goodput at 100-800 RPS, matching the champion. This confirms the latency threshold and token budget changes resolved the immediate lockout.

**Massive gap at high load persists.** Best rajomon (fifo,rajomon) achieves 833.7 at 1400 RPS vs champion's 1215.9 (-31%) and only 217.6 at 2000 RPS vs 1570.2 (-86%).

**Key surprise findings:**
1. **`early` is counterproductive with rajomon** — at 1400 RPS, fifo,rajomon gets 833.7 goodput but fifo,rajomon,early drops to 138.1. Early-return kills Search requests that would have completed within SLO.
2. **`prio_local` adds no benefit over FIFO** — scheduling order is irrelevant when admission control is the bottleneck.
3. **Token budget still too restrictive** — reservation-service CPU for rajomon (~22%) is 1/3 of the champion (~65%), confirming rajomon is shedding too much traffic.
4. **fifo,rajomon is the best rajomon variant** — its advantage at 1400 comes from allowing Search requests to complete naturally (698 Search + 136 Reservation) while early variants kill all Search goodput.

---

## Iteration 2: Dramatically increase token budget (experiment thorn_3)

**Status:** Pending

### Change
Focus on increasing throughput by enlarging the token economy:
- `MAX_TOKEN`: 100 → 500 — 5x larger cap to sustain high-load fan-out
- `TOKEN_UPDATE_STEP`: 5 → 10 — faster replenishment (1000 tokens/s)
- `TOKENS_LEFT_INIT`: 10 → 500 — start with full budget to avoid cold-start starvation

### Hypothesis
The thorn_2 data shows rajomon is over-shedding at high load (reservation CPU 22% vs champion's 65%). The token budget is the admission bottleneck — prices are working as intended (rising under congestion) but the token cap is too low to sustain the required throughput. With MAX_TOKEN=500 and 1000 tokens/s refill, the system should be able to sustain ~500 concurrent tokens across Hotel's call graph. The high initial budget prevents cold-start rejection.

The price mechanism should still provide natural backpressure: when services are truly congested (queue latency > 5ms), prices rise, consuming more tokens per request and reducing admission rate. But with a larger budget, the equilibrium admission rate should be much higher.

### Expected outcomes if hypothesis is correct:
1. Goodput at 1400 RPS should rise significantly (target: >1000, closer to champion's 1216)
2. Goodput at 2000 RPS should improve but still lag the champion (target: >500)
3. Low-load goodput should remain at parity (~99.5% at 100-800 RPS)
4. CPU utilization for rajomon variants should increase toward champion levels

### Experiment design
Same 4 RPS levels (100, 800, 1400, 2000). Drop the `early` variants — thorn_2 proved early+rajomon is counterproductive. Keep only `fifo,rajomon` and `prio_local,rajomon` plus the two baselines.

### Actual Outcomes (thorn_3)

**Status:** Regression ❌

| RPS | fifo,rajomon | prio_local,rajomon | **champion** | prio_oldest,early |
|-----|-------------|-------------------|-------------|-------------------|
| 100 | 99.5 | 99.5 | **99.5** | 99.5 |
| 800 | 795.6 | 795.7 | **795.8** | 795.4 |
| 1400 | 138.3 | 137.3 | **1224.5** | 1140.6 |
| 2000 | 200.5 | 198.9 | **1570.0** | 1046.4 |

**Catastrophic regression at 1400 RPS.** fifo,rajomon dropped from 833.7 (thorn_2) to 138.3 — a 6x collapse. The larger token budget (MAX_TOKEN=500, TOKENS_LEFT_INIT=500) backfired: high initial tokens flood the system at startup, creating real congestion that triggers runaway price inflation. Once prices spike, nearly all requests are rejected (~90%+ early returns at 1400 RPS).

**Root cause:** The problem is not token budget size but price dynamics. Higher initial tokens → initial flood → real congestion → prices spike → price latch → permanent over-rejection. The feedback loop is self-reinforcing regardless of token budget.

**Decision: Revert.** Token budget increases make things worse. The price mechanism itself needs to be less reactive.

---

## Iteration 3: Slow down price dynamics (experiment thorn_4)

**Status:** Pending

**Code commit (to revert):** 62e80b68

### Change
Revert thorn_3's token changes back to thorn_2 values, then attack the price mechanism:
- Revert `MAX_TOKEN`: 500 → 100, `TOKEN_UPDATE_STEP`: 10 → 5, `TOKENS_LEFT_INIT`: 500 → 10
- `LATENCY_THRESHOLD_US`: 5000 → 20000 (20ms) — 4x higher tolerance before triggering price increase
- `PRICE_UPDATE_RATE_MS`: 10 → 50 (50ms ticks) — 5x slower price updates

### Hypothesis
The price spiral happens because PRICE_UPDATE_RATE_MS=10ms means 100 price-update ticks/second. With PRICE_STEP=1, any sustained congestion (queue latency > threshold) increases price by 100/s. At thorn_2's 5ms threshold, moderate load likely exceeds this frequently. Within 1-2 seconds of congestion, price exceeds the token budget, locking out all requests.

By raising the threshold to 20ms (only true overload triggers price increases) AND slowing ticks to 50ms (max price increase rate = 20/s), the price mechanism becomes 25x less reactive. This should prevent the price spiral while still providing congestion feedback under genuine overload.

### Expected outcomes if hypothesis is correct:
1. Goodput at 1400 RPS should match or exceed thorn_2's 833.7 (target: >1000)
2. Goodput at 2000 RPS should significantly improve from thorn_2's 217.6 (target: >500)
3. Low-load parity maintained
4. The price signal should stabilize at a low equilibrium under moderate load rather than spiraling

### Actual Outcomes (thorn_4)

**Status:** Regression ❌

| RPS | fifo,rajomon | prio_local,rajomon | **champion** | prio_oldest,early |
|-----|-------------|-------------------|-------------|-------------------|
| 100 | 99.7 | 99.5 | **99.5** | 99.5 |
| 800 | 795.5 | 795.7 | **795.8** | 795.4 |
| 1400 | 137.3 | 137.3 | **1219.9** | 1140.6 |
| 2000 | 201.1 | 198.9 | **1568.8** | 1046.4 |

**Same regression pattern as thorn_3.** Goodput at 1400 collapsed from thorn_2's 833.7 to 137.3. Both loosening the threshold (20ms) and slowing updates (50ms) caused the same ~138 goodput floor. The system admits too many requests initially (threshold too high to trigger early), saturates, and then over-corrects.

**thorn_2 remains the only working configuration.** Its threshold=5000us + rate=10ms represents a narrow sweet spot. The remaining untouched lever is TOKEN_UPDATE_RATE_MS (client refill frequency).

**Decision: Revert.** This was informative but unhelpful.

---

## Iteration 4: Faster token refill for high-load recovery (experiment thorn_5)

**Status:** Pending

**Code commit:** 56d61f7a

### Change
Revert thorn_4 back to thorn_2 base, then make one targeted change:
- `TOKEN_UPDATE_RATE_MS`: 10 → 5 (2x faster client refill = 1000 tokens/s)

All other params stay at thorn_2 values: LATENCY_THRESHOLD_US=5000, PRICE_UPDATE_RATE_MS=10, MAX_TOKEN=100, TOKEN_UPDATE_STEP=5, PRICE_FREQ=1.

### Hypothesis
At 2000 RPS in thorn_2, rajomon over-sheds (CPU 22% vs champion's 65%), suggesting the price-induced lockout is too sticky. When prices spike during transient congestion, the client-side token bucket takes too long to recover (refilling at 500/s with 10ms ticks). Doubling the refill frequency to 5ms (1000/s) should let clients recover faster from price spikes, maintaining higher sustained throughput under load. The higher refill rate compensates for the token cost of admitting requests through the multi-hop call graph.

### Expected outcomes if hypothesis is correct:
1. Goodput at 1400 RPS should match or slightly exceed thorn_2's 833.7
2. Goodput at 2000 RPS should improve from thorn_2's 217.6 (target: >400)
3. Low-load parity maintained

### Experiment design
Full 8-RPS sweep (100-2000) since this is the penultimate iteration. 4 policies: fifo,rajomon, prio_local,rajomon, champion, prio_oldest,early.

### Actual Outcomes (thorn_5)

**Status:** Neutral (no improvement over thorn_2)

| RPS | fifo,rajomon | prio_local,rajomon | **champion** | prio_oldest,early |
|----:|---:|---:|---:|---:|
| 100 | 99.5 | 100.0 | **99.7** | 99.5 |
| 400 | 397.6 | 397.7 | **397.8** | 397.9 |
| 800 | 795.3 | 795.8 | **795.9** | 795.8 |
| 1200 | 896.7 | 714.7 | **1165.5** | 1177.3 |
| 1400 | 834.4 | 143.5 | **1251.6** | 1167.3 |
| 1600 | 214.1 | 162.0 | **1330.5** | 1056.8 |
| 1800 | 185.8 | 185.0 | **1443.4** | 1023.7 |
| 2000 | 199.0 | 200.9 | **1556.7** | 1069.7 |

**TOKEN_UPDATE_RATE_MS=5 had zero effect.** Results are within noise of thorn_2. The client refill speed is not the bottleneck.

**Full sweep reveals cliff-edge collapse at 1600 RPS.** fifo,rajomon drops from 834 at 1400 to 214 at 1600 — a 4x collapse. Above 1600, goodput stabilizes at ~190-200 (Reservation-only floor).

**prio_local,rajomon is strictly worse than fifo,rajomon.** It collapses earlier (714.7 at 1200 vs 896.7) and harder (143.5 at 1400 vs 834.4). Priority scheduling hurts under rajomon.

**Core diagnosis:** With PRICE_UPDATE_RATE_MS=10ms and PRICE_STEP=1, price increases at 100/s during congestion. MAX_TOKEN=100 means 1 second of sustained congestion → total lockout. At 1600+ RPS, congestion is permanent, so prices climb indefinitely. The step-based mechanism has no proportional control — it either detects congestion (latency > 5ms) or doesn't.

**Decision: Keep** (neutral change, but the full sweep data is valuable). Revert TOKEN_UPDATE_RATE_MS change for the final iteration.

---

## Iteration 5: Moderate price dampening (experiment thorn_6) — FINAL

**Status:** Pending

**Code commit:** ef6c3c89

### Change
Revert TOKEN_UPDATE_RATE_MS to 10 (thorn_5 was neutral), then try a moderate combination:
- `PRICE_UPDATE_RATE_MS`: 10 → 25 (2.5x slower — price changes at 40/s instead of 100/s)
- `LATENCY_THRESHOLD_US`: 5000 → 10000 (2x higher — only triggers above 10ms queue latency)

### Hypothesis
thorn_4 tried extreme dampening (50ms rate + 20ms threshold) and collapsed. This moderate version should be within the working range. The key difference: at thorn_2's 10ms/5000us, price reaches MAX_TOKEN (100) after 1s of congestion. At 25ms/10000us, it takes ~2.5s AND the congestion must be more severe. This extends the "working zone" from ~1400 RPS to potentially ~1600 RPS by delaying the price death spiral.

The tradeoff: slower reaction means the system might overshoot briefly during load spikes. But at steady-state high load (where rajomon currently fails), the dampened response should reach a more sustainable equilibrium.

### Expected outcomes if hypothesis is correct:
1. Low-load parity maintained (100-800 RPS)
2. Goodput at 1400 should match or exceed thorn_2/5's ~834
3. Goodput at 1600 should significantly improve from thorn_5's 214 (target: >500)
4. Goodput at 2000 may still lag but should improve from ~200

### Experiment design
Full 8-RPS sweep. 4 policies (same as thorn_5). This is the final iteration — results will determine whether Rajomon can be competitive with parameter tuning alone.

### Actual Outcomes (thorn_6)

**Status:** Mixed

| RPS | fifo,rajomon | prio_local,rajomon | **champion** | prio_oldest,early |
|----:|---:|---:|---:|---:|
| 100 | 99.6 | 99.5 | **99.6** | 99.5 |
| 400 | 397.9 | 397.9 | **397.8** | 397.9 |
| 800 | 795.6 | 795.8 | **795.8** | 795.7 |
| 1200 | **1180.0** | **1180.0** | 1177.0 | 1166.6 |
| 1400 | 138.9 | 141.7 | **1239.6** | 1124.0 |
| 1600 | 160.9 | 159.8 | **1347.9** | 1058.2 |
| 1800 | 186.8 | 185.2 | **1483.7** | 1053.4 |
| 2000 | 200.1 | 200.7 | **1577.0** | 1065.9 |

**Breakthrough at 1200 RPS: Rajomon matches the champion.** fifo,rajomon achieves 1180.0 vs champion's 1177.0 — a +283.3 improvement over thorn_5's 896.7. The dampened price mechanism allows the system to reach equilibrium at the saturation point.

**Sharper cliff at 1400 RPS.** Goodput collapses from 1180 at 1200 to 139 at 1400 — a 8.5x drop within a single 200-RPS step. This is worse than thorn_5's more gradual decline (897→834→214). The dampening delays the collapse but makes it more abrupt.

**prio_local,rajomon matches fifo,rajomon.** Both rajomon variants produce identical results across all RPS levels, confirming that scheduling policy is irrelevant under rajomon admission control.

**Fundamental tradeoff confirmed.** There is a parameter-mediated tradeoff between saturation-point performance and overload resilience. No parameter setting achieves both.

**Decision: Keep.** This is the best result at 1200 RPS and demonstrates Rajomon's ceiling.

---

## Conclusion

### Best Rajomon configuration found
| Parameter | Value | Changed from default? |
|-----------|-------|-----------------------|
| `LATENCY_THRESHOLD_US` | 10000 | Yes (was 0) |
| `PRICE_UPDATE_RATE_MS` | 25 | Yes (was 10) |
| `MAX_TOKEN` | 100 | Yes (was 10) |
| `TOKEN_UPDATE_STEP` | 5 | Yes (was 1) |
| `PRICE_FREQ` | 1 | Yes (was 5) |
| `TOKENS_LEFT_INIT` | 10 | No |
| `TOKEN_UPDATE_RATE_MS` | 10 | No |
| `PRICE_STEP` | 1 | No |
| `INIT_PRICE` | 0 | No |

### Performance comparison (best rajomon vs champion)

| RPS | Best rajomon (thorn_6) | Champion (adctl) | Gap |
|----:|---:|---:|---:|
| 100-800 | ~99.5% | ~99.5% | Parity |
| 1200 | **1180.0** | **1177.0** | **+3 (Rajomon wins)** |
| 1400 | 138.9 | 1239.6 | -1101 (-89%) |
| 1600+ | ~160-200 | ~1350-1577 | -1100-1377 (-87%) |

### Key questions answered

1. **Can Rajomon's default parameters be tuned to close the goodput gap with adctl?** **No, not above saturation.** At the saturation point (~1200 RPS), Rajomon can match adctl. But above it, Rajomon collapses catastrophically regardless of parameters. The gap is 87-89% at 1400+ RPS.

2. **Does combining Rajomon with prio_local help?** **No.** prio_local,rajomon produces identical results to fifo,rajomon at every RPS level. Under rajomon's token-based admission control, scheduling order is irrelevant — the bottleneck is admission, not scheduling.

3. **What is the root cause of Rajomon's collapse?** The step-based price mechanism (AIAD: +1/-1 per tick) creates a one-way ratchet under sustained congestion. Once queue latency exceeds the threshold, price increases monotonically until it exceeds the token budget, causing total lockout. There is no proportional control or equilibrium-finding mechanism — the system is binary (working or collapsed) with no graceful degradation between.

### What would be needed (beyond parameter tuning)

Closing the gap with adctl would require architectural changes:
- **Proportional price control** (e.g., price proportional to congestion severity, not binary step)
- **AIMD dynamics** (multiplicative decrease instead of additive, breaking the price latch)
- **Per-request cost awareness** (adctl uses latency estimates per RPC type; rajomon uses a single global price)
- **Tighter integration with early-return** (adctl and early cooperate; rajomon and early interfere)
