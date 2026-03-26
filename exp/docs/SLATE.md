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

**Code commit:** 9fff3118

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

### Actual Outcomes (slate_1)

**Status:** Mixed — `prio_local,rajomon` fixed at 800 RPS, `fifo,rajomon` regressed, cliff at 1000 RPS unchanged

| RPS | fifo,raj (slate_1) | fifo,raj (est01) | prio_local,raj (slate_1) | prio_local,raj (est01) | prio_local,early (slate_1) |
|----:|---:|---:|---:|---:|---:|
| 800 | 160.5 | 199.2 | **798.5** | 199.2 | 802.1 |
| 1000 | 168.4 | 248.2 | 254.8 | 262.9 | 857.3 |
| 1200 | 159.5 | 301.4 | 294.8 | 297.9 | 757.5 |
| 1400 | 158.3 | 350.8 | 355.5 | 348.0 | 681.1 |
| 1500 | 168.9 | 381.0 | 386.3 | 381.4 | 673.8 |
| 1600 | 174.4 | 405.9 | 402.4 | 412.5 | 747.1 |
| 1800 | 164.7 | 464.2 | 452.0 | 467.1 | 667.7 |

**Goodput fraction (slate_1):**

| RPS | fifo,rajomon | prio_local,rajomon | prio_local,early |
|----:|---:|---:|---:|
| 800 | 0.201 | **0.998** | 1.003 |
| 1000 | 0.168 | 0.255 | 0.857 |
| 1200 | 0.133 | 0.246 | 0.631 |
| 1400 | 0.113 | 0.254 | 0.487 |

**Key findings:**

1. **`prio_local,rajomon` at 800 RPS: fully fixed.** 199 → 799 (+600), fraction 0.249 → 0.998. The 50ms threshold eliminates false congestion detection for the priority-scheduled variant. Hypothesis confirmed for prio_local.

2. **`fifo,rajomon` regressed at all RPS.** 199 → 160 at 800 RPS. The 50ms threshold makes fifo *worse*, not better. Likely explanation: FIFO scheduling occasionally allows tasks to accumulate queue times >50ms even at 800 RPS (due to FIFO head-of-line blocking), triggering price increases. At 10ms threshold, prices were consistently small (2–3) and the token economy was in a stable equilibrium. At 50ms threshold, prices spike to higher values during rare FIFO bursts, and the token budget (500 tokens/s at TOKEN_UPDATE_STEP=5) can't sustain throughput at those prices.

3. **Cliff at 1000 RPS is unchanged for both variants.** `prio_local,rajomon` drops from 0.998 (800 RPS) to 0.255 (1000 RPS) in one step. The threshold shift moved the cliff from 800 to 1000 RPS (for prio_local), but the cliff itself is unchanged in character. Above the saturation point (~900 RPS), even rare price increases exhaust the token budget (TOKEN_UPDATE_STEP=5 = 500 tokens/s). At price=1 per hop with a multi-service call graph costing ~6 tokens per request, max throughput = 500/6 ≈ 83 req/s. Immediate collapse to ~25%.

4. **`prio_local,early` stable and slightly improved at several points** (1200: 700→757, 1600: 622→747). The threshold change is irrelevant to this policy, so these are likely noise or minor run-to-run variation.

**Root cause of cliff:** Token refill rate (500 tokens/s at TOKEN_UPDATE_STEP=5) is far too low to sustain any non-zero price. A 6-service call graph where each hop deducts its accumulated_price consumes ~6 tokens per request. Equilibrium admission rate at price=1/hop = 500/6 ≈ 83 req/s — nowhere near the 900 RPS system capacity. The token economy must be enlarged to allow stable equilibrium at moderate prices.

**Decision: Keep slate_1 code (LATENCY_THRESHOLD_US=50000).** The +600 goodput at 800 RPS for prio_local,rajomon is a large win. The fifo regression and the cliff are to be addressed in Iteration 2.

---

## Iteration 2: Enlarge token economy to allow equilibrium at moderate prices (experiment slate_2)

**Status:** Pending

**Code commit:** b4c8e285

### Change
- `TOKEN_UPDATE_STEP`: 5 → 50 (10× increase, token refill rate 500/s → 5000/s)

All other parameters remain at slate_1 values (LATENCY_THRESHOLD_US=50000, PRICE_UPDATE_RATE_MS=25, MAX_TOKEN=100, PRICE_FREQ=1, TOKEN_UPDATE_RATE_MS=10, TOKENS_LEFT_INIT=10, PRICE_STEP=1, INIT_PRICE=0).

### Hypothesis

The cliff at 1000 RPS is caused by the token economy being too tight to sustain any non-zero price. With TOKEN_UPDATE_STEP=5 (500 tokens/s), a 6-hop mssim call graph consuming ~6 tokens/request caps admission at 500/6 ≈ 83 req/s once any service sees price=1. This means there is no stable equilibrium between 0 (unlimited) and 83 req/s — the system snaps directly from full admission to near-lockout.

By raising TOKEN_UPDATE_STEP to 50 (5000 tokens/s), equilibrium admission at price=1/hop becomes 5000/6 ≈ 833 req/s, just below the system capacity of ~900 RPS. This creates a useful oscillating equilibrium: at price=1, admission=833 < capacity, so queue drains and price returns to 0; at price=0, full load admitted → slight overload → price rises to 1. The system oscillates around 833–900 req/s admission rather than collapsing to 83.

