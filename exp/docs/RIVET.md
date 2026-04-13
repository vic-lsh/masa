# RIVET — sched_fifo,ac_rajomon

## Key questions
- Can rajomon achieve graceful degradation instead of cliff-edge rejection on Hotel Search?
- What parameter regime gives rajomon competitive goodput vs tailclipper,abort_slo and pred+ac_pred baselines?
- Is the rajomon algorithm fundamentally limited, or just poorly tuned for this workload?

## Experiment series: hotel_search_v4 (baseline), rivet_1, rivet_2, ... (hotel)

## Observed Symptoms (hotel_search_v4)

**Config:** Search API only, SLO=200ms, RPS=[700, 1000, 1300], warmup=10s, duration=40s

**Rajomon params (sched_fifo):**
- latency_threshold_us: 25343 (25ms)
- price_update_rate_ms: 25
- price_step_up: 4
- init_price: 0, price_freq: 3
- tokens: init=2000, refill=2000/1ms, max=2000
- NO price_cap set (defaults to u64::MAX)

**Goodput (sched_fifo,ac_rajomon):**

| RPS | Goodput | Fraction |
|-----|---------|----------|
| 700 | 703 | 1.00 |
| 1000 | 48 | 0.05 |
| 1300 | 26 | 0.02 |

**Reference baselines (from hotel_search_v3, same app/SLO):**

| RPS | tailclipper,abort_slo | pred,abort_slo,ac_pred |
|-----|-----------------------|------------------------|
| 700 | 695 | 698 |
| 800 | 687 | 728 |
| 900 | 645 | 699 |
| 1000 | 494 | 613 |
| 1100 | 344 | 241 |
| 1200 | 287 | 267 |

**Early returns at 1000 RPS:** 747/s HandleSearch rejections at frontend, 190/s CheckAvailability at reservation. Rajomon is rejecting ~75% of requests at the entry point.

**Root cause analysis:**
1. `latency_threshold_us=25ms` is far below actual queue latencies. Even at 700 RPS, end-to-end p50 is 131ms; queue latency likely exceeds 25ms at moderate load.
2. With no `price_cap`, price grows unbounded. With `price_step_up=4` and excess of 50ms, price increases by ~200 per 25ms tick. Within 250ms, price exceeds `max_token=2000` → 100% rejection.
3. Once all requests are rejected, no new queue latency observations are made. The half-on-read decay mechanism slowly reduces `window_max`, but price is already astronomically high and only decreases by 1 per tick when latency drops below threshold/2. Recovery takes minutes.
4. The cliff is self-reinforcing: high price → rejection → no load → no observations → stale high price persists.

**Initial hypotheses:**
1. Adding a reasonable `price_cap` (e.g., ~75% of `max_token`) is the most critical fix — prevents total shutdown
2. Raising `latency_threshold_us` to match actual queue latency profile would delay price escalation
3. Lowering `price_step_up` would slow price escalation but doesn't prevent eventual cliff without a cap

**Important finding:** The `policy_param.json` uses per-policy nesting (`rajomon.sched_fifo`). The experiment runner extracts the correct sub-key before mounting. The Rust `PolicyParams` struct expects flat `rajomon`. Confirmed via logs that `sched_fifo` params were loaded correctly.

---

## Iteration 1: Add price_cap + raise latency_threshold (experiment rivet_1)

**Status:** Pending

### Change
Config-only (policy_param.json):
- `price_cap`: u64::MAX → **1000** (caps rejection at ~50% since max_token=2000)
- `latency_threshold_us`: 25343 → **100000** (100ms = half of SLO)
- All other params unchanged

### Hypothesis
The cliff behavior is caused by unbounded price growth. Once queue latency exceeds threshold, price escalates by ~(excess_us * 4 / 1000) per 25ms tick. With 75ms excess (100ms queue - 25ms threshold), that's +300/tick. Price exceeds max_token=2000 in ~7 ticks (175ms), causing 100% rejection.

Adding price_cap=1000 means maximum rejection probability is 1000/2000 = 50%. Raising threshold to 100ms delays price escalation onset and reduces the excess that drives price up.

