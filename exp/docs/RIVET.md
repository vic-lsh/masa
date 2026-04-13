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

### Actual Outcomes (rivet_4)

**Status:** Mixed

| RPS | rivet_3 (no cap) | rivet_4 (cap=1000) | Delta |
|-----|-------------------|---------------------|-------|
| 300 | 298 | 300 | +2 |
| 500 | 497 | 500 | +3 |
| 700 | 630 | 695 | **+65** |
| 900 | 69 | 83 | +14 |
| 1100 | 30 | 39 | +9 |
| 1300 | 31 | 30 | -1 |

No early returns at 300-700 RPS (good). At 900 RPS: 640/s frontend rejections = 71% — same as rivet_3 despite price_cap=1000.

**Root cause: Accumulated price exceeds max_token.** The admission check uses `accumulated_price = own_price + max(downstream_prices)`. With frontend and reservation each capped at 1000, accumulated = 2000 = max_token → 100% rejection at server. The price_cap prevents individual price runaway but doesn't cap the SUM across the call chain.

For Hotel Search path: frontend → search → {geo, rate, profile, reservation}. Reservation is the bottleneck. Frontend accumulated = own + max(downstream for HandleSearch) = 1000 + 1000 = 2000.

**Key insight: max_token must be >> sum of all price_caps along the critical call path.**

---

## Iteration 5: Raise max_token for accumulated price headroom (experiment rivet_5)

**Status:** Pending

### Change
Config-only — keeping price_cap=1000, raising token budget:
- `max_token`: 2000 → **5000** (accumulated 2000 = 40% rejection max, not 100%)
- `tokens_left_init`: 2000 → **5000**
- `token_update_step`: 2000 → **5000** (keep bucket full)
- All other params unchanged (threshold=25343, step_up=4, price_cap=1000)

### Hypothesis
With max_token=5000, even when both frontend and reservation reach price_cap=1000, accumulated_price=2000 gives rejection prob = 2000/5000 = 40%. This should convert the cliff into graceful degradation:
- At 900 RPS with 40% rejection: ~540 admitted, below saturation (~800), should meet SLO
- At 1300 RPS with 40% rejection: ~780, near saturation, some SLO misses but much better than 30 goodput

### Expected outcomes:
1. 300-700 RPS: unchanged (~300, ~500, ~695 goodput)
2. 900 RPS: major improvement from 83 to ~400-500 
3. 1100-1300 RPS: improvement from 30-39 to 200+

### Actual Outcomes (rivet_5)

**Status:** Regression ❌ (negligible improvement)

| RPS | rivet_4 (max=2000) | rivet_5 (max=5000) | Delta |
|-----|---------------------|---------------------|-------|
| 300 | 300 | 294 | -6 |
| 500 | 500 | 492 | -8 |
| 700 | 695 | 694 | -1 |
| 900 | 83 | 107 | +24 |
| 1100 | 39 | 31 | -8 |
| 1300 | 30 | 29 | -1 |

Raising max_token to 5000 barely helped. At 900 RPS: 618/s frontend rejections = 69%, nearly unchanged from rivet_4's 71%. The max_token increase should limit rejection to 40% (accumulated/max_token = 2000/5000), but actual rejection is 69%.

**Root cause: Accumulated price exceeds 2000.** Hotel Search has a 3+ hop critical path (frontend → search → {geo, rate}; frontend → reservation). With price_cap=1000 per service and ~3 contributing services, accumulated_price ≈ 3000. With max_token=5000, rejection = 3000/5000 = 60%. This matches the observed 69% (with some additional variance from downstream dynamics).

**Fundamental insight:** Rajomon's price propagation creates accumulated prices proportional to call graph depth. For Hotel Search (~3 hops), max_token would need to be ~4x the sum of all price_caps to achieve reasonable (<30%) max rejection. This requires very careful tuning specific to each application's call graph topology.

---

## Summary of RIVET Track

### What we learned

1. **The baseline (hotel_search_v4) had cliff behavior at ~800 RPS** — good at 700, catastrophic at 900+. The original 703 goodput at 700 was somewhat lucky (rivet_3 got 630, rivet_4 got 695 with same params).

2. **Rajomon's unbounded price growth is the primary cliff driver.** Without price_cap, price exceeds max_token in <1 second when overloaded, causing 100% rejection.

