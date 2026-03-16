# SLATE — Rajomon parameter tuning (mssim)

## Key questions
- Can Rajomon's parameters be tuned to achieve competitive goodput with `prio_local,early` on the mssim workload (trace S_14677443)?
- Why does the current THORN-final parameter set (designed for hotel) produce ~25% flat goodput on mssim at all load levels, and which parameters are responsible?
- Is there a parameter configuration that avoids the binary collapse seen in THORN while also fixing mssim's low-load lockout?

## Constraints
- No algorithm changes — only tuning the 9 constants in `rajomon.rs`
- Maximum 10 iterations
- All experiments use the mssim app with the est01 config as the reference baseline

## Parameters under tuning
| Parameter | THORN-final value | Description |
|-----------|------------------|-------------|
| `PRICE_UPDATE_RATE_MS` | 25 | Background worker tick interval (ms) |
| `LATENCY_THRESHOLD_US` | 10000 | Queue latency threshold for congestion (µs) |
| `PRICE_STEP` | 1 | Additive price step on congestion |
| `INIT_PRICE` | 0 | Initial own_price |
| `PRICE_FREQ` | 1 | Send price in 1/N responses |
| `TOKENS_LEFT_INIT` | 10 | Initial client token budget |
| `TOKEN_UPDATE_RATE_MS` | 10 | Client token refill interval (ms) |
| `TOKEN_UPDATE_STEP` | 5 | Tokens added per refill |
| `MAX_TOKEN` | 100 | Token cap |

## Experiment series: slate_1, slate_2, ... (mssim)

## Phase 0: Baseline (est01 — THORN-final parameters, mssim)

Config: `exp/mssim/in/est01/` — trace S_14677443, SLO=200ms, RPS sweep [800, 1000, 1200, 1400, 1500, 1600, 1800], WarmupSecs=20, DurationSecs=30, MaxInFlight=500. Policies: `fifo,rajomon`, `prio_local,rajomon`, `prio_local,early`.

Code state: THORN 5 final — LATENCY_THRESHOLD_US=10000, PRICE_UPDATE_RATE_MS=25, MAX_TOKEN=100, TOKEN_UPDATE_STEP=5, PRICE_FREQ=1, TOKEN_UPDATE_RATE_MS=10, TOKENS_LEFT_INIT=10, PRICE_STEP=1, INIT_PRICE=0.

### Observed Symptoms (est01)

| RPS | fifo,rajomon | prio_local,rajomon | prio_local,early |
|----:|---:|---:|---:|
| 800 | 199.2 | 199.2 | 802.7 |
| 1000 | 248.2 | 262.9 | 856.6 |
| 1200 | 301.4 | 297.9 | 700.8 |
| 1400 | 350.8 | 348.0 | 642.4 |
| 1500 | 381.0 | 381.4 | 660.1 |
| 1600 | 405.9 | 412.5 | 622.1 |
| 1800 | 464.2 | 467.1 | 649.5 |

**Goodput fraction:**

| RPS | fifo,rajomon | prio_local,rajomon | prio_local,early |
|----:|---:|---:|---:|
| 800 | 0.249 | 0.249 | 1.003 |
| 1000 | 0.248 | 0.263 | 0.857 |
| 1200 | 0.251 | 0.248 | 0.584 |
| 1400 | 0.251 | 0.249 | 0.459 |
| 1500 | 0.254 | 0.254 | 0.440 |
| 1600 | 0.254 | 0.258 | 0.389 |
| 1800 | 0.258 | 0.260 | 0.361 |

**Critical observation:** Both rajomon variants achieve only ~25% of offered load at ALL RPS levels — including 800 RPS where prio_local,early achieves 100% goodput. The system is NOT congested at 800 RPS (prio_local,early proves this), yet rajomon admits only ~25%.

**Root cause diagnosis: False congestion detection from threshold mismatch.**
Token economy: TOKEN_UPDATE_STEP=5 every TOKEN_UPDATE_RATE_MS=10ms = 500 tokens/s refill. For ~25% at 800 RPS (199 req/s): accumulated_price ≈ 500/199 ≈ 2.5 tokens/request. Since prices are integers, this implies prices oscillate around 2–3, meaning some service's own_price is regularly non-zero — i.e., queue latency regularly exceeds LATENCY_THRESHOLD_US=10000µs (10ms) even at light load.

mssim is a trace-driven simulator with realistic processing times. Services in the S_14677443 call graph likely have baseline queue latencies of 10–50ms in the single-threaded tokio runtime — far higher than hotel's sub-5ms baselines. THORN's 10ms threshold (tuned for hotel) fires continuously on mssim at any load, causing permanent price inflation that caps throughput at ~25%.

**Saturation point:** prio_local,early peaks at 1000 RPS (856.6), then drops to 700.8 at 1200 RPS. System saturation is approximately 900–950 RPS.

---

## Iteration 1: Raise congestion threshold for mssim workload (experiment slate_1)

**Status:** Pending

### Change
- `LATENCY_THRESHOLD_US`: 10000 → 50000 (50ms — 5× increase, 25% of the 200ms SLO)

All other parameters remain at THORN-final values.

### Hypothesis
mssim's trace-driven services have inherently higher baseline queue latency than hotel's synthetic services. At 800 RPS (well below saturation), services regularly show queue latency >10ms, causing the LATENCY_THRESHOLD_US=10000 check to fire on nearly every tick. This creates permanent own_price > 0 across services, propagated to the root via max-aggregation, capping admission at ~25%. By raising the threshold to 50ms (still well below the 200ms SLO), normal operating queue latencies stop triggering price increases. Prices should hover near zero at light load, restoring near-100% admission. At saturation (>900 RPS), actual congestion should push queue latencies above 50ms, triggering price increases that provide real admission control signal.

### Expected outcomes if hypothesis is correct:
1. Goodput at 800 RPS should rise from 199 → ~802 (near 100%), matching prio_local,early
2. Goodput at 1000–1200 RPS should improve significantly toward prio_local,early
3. At 1400–1800 RPS, rajomon should maintain non-trivial admission control (not collapse to near-zero)
4. prio_local,rajomon should continue to closely match fifo,rajomon

### Experiment design
Full 7-level sweep matching est01 (800, 1000, 1200, 1400, 1500, 1600, 1800 RPS). Same policies. The key test is whether the 25% lockout at 800 RPS is eliminated. Named slate_1.
