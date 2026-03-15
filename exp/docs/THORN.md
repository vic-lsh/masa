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

**Status:** Pending

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
