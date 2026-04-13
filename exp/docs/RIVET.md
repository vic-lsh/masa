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