### Expected outcomes if hypothesis is correct:
1. 700 RPS: ~703 goodput (unchanged — queue latency should be well below 100ms)
2. 1000 RPS: significant improvement from 48 to 400+ (50% admission cap means at least 500 admitted, most should meet SLO)
3. 1300 RPS: improvement from 26 to 200+ (some meaningful goodput even in deep overload)

### Experiment design
Same gen_config as baseline (Search only, SLO=200ms, RPS=[700, 1000, 1300]) to enable direct comparison. Rajomon-only.

### Actual Outcomes (rivet_1)

**Status:** Regression ❌

| RPS | Goodput (rivet_1) | Goodput (baseline) | Delta |
|-----|-------------------|-------------------|-------|
| 700 | 0.2 | 703 | −703 |
| 1000 | 0.07 | 48 | −48 |
| 1300 | 0.07 | 26 | −26 |

**Total collapse.** Goodput dropped to near-zero at ALL RPS levels including 700 RPS which was perfect in the baseline.

**Root cause: Reservation warm-up cascade.**
- Reservation service has peak queue latency of 150-190ms at 700 RPS (vs 22-52ms in baseline)
- With threshold=100ms, these peaks trigger price escalation (own_price reaches 298-892)
- High reservation price propagates to frontend via downstream price (150-209)
- Frontend rejects 415/s out of 700/s incoming requests
- With less traffic, reservation's Mongo/Redis connections fail to warm up, keeping queue latency elevated
- Vicious cycle: high price → rejection → cold connections → high queue latency → high price

**Key insight:** Baseline worked at 700 RPS despite threshold=25ms because:
- Reservation queue latency was lower in that run (22-52ms peak)
- Even when price climbed (to 99-132 in baseline), with max_token=2000 the rejection rate was only ~5-7%
- Enough traffic still reached reservation to keep connections warm

**Why raising threshold made things WORSE:** The higher threshold doesn't help because the problem is peak queue latency during warm-up transients, not the steady-state threshold. Once the cascade starts, the system can't recover because price only decreases by 1 per tick (hardcoded).

**Decision:** Revert. The threshold+cap change doesn't address the fundamental warm-up cascade problem.

---

## Iteration 2: Very conservative params + longer warmup (experiment rivet_2)

**Status:** Pending

### Change
Config-only (policy_param.json + gen_config.json):
- `latency_threshold_us`: 25343 → **200000** (200ms = SLO, only trigger on extreme overload)
- `price_step_up`: 4 → **1** (minimum climb rate)
- `price_cap`: add **200** (max ~10% rejection rate with max_token=2000)
- `WarmupSecs`: 10 → **30** (give Mongo/Redis connections time to warm up before measurement)

### Hypothesis
The baseline's success at 700 RPS was partly lucky — reservation's warm-up latency happened to be lower in that run. The combination of very high threshold (200ms, equal to SLO), minimum climb rate (price_step_up=1), and low price_cap (200) should make rajomon nearly transparent at normal load. The longer warmup (30s) gives services time to establish database connections before measurement begins, avoiding the warm-up cascade that killed rivet_1.

### Expected outcomes if hypothesis is correct:
1. 700 RPS: ~700 goodput (near perfect, rajomon essentially inactive)
2. 1000 RPS: some improvement over baseline's 48 (rajomon only mildly active)
3. 1300 RPS: some improvement over baseline's 26 (mild rejection only)

### Actual Outcomes (rivet_2)

**Status:** Regression ❌

| RPS | Goodput (rivet_2) | Goodput (baseline) | Delta |
|-----|-------------------|-------------------|-------|
| 700 | 1.6 | 703 | −701 |
| 1000 | 0.07 | 48 | −48 |
| 1300 | 0.07 | 26 | −26 |

**Still catastrophic collapse**, despite very conservative params and 30s warmup.

**Key finding: The system is fundamentally overloaded, not a rajomon tuning issue.**
- Reservation queue latency: 200-450ms peak (vs 22-52ms in baseline)
- End-to-end p50 latency: 466ms at 700 RPS (vs 131ms in baseline)
- p99: 1028ms. SLO is 200ms, so nearly ALL requests miss SLO.
- Reservation CPU was P90=93.85% in baseline — it's right at saturation.