For offered loads above 900 RPS (1000–1800 RPS), the system can shed the excess (~100–900 req/s) proportionally via price increases, rather than the current binary collapse. At price=2/hop: 5000/12 = 416 req/s admission (rough proportional shedding).

The larger token throughput should also fix the `fifo,rajomon` regression: even if fifo's FIFO scheduling occasionally causes queue bursts that push prices to 2–3, the token economy can sustain 5000/12–18 = 278–416 req/s, which is substantially better than the current 83 req/s floor.

### Expected outcomes if hypothesis is correct:
1. `prio_local,rajomon` at 800 RPS: maintains ~100% (ceiling held from slate_1)
2. `prio_local,rajomon` at 1000–1200 RPS: significant improvement from 25% toward 50%+ — price equilibrium should allow ~700–800 goodput at 1000 RPS
3. `fifo,rajomon` at 800 RPS: recovers from 160 toward 200+ (token floor raised)
4. Both variants: no longer flat at ~25% across all RPS — should show differentiation by load level

### Experiment design
Full 7-level sweep. Same policies. Named slate_2.

### Actual Outcomes (slate_2)

**Status:** Revert ❌ — TOKEN_UPDATE_STEP axis is irrelevant

| RPS | fifo,raj (slate_2) | fifo,raj (slate_1) | prio_local,raj (slate_2) | prio_local,raj (slate_1) | prio_local,early (slate_2) |
|----:|---:|---:|---:|---:|---:|
| 800 | 176.2 | 160.5 | **795.4** | 798.5 | 806.2 |
| 1000 | 156.0 | 168.4 | 255.5 | 254.8 | 818.5 |
| 1200 | 172.5 | 159.5 | 303.4 | 294.8 | 734.2 |
| 1400 | 169.0 | 158.3 | 348.2 | 355.5 | 722.2 |
| 1600 | 163.2 | 174.4 | 400.4 | 402.4 | 869.0 |
| 1800 | 161.8 | 164.7 | 461.4 | 452.0 | 821.0 |

**Key findings:**

1. **TOKEN_UPDATE_STEP=50 had zero effect.** prio_local,rajomon at every RPS level is within noise of slate_1. The ~25% flat floor at 1000+ RPS is completely unchanged.
2. **Root cause of invariance:** The AIAD price mechanism self-calibrates. Higher TOKEN_UPDATE_STEP → more initial admission → more congestion → prices rise proportionally → same admission rate at equilibrium. The equilibrium throughput is set by the multiplicative admission structure, not by the token economy.
3. **The 25% floor is multiplicative across N hops.** If N services each independently check admission with duty cycle D, total throughput = D^N. For 25%: N=2 with D=0.5, or N=4 with D≈0.71. Changing TOKEN_UPDATE_STEP only changes the price level at equilibrium, not the throughput.
4. **fifo,rajomon unchanged** — remains at ~160-175 across all RPS. Not improved by larger token economy.

**Decision: Revert TOKEN_UPDATE_STEP to 5 (neutral, cleaner code). Address the multi-hop multiplication in Iteration 3 by raising threshold to 150ms (75% of SLO) to reduce N — the number of services actively running admission control.**

Reverted in commit a96c4bcb (TOKEN_UPDATE_STEP: 50 → 5, LATENCY_THRESHOLD_US: 50000 → 150000 applied together).

---

## Iteration 3: Raise threshold to 150ms to reduce multi-hop admission multiplication (experiment slate_3)

**Status:** Pending

**Code commit:** a96c4bcb

### Change
- `TOKEN_UPDATE_STEP`: 50 → 5 (revert slate_2, neutral)
- `LATENCY_THRESHOLD_US`: 50000 → 150000 (150ms — 3× increase from slate_1, 75% of 200ms SLO)

Current params: LATENCY_THRESHOLD_US=150000, PRICE_UPDATE_RATE_MS=25, MAX_TOKEN=100, TOKEN_UPDATE_STEP=5, PRICE_FREQ=1, TOKEN_UPDATE_RATE_MS=10, TOKENS_LEFT_INIT=10, PRICE_STEP=1, INIT_PRICE=0.

### Hypothesis
The ~25% floor at 1000+ RPS is caused by multiplicative independent admission control at N services in the call graph (total throughput = D^N per hop). At threshold=50ms, multiple services trigger price increases simultaneously (even at modest overload), producing N≥2 active admission control points. By raising to 150ms (75% of SLO), only services with severe congestion (queue > 150ms) trigger price increases. At slight overload (1000 RPS, ~11% above capacity), fewer services should reach 150ms queue latency, reducing N toward 1. With N=1, throughput = D rather than D^N, potentially improving from 25% to 50%+ at 1000 RPS.

At deep overload (1400–1800 RPS), queue latencies may still exceed 150ms at multiple services, maintaining some admission control. But even if N=2 with D=0.5 at deep overload: 25% throughput = same as current. No regression expected.

### Expected outcomes if hypothesis is correct:
1. `prio_local,rajomon` at 1000–1200 RPS: significant improvement from 25% (target: 40%+ at 1000 RPS, approaching 400 goodput)
2. `prio_local,rajomon` at 800 RPS: maintains ~100% (threshold well above light-load queue latencies)
3. `fifo,rajomon` may also improve if fewer services trigger simultaneously
4. At 1400–1800 RPS: outcome uncertain — may maintain ~25% or slightly improve depending on how many services exceed 150ms

### Experiment design
Full 7-level sweep. Same policies. Named slate_3.