3. **Raising latency_threshold backfires.** The low threshold (25ms) provides essential early backpressure that protects downstream services (especially reservation) from warm-up cascades. Raising it to 100-200ms removes this protection and makes things worse (rivet_1, rivet_2).

4. **price_cap helps at moderate load but doesn't solve the cliff.** Adding price_cap=1000 improved 700 RPS from 630→695 but barely dented 900 RPS (69→83). The reason: accumulated price across the call chain exceeds a single service's price_cap.

5. **Accumulated price inflation is the core structural issue.** With ~3 services in the critical path, accumulated_price ≈ 3 × price_cap. Even with max_token 2.5× higher (5000), rejection rate at 900 RPS stays ~69%.

### Parameter tuning levers explored

| Parameter | Baseline | Best | Effect |
|-----------|----------|------|--------|
| latency_threshold_us | 25343 | 25343 | Must keep LOW — protects warm-up |
| price_cap | u64::MAX | 1000 | Small improvement at 700 RPS |
| max_token | 2000 | 5000 | Negligible improvement |

### Abort reason verification
All rejections confirmed 100% rajomon-driven via `abort_reason_timeline.csv`:
- **`RajomonAdmissionRej`**: ~640-960/s at overloaded RPS — server rejects when `ctx.tokens < accumulated_price`
- **`RajomonChildBudgetRej`**: ~280/s at 1100-1300 RPS — admitted requests can't afford downstream child calls (e.g., frontend → reservation). This secondary mechanism wastes work: the request passes initial admission but fails mid-flight.
- No other abort reasons (no SLO-based aborts, no timeouts) — this is purely rajomon.

### Open hypotheses (not tested)
- Reduce price_cap to ~200-300 per service so accumulated stays well under max_token=5000

---

## Iteration 6: Lower price_cap + slower climb (experiment rivet_6)

**Status:** Pending

### Change
Config-only, building on rivet_5 (max_token=5000):
- `price_cap`: 1000 → **500** (accumulated ≈ 3×500 = 1500, rejection = 1500/5000 = 30% max)
- `price_step_up`: 4 → **1** (slowest climb, prevents oscillation)
- Keep: threshold=25343, rate=25ms, max_token=5000, init/refill=5000

### Hypothesis
The rivet_5 cliff was because accumulated_price (3×1000=3000) was too close to max_token (5000), giving ~60% rejection. By halving price_cap to 500, accumulated maxes at ~1500 → 30% max rejection. Combined with price_step_up=1, the system should approach equilibrium smoothly instead of overshooting.

At 900 RPS with 30% rejection: ~630 admitted (below ~800 saturation → good latency → good goodput).
At 1300 RPS: ~910 admitted (still above saturation → latency degrades but not cliff).

### Expected outcomes:
1. 300-700 RPS: unchanged (~300, ~500, ~695)
2. 900 RPS: major improvement from 107 to 400-600
3. 1100-1300 RPS: improvement from 30 to 100-300

### Actual Outcomes (rivet_6)

**Status:** Complete ✅ (best result so far)

| RPS | rivet_5 (cap=1000,step=4) | rivet_6 (cap=500,step=1) | Delta |
|-----|---------------------------|--------------------------|-------|
| 300 | 294 | 298 | +4 |
| 500 | 492 | 500 | +8 |
| 700 | 694 | 697 | +3 |
| 900 | 107 | **328** | **+221** |
| 1100 | 31 | **146** | **+115** |
| 1300 | 29 | **88** | **+59** |

Abort reasons confirmed: `RajomonAdmissionRej` (primary, ~440-590/s at 900 RPS initial spike) + `RajomonChildBudgetRej` (secondary, ~70-140/s). All rajomon-driven.

At 900 RPS: 326/s frontend ER = 36% rejection (was 69% in rivet_5). The lower price_cap and slower climb produce more proportionate rejection.

At 900 RPS: 574 admitted → 328 meet SLO (57% of admitted). The gap is from latency inflation when system is near saturation.

**Remaining issues:** 
1. 900 RPS goodput (328) is still well below system capacity (~700). The admitted requests face high latency because 574 RPS is near saturation.
2. At 1100+ RPS, rejection rate climbs to 59-67% but goodput is still low (146, 88) because admitted traffic still exceeds capacity.

**Decision:** Keep. This is the best config so far.

---

