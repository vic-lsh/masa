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
Same gen_config as baseline (Search only, SLO=200ms, RPS=[700, 1000, 1300]) to enable direct comparison. Added `sched_tailclipper,abort_slo` as a reference baseline.