The baseline's 703 goodput at 700 RPS was likely a lucky run where reservation happened to have lower queue latency. The rajomon params are not the primary driver.

**Hypothesis for next iteration:** Before tuning rajomon, we need to establish a reproducible baseline. Test with lower RPS to find where the system is truly stable, and re-run the original baseline params to check reproducibility.

---

## Iteration 3: Reproducibility check + lower RPS range (experiment rivet_3)

**Status:** Pending

### Change
Config-only:
- Use EXACT baseline params (threshold=25343, price_step_up=4, no price_cap)
- RPS range: [300, 500, 700, 900, 1100, 1300] — wider sweep to find stability boundary
- WarmupSecs: 20 (compromise between 10s and 30s)
- DurationSecs: 40

### Hypothesis
The baseline's 703 goodput at 700 RPS was a lucky run. By adding lower RPS points (300, 500), we'll establish where the system is genuinely stable and identify the true saturation point. If 700 RPS produces ~703 again, the baseline was reproducible and the param changes truly caused regressions. If 700 RPS collapses again, the system is fragile at 700 RPS regardless of params.

### Expected outcomes:
1. 300 RPS: ~300 goodput (well below saturation)
2. 500 RPS: ~500 goodput (moderate load)
3. 700 RPS: either ~703 (reproducible baseline) or <100 (fragile system)
4. 900-1300 RPS: steep decline

### Actual Outcomes (rivet_3)

**Status:** Complete ✅ (reproducibility confirmed)

| RPS | Goodput | Fraction | p50 (ms) | p90 (ms) | Frontend ER |
|-----|---------|----------|----------|----------|-------------|
| 300 | 298 | 99.2% | 116 | 123 | 0 |
| 500 | 497 | 99.3% | 116 | 123 | 0 |
| 700 | 630 | 90.1% | 136 | 167 | 42 (6%) |
| 900 | 69 | 7.6% | 174 | 198 | 647 (72%) |
| 1100 | 30 | 2.8% | 185 | 208 | 862 (78%) |
| 1300 | 31 | 2.4% | 189 | 214 | 1035 (80%) |

**Key findings:**
1. System is stable at 300-500 RPS. Near-perfect goodput, low latency.
2. 700 RPS: 630 goodput (vs baseline's 703). Baseline was slightly lucky but system IS functional here.
3. **Cliff at 900 RPS**: goodput drops from 630 → 69. Price explodes past max_token=2000.
4. System saturation is ~800 RPS. Beyond this, rajomon enters total-shutdown mode.

**Critical insight from rivet_1 vs rivet_3:** The LOW threshold (25ms) actually HELPS at moderate load by providing early backpressure that prevents reservation from being overwhelmed during warm-up. The rivet_1 failure (threshold=100ms) happened because NO early backpressure → reservation flooded → cascading queue buildup. The threshold is doing useful work — the problem is the UNBOUNDED price growth once the cliff is hit.

---

## Iteration 4: Add price_cap to baseline params (experiment rivet_4)

**Status:** Pending

### Change
Config-only — single-variable change from rivet_3 baseline:
- `price_cap`: add **1000** (max 50% rejection with max_token=2000)
- All other params identical to baseline (threshold=25343, step_up=4, etc.)

### Hypothesis
The baseline's low threshold provides necessary early backpressure at moderate load (confirmed by rivet_1 failure). The cliff at 900 RPS is caused by price growing past max_token=2000, creating total shutdown. Adding price_cap=1000 limits maximum rejection to ~50%, which should:
1. Preserve the working behavior at 300-700 RPS (price rarely reaches 1000 there)
2. Convert the 900 RPS cliff into partial rejection — 50% admission means ~450 RPS throughput, below saturation, so admitted requests should meet SLO
3. Allow some goodput at 1100-1300 (vs near-zero currently)

### Expected outcomes if hypothesis is correct:
1. 300-500 RPS: unchanged (~300, ~500 goodput)
2. 700 RPS: unchanged (~630 goodput)
3. 900 RPS: significant improvement from 69 to 300-400 (partial rejection instead of total shutdown)
4. 1100-1300 RPS: improvement from ~30 to 100-300