## Iteration 7: Increase max rejection via lower max_token (experiment rivet_7)

**Status:** Pending

### Change
Config-only, modifying rivet_6:
- `max_token`: 5000 → **3000** (accumulated 1500 / 3000 = 50% max rejection)
- `tokens_left_init`: 5000 → **3000**
- `token_update_step`: 5000 → **3000**
- All other params same as rivet_6 (cap=500, step=1, threshold=25343)

### Hypothesis
At 900 RPS in rivet_6, 574 admitted but only 328 meet SLO (57%). The problem is that 574 is too close to saturation (~800). More aggressive rejection would admit fewer requests with better latency, improving goodput.

With max_token=3000: accumulated=1500 → 50% max rejection. At 900 RPS: ~450 admitted, well below saturation → better latency → higher SLO-meeting fraction. Expected: 450 × 0.85+ = 380+.

### Expected outcomes:
1. 300-700 RPS: unchanged (price stays low, rejection minimal)
2. 900 RPS: improvement from 328 to 380+ (fewer admits but better quality)
3. 1100-1300 RPS: improvement (tighter admission keeps latency manageable)

### Actual Outcomes (rivet_7)

**Status:** Mixed

| RPS | rivet_6 (max=5000) | rivet_7 (max=3000) | Delta |
|-----|---------------------|---------------------|-------|
| 300 | 298 | 299 | +1 |
| 500 | 500 | 497 | -3 |
| 700 | 697 | 681 | **-16** |
| 900 | 328 | 301 | **-27** |
| 1100 | 146 | 165 | **+19** |
| 1300 | 88 | 116 | **+28** |

Abort reasons: equal RajomonAdmissionRej (231) and RajomonChildBudgetRej (231). The lower max_token makes child budget rejections more prominent — admitted requests have less remaining budget for downstream calls.

The lower max_token trades moderate-overload goodput for deep-overload goodput. The curve is flatter but 900 RPS is worse. max_token=5000 (rivet_6) remains better for the critical 900 RPS region.

---

## Iteration 8: Midpoint max_token=4000 (experiment rivet_8)

**Status:** Pending

### Change
Config-only, same as rivet_6 except:
- `max_token`: 5000 → **4000**
- `tokens_left_init`: 5000 → **4000**
- `token_update_step`: 5000 → **4000**

### Hypothesis
max_token=5000 was best at 900 (328) but weak at 1100+ (146, 88). max_token=3000 was better at 1100+ (165, 116) but worse at 900 (301). The midpoint (4000) should give accumulated/max_token = 1500/4000 = 37.5% max rejection — slightly more aggressive than 5000's 30%, less than 3000's 50%.

### Expected outcomes:
1. 300-700 RPS: unchanged
2. 900 RPS: ~310-320 (between rivet_6 and rivet_7)
3. 1100-1300 RPS: ~150-160, ~100-110

### Actual Outcomes (rivet_8)

**Status:** Regression ❌

| RPS | rivet_6 (max=5000) | rivet_7 (max=3000) | rivet_8 (max=4000) |
|-----|---------------------|---------------------|---------------------|
| 700 | **697** | 681 | 640 |
| 900 | **328** | 301 | 293 |
| 1100 | 146 | **165** | 132 |
| 1300 | 88 | **116** | 102 |

rivet_8 underperformed at most RPS. Run-to-run variance is significant (~±30 at 900). rivet_6 (max_token=5000) remains the best config for 900 RPS.

---

## Iteration 9: price_freq=1 for 100% price propagation (experiment rivet_9)

**Status:** Pending

### Change
Config-only, modifying rivet_6:
- `price_freq`: 3 → **1** (100% of responses carry price, was 33%)
- All else same as rivet_6

### Hypothesis
With price_freq=3, only 33% of responses update the client's cached price. This stale-price lag means the client continues sending requests at a higher rate than the server wants. With price_freq=1, every response carries the current price, allowing the client to react instantly to price changes. Should reduce oscillation and improve equilibrium.

---

## Iteration 10: max_token=8000 to continue the higher-is-better trend (experiment rivet_10)

**Status:** Pending

### Change
Config-only, modifying rivet_6:
- `max_token`: 5000 → **8000** (accumulated 1500/8000 = 19% max rejection)
- `tokens_left_init` / `token_update_step`: → **8000**
- All else same as rivet_6
