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
