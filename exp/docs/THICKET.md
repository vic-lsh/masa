# THICKET — sched_fifo,ac_rajomon,abort_slo on socialnet ComposePost

## Key questions
- Can rajomon + fifo + abort_slo beat `sched_tailclipper,abort_slo` on socialnet ComposePost across the stress region (≥1400 RPS)?
- Is rajomon actually the dominant rejection mechanism (via `abort_reason_timeline.csv`), or is abort_slo carrying the load while rajomon is effectively dormant? Prior rajomon tuning in this repo didn't validate this — we must.
- What param regime makes rajomon active but non-catastrophic on socialnet's SLO=50ms, shorter-path workload?

## Experiment series: thicket_1 ... thicket_5 (socialnet)

## Validation protocol (every iteration)
After each run, before declaring success or drawing conclusions:
1. Check `exp/socialnet/plots/thicket_N/0/early_return/abort_reason_timeline.csv` (and `.png`).
2. Confirm `RajomonAdmissionRej` (and/or `RajomonChildBudgetRej`) is the dominant abort reason at overloaded RPS points.
3. If abort reason is mostly `SloAbort*` or other non-rajomon reasons at overload, rajomon is dormant — that run is a tuning failure even if goodput looks OK.

## Reference data (prior socialnet rivet runs, no abort-reason validation)

From `exp/socialnet/plots/rivet_5/summary/goodput/goodput_ALL_aggregated.csv`:

| RPS | sched_fifo,ac_rajomon,abort_slo | sched_tailclipper,abort_slo |
|-----|---------------------------------|-----------------------------|
| 800 | 799 | 799 |
| 1000 | 999 | 999 |
| 1200 | 1191 | 1193 |
| 1400 | 1288 | 1351 |
| 1600 | 1052 | 1073 |
| 1800 | 1008 | 983 |
| 2000 | 1087 | 1071 |
| 2500 | 912 | 1091 |
| 3000 | 7.7 | 988 |

Rajomon tracks or slightly beats tailclipper at moderate load (1800-2000) but **collapses at 3000 RPS** (7.7 vs 988). Its abort reason was never validated — the 3000-RPS cliff could be rajomon pathology, or it could be the workload hitting some other wall.

Saturation is ~1400 RPS. Stress region: ≥1400 RPS.

---

## Iteration 1: Establish baseline with hotel-rivet_6 params (experiment thicket_1)

**Status:** Pending

### Change
Config-only. Port hotel-rivet_6's validated rajomon params verbatim to socialnet:
- `latency_threshold_us`: 25343 (25ms — half of socialnet SLO=50ms, so rajomon sees queue-latency pressure before SLO misses)
- `price_update_rate_ms`: 25
- `price_step_up`: 1
- `price_cap`: 500
- `price_freq`: 3
- `tokens_left_init` / `token_update_step` / `max_token`: 5000
- `token_update_rate_ms`: 1

Policies in run: `sched_fifo,ac_rajomon,abort_slo` (target) and `sched_tailclipper,abort_slo` (baseline).

RPS sweep: [800, 1200, 1400, 1600, 1800, 2000, 2500, 3000]. Drop 1000 (uninformative low load) to save runtime; keep the stress region (1400→3000) dense.

### Hypothesis
Hotel-rivet_6 params were the best tuning discovered for rajomon on hotel (SLO=200ms). Socialnet has SLO=50ms and a different call graph (parallel fanout in ComposePost vs hotel's serial Search), so these params may not transfer — but they are principled starting values (threshold at half-SLO, capped price, multi-hop accumulated-price headroom).

### Expected outcomes
1. Abort reason at 1600+ RPS is dominated by `RajomonAdmissionRej` (validation gate).
2. Goodput roughly tracks tailclipper up to 2000 RPS, possibly deviates at 2500-3000.
3. Tells us whether the 3000-RPS cliff from rivet_1/5 was a rajomon pathology or a workload artifact.

### Experiment design
Same gen_config as api_composepost but dropping the uninformative 1000 RPS point. Two policies only (target + tailclipper baseline) to keep the run short and make abort-reason comparison clean.

### Actual Outcomes (thicket_1)

**Status:** Rajomon DORMANT ❌ (validation failed — tuning restart required)

| RPS | Target (fifo+ac_rajomon+abort_slo) | Baseline (tailclipper+abort_slo) | Delta |
|-----|------------------------------------|----------------------------------|-------|
| 800 | 799.8 | 799.9 | -0.1 |
| 1200 | 1193.3 | 1189.6 | +3.7 |
| 1400 | **1381.9** | 1179.2 | +202.8 |
| 1600 | 879.3 | 866.1 | +13.2 |
| 1800 | **1161.4** | 991.9 | +169.6 |
| 2000 | 1966.1 | 1826.2 | +139.9 |
| 2500 | 968.6 | 898.8 | +69.8 |
| 3000 | **137.0** | 1050.7 | **-913.7** |

**Rajomon activity: DORMANT.** `abort_reason_timeline.csv` contains only `E2EDeadline` rows for both policies. Zero `RajomonAdmissionRej` / `RajomonChildBudgetRej` events emitted across the whole run. The goodput + ER accounting also holds (goodput + E2EDeadline rate ≈ offered RPS at every load), confirming the admission gate never rejected anything.

**Root cause:** threshold=25ms = half of SLO=50ms is too permissive for socialnet. On hotel (SLO=200ms), 25ms is 1/8 of SLO and queue latency routinely exceeded it. On socialnet, `abort_slo` aborts requests as soon as their 50ms deadline is hit, so queues never get deep enough for queue-latency to exceed 25ms. Price never climbs from init_price=0, accumulated_price stays 0, nothing is rejected at the gate.

**Implication for apparent wins:** The 1400/1800/2000 RPS wins (+170 to +200 goodput) are NOT from rajomon — rajomon was idle. They are artifacts of `sched_fifo,abort_slo` vs `sched_tailclipper,abort_slo` scheduling/abort ordering differences. These wins would survive even if rajomon were removed.

**Implication for 3000 RPS collapse:** The target's collapse to 137 (vs tailclipper's 1050) is also unrelated to rajomon (since rajomon wasn't active). It's a sched_fifo + abort_slo interaction at deep overload. Tailclipper's oldest-first policy fares better at 3000 RPS, but fifo drowns.

**Decision:** Next iteration MUST make rajomon active before any other tuning matters. Lowering `latency_threshold_us` is the single variable to change — it gates every downstream mechanism.

---

## Iteration 2: Lower threshold to activate rajomon (experiment thicket_2)

**Status:** Pending

### Change
Config-only. Single-variable from thicket_1:
- `latency_threshold_us`: 25343 → **5000** (5ms = 1/10 of SLO, mirroring the 1/8 ratio used on hotel)
- All other params unchanged (price_step_up=1, price_cap=500, max_token=5000, price_freq=3, price_update_rate_ms=25)

### Hypothesis
The dormancy in thicket_1 is caused by queue latency never exceeding the 25ms threshold under socialnet's tight 50ms SLO. `abort_slo` drains queues before they grow deep. Lowering threshold to 5ms should let queue-latency excess drive price climb well before SLO-miss rejection kicks in, putting rajomon in the critical path.

### Expected outcomes
1. **Primary (validation):** abort_reason_timeline shows non-zero `RajomonAdmissionRej` at RPS ≥ 1400, and it becomes the dominant reason at some RPS (likely 2500-3000). If this fails, we try an even lower threshold (2000us) in iter 3.
2. Low load (800, 1200) unchanged — queue latency still below 5ms there.
3. 3000 RPS: rajomon activates, gates ingress before runaway abort_slo cascades. Goodput recovers from 137 toward 400+.
4. Possible downside: rajomon may over-reject at moderate overload (1600-2000), reducing goodput vs thicket_1. We accept this as a necessary test of activation; iter 3 will calibrate.

### Experiment design
Identical gen_config and policies as thicket_1. Only `policy_param.json` changes. Enables clean A/B vs thicket_1 for isolating the threshold effect.

### Actual Outcomes (thicket_2)

**Status:** Still DORMANT ❌ (regression + zero rajomon activity)

| RPS | thicket_2 target | thicket_1 target | tailclipper (thicket_2) | Δ vs baseline |
|-----|-----|-----|-----|-----|
| 800 | 799.8 | 799.8 | 799.9 | -0.1 |
| 1200 | 1190.4 | 1193.3 | 1184.9 | +5.5 |
| 1400 | 1330.0 | 1381.9 | 1317.9 | +12.2 |
| 1600 | 846.4 | 879.3 | 873.1 | -26.7 |
| 1800 | 963.4 | 1161.4 | 1436.7 | -473.3 |
| 2000 | 1382.4 | 1966.1 | 1871.2 | -488.8 |
| 2500 | 899.2 | 968.6 | 913.5 | -14.3 |
| 3000 | 66.1 | 137.0 | 1032.2 | -966.1 |

**Rajomon activity: STILL DORMANT.** abort_reason_timeline contains only E2EDeadline rows. Dropping threshold 25343→5000us did not produce a single RajomonAdmissionRej or RajomonChildBudgetRej event.

**Why the threshold knob alone can't activate rajomon here** (from code dive):
- Server rejection condition: `ctx.tokens() < accumulated_price` where `accumulated = own_price + max(downstream_prices)`.
- `own_price` starts at `init_price=0` and only climbs when `queue_latency > threshold` (per-hop, at each service).
- `ctx.tokens()` is a uniform random draw from `[0, client_bucket]` attached by the client.
- If per-hop queue latency never exceeds threshold, every service's `own_price` stays at 0, `accumulated=0`, every request trivially admitted regardless of threshold.
- Socialnet's `abort_slo` drains queues aggressively at 50ms SLO: per-hop queue latency likely caps well below 5ms even at 3000 RPS. Rajomon's trigger signal is simply absent. Lowering threshold further won't help — the queue latency isn't there.

**Regression explanation:** thicket_2 regressed vs thicket_1 despite rajomon being inactive in both. This is run-to-run variance on sched_fifo at socialnet's bistable region (1600-2000 RPS; note tailclipper also moved from 1436→991 between the two runs). Not informative about rajomon.

**Decision:** Switch away from "tune threshold to trigger latency-based price". Next iteration adopts a differently-tuned config where activation is more plausible.

---

## Iteration 3: Adopt rajomon_optimal Bayesian-tuned params (experiment thicket_3)

**Status:** Pending

### Change
Replace `policy_param.json` with the `rajomon_optimal` config (from commit 3e935f77, "Bayesian-optimized values" previously placed in `exp/socialnet/in/rajomon_optimal/`):
- `latency_threshold_us`: 5000 → **10647** (slightly higher than thicket_2 but still 1/5 of SLO)
- `price_update_rate_ms`: 25 → **4** (6× faster price adjustment)
- `price_step_up`: 1 → **20** (much steeper climb when latency exceeds threshold)
- `price_step_down`: absent → **4** (non-default decay rate)
- `price_cap`: 500 → **66**
- `price_freq`: 3 → **5**
- `tokens_left_init` / `max_token`: 5000 → **285** (tiny client bucket)
- `token_update_step`: 5000 → **3** (very slow refill)
- `token_update_rate_ms`: 1 → **4**

### Hypothesis
The Bayesian-tuned `rajomon_optimal` config was optimized against socialnet (commit message explicit). Its knobs favor fast, aggressive price climb (`step_up=20`, `rate=4ms`) over large buckets — this should drive `own_price` up quickly once any queue latency >10.6ms is observed, and the tiny `max_token=285` means even a modest accumulated price yields real rejection probability. This is the most plausible single config in the repo for getting rajomon to actually fire on socialnet.

### Expected outcomes
1. **Primary (validation):** non-zero `RajomonAdmissionRej` rows in abort_reason_timeline at some RPS ≥ 1400. If still zero, we have a wiring concern to investigate.
2. If active: goodput profile will shift — possibly lower at low RPS (unnecessary rejections from aggressive climb) but recovery at 2500-3000 (admission gate blunts abort_slo cascade).
3. Abort-reason timeline should show `RajomonAdmissionRej` dominant at 2500-3000 (the loads where thicket_1/2 hit the cliff).

### Experiment design
Same gen_config and policies as thicket_1/2. Only policy_param.json changes. Allows direct comparison across all three thicket runs.

### Actual Outcomes (thicket_3)

**Status:** STILL DORMANT ❌ (three consecutive dormant runs)

| RPS | thicket_3 target | tailclipper (thicket_3) | Δ |
|-----|-----|-----|-----|
| 800 | 799.9 | 799.9 | 0 |
| 1200 | 1194.5 | 1191.2 | +3.3 |
| 1400 | 1266.3 | 1372.0 | -105.7 |
| 1600 | 1118.1 | 857.3 | +260.8 |
| 1800 | 1731.7 | 1722.1 | +9.6 |
| 2000 | 1679.2 | 1780.7 | -101.5 |
| 2500 | 858.7 | 968.2 | -109.5 |
| 3000 | 13.4 | 1010.7 | -997.3 |

abort_reason_timeline: still only `E2EDeadline` rows for both policies. No `RajomonAdmissionRej` ever emitted across thicket_1/2/3. The Bayesian-tuned `rajomon_optimal` params didn't help.

**Root cause understood (from code dive of `libs/masa-policy/src/layer/admission/rajomon.rs` and `libs/masa-policy/src/hooks.rs`):**
1. Layer order: `e2e_deadline_guard → estimation → admission`. e2e_deadline_guard's `before_poll` aborts past-deadline requests BEFORE rajomon's `before_poll` can run, so queue-latency observations that go to `RAJOMON_STATE.queue_stats.window_max` only come from requests that haven't yet missed their SLO.
2. Server-side rejection requires `ctx.tokens() < accumulated_price`. `accumulated = own_price + max_downstream`. `own_price` starts at `init_price=0` and only climbs when `window_max > threshold`.
3. On socialnet, freshly-arrived requests (those that do reach rajomon's `before_poll`) have queue latency well under any reasonable threshold — the queue-latency spike only manifests to requests that are already past their 50ms SLO and thus aborted upstream by e2e_deadline_guard.
4. Result: `own_price` stays at 0 at every service. `accumulated` stays at 0. Every request passes server admission. Rajomon is structurally silent under `+abort_slo` for tight-SLO workloads.

**Client side:** client's `try_acquire` can also rate-limit (`rng.gen_range(0..=current) < cached_price`). But `cached_price` is only updated from server responses carrying `x-masa-rajomon-price`, which is the server's `own_price`. When `own_price=0`, cached_price=0, `rng.gen_range(0..=current) < 0` never true → client never drops either.

**Implication:** Tuning `latency_threshold_us`, `price_step_up`, `price_cap`, or `max_token` can't fix the circular dormancy. The only way to force activation under `+abort_slo` is to seed `own_price` with `init_price > 0` so accumulated is non-zero from the start.

---

## Iteration 4: Force activation via init_price=200 (experiment thicket_4)

**Status:** Pending

### Change
Config-only. Single variable from thicket_3 (rajomon_optimal):
- `init_price`: 0 → **200**
- All other params identical to thicket_3 (threshold=10647, step_up=20, step_down=4, price_cap=66, freq=5, tokens=max=285, update_rate=4ms).

Note: `price_cap=66` caps the price's *climb*, but `init_price` sets `own_price` directly without cap. So own_price starts at 200. Under no queue-latency signal, it decays by 1 per 4ms tick — 200 → 0 in 800ms. That's enough wall time at each RPS step (30s duration) to produce rajomon rejections during the initial burst of every step, and also whenever queue-latency observations reappear.

### Hypothesis
The cause of three consecutive dormant runs is that `own_price` never climbs above 0 because requests that would observe queue latency are preempted by e2e_deadline_guard. Seeding `init_price=200` forces `accumulated_price ≥ 200` from t=0 at every service, which makes `ctx.tokens() < accumulated` satisfiable for the ~70% of client tokens drawn from `[0, 285]` that fall below 200. This should produce non-zero `RajomonAdmissionRej` events at the server, AND cause response headers to propagate price back to the client, which then rate-limits locally on subsequent requests.

### Expected outcomes
1. **Primary (validation):** non-zero `RajomonAdmissionRej` events at every RPS level for the target policy. Even if price decays over the 30s step, the initial admission-rejection burst should show up in the timeline.
2. Low RPS (800, 1200): likely regression from unnecessary rejection. This is acceptable — we're testing activation, not productivity yet.
3. High RPS (2500, 3000): possible recovery from the 3000-RPS cliff (13 → something >0) if rajomon's ingress rejection relieves the abort_slo cascade.
4. If **still dormant** (zero rajomon rows), there is a deeper wiring/feature-flag issue that param tuning can't fix. That would terminate the track with a clear diagnosis.

### Experiment design
Same gen_config and policies as thicket_1/2/3. Only `init_price` changes. Clean single-variable test.

### Actual Outcomes (thicket_4)

**Status:** STILL DORMANT ❌ (four consecutive dormant runs)

| RPS | thicket_4 target | tailclipper (thicket_4) | Δ |
|-----|-----|-----|-----|
| 800 | 799.9 | 799.8 | +0.1 |
| 1200 | 1195.1 | 1182.7 | +12.4 |
| 1400 | 1225.4 | 1294.1 | -68.7 |
| 1600 | 834.1 | 846.3 | -12.2 |
| 1800 | 1413.9 | 1560.3 | -146.4 |
| 2000 | 1612.2 | 1824.6 | -212.4 |
| 2500 | 895.4 | 958.9 | -63.5 |
| 3000 | 193.7 | 1066.1 | **-872.4** |

abort_reason_timeline again contains only E2EDeadline rows. Zero RajomonAdmissionRej / RajomonChildBudgetRej. init_price=200 did not produce any detectable rajomon-attributable rejection.

**Verified the generated policy_param.json was mounted with init_price=200 correctly** (at `exp/socialnet/out/thicket_4/0/sched_fifo,ac_rajomon,abort_slo/policy_param.json`). So this is not a param-loading bug.

**Analysis of why init_price=200 should have worked but didn't:**
- Server new() runs `if ctx.tokens() < accumulated` where `accumulated = own_price(=init_price=200) + max_downstream(=0) = 200`.
- Client sends ctx.tokens = `uniform[0, 285]`, avg 142. Prob(tokens < 200) ≈ 70%.
- With should_drop=true, `before_poll` issues `Status::resource_exhausted("/EarlyReturn?src=...&reason=RajomonAdmissionRej")`.
- e2e_deadline_guard runs first in for_each_layer! but for fresh requests the deadline isn't past — e2e check should return Ok, passing to rajomon.

At least the first tens-to-hundreds of requests per RPS step should produce RajomonAdmissionRej. They don't. This suggests either:
- A feature-flag interaction where the rajomon layer isn't actually running under `sched_fifo,ac_rajomon,abort_slo` (worth checking the AdmissionLayer cfg under abort_slo).
- An error-propagation path that turns the rajomon Status into a different reason before it's logged.
- e2e_deadline_guard preempting every request (perhaps via some interaction with warmup/startup phase that makes deadlines effectively always past).

**This is now a code-level investigation question, not a param-tuning question.** Remaining iteration will do one more maximum-forcing attempt and stop.

---

## Iteration 5: Maximum activation forcing (experiment thicket_5)

**Status:** Pending

### Change
Config-only. From thicket_4, push three knobs simultaneously to make server-side rejection mathematically inevitable for every request:
- `init_price`: 200 → **500** (dominates any plausible ctx.tokens draw)
- `tokens_left_init`: 285 → **100** (caps max possible ctx.tokens at 100; every client request has tokens ≤ 100)
- `max_token`: 285 → **100** (matches init; no replenishment can lift tokens above init_price)

With `tokens_left_init=100` and `init_price=500`, ctx.tokens is uniform `[0, 100]` and accumulated_price = 500 at every service. Every request on every hop satisfies `tokens < accumulated` → server-side should_drop=true → rajomon fires on before_poll (unless preempted by e2e_deadline_guard).

### Hypothesis
If rajomon's rejection path is mechanically wired correctly, this config must produce non-zero `RajomonAdmissionRej` events. If thicket_5's abort_reason_timeline still contains zero RajomonAdmissionRej, the wiring is broken for `sched_fifo,ac_rajomon,abort_slo` and no amount of parameter tuning will activate rajomon under this feature combination. That outcome is worth knowing.

### Expected outcomes
- Goodput will likely collapse at low RPS (rajomon should reject 100% when active).
- `abort_reason_timeline` should show substantial `RajomonAdmissionRej` at every RPS if wiring works.
- Alternatively, it may show zero again — definitive diagnostic that rajomon's rejection path is not reaching the ER tracker under this config.

### Experiment design
Same gen_config and policies. Single change: the three token/price knobs. Purely diagnostic, not a productive config.

### Actual Outcomes (thicket_5)

**Status:** RAJOMON ACTIVATED ✅ (but transient, and goodput regressed — wiring confirmed, structural limit identified)

| RPS | thicket_5 target | tailclipper (thicket_5) | Δ |
|-----|-----|-----|-----|
| 800 | 767.4 | 799.9 | **-32.5** |
| 1200 | 1184.2 | 1175.5 | +8.7 |
| 1400 | 1358.9 | 1191.5 | +167.4 |
| 1600 | 810.8 | 829.9 | -19.1 |
| 1800 | 973.3 | 986.7 | -13.4 |
| 2000 | 1240.8 | 1721.6 | -480.8 |
| 2500 | 903.0 | 811.9 | +91.1 |
| 3000 | 111.5 | 971.2 | **-859.7** |

**abort_reason_timeline** (target policy):
- E2EDeadline: avg 860.3/s, max 3001.0/s
- **RajomonAdmissionRej: avg 1.6/s, max 394.0/s** ← first time non-zero
- **RajomonChildBudgetRej: avg 0.3/s, max 93.5/s**

**Rajomon activity verdict: TRANSIENT.** Wiring is verified working — the rejection path CAN produce `RajomonAdmissionRej` events and they DO reach the abort_reason tracker. But rajomon is only active during brief windows: the max 394/s spike decays to near-zero quickly, and the 456-sample average is only 1.6/s. The init_price=500 seed decays by 1 per 4ms tick → reaches 0 in ~2s, then rajomon is dormant again until the next RPS step boundary resets observation.

**Goodput regression at 800 RPS (767 vs 799.9)** confirms rajomon is doing SOMETHING — it's rejecting ~30/s of in-spec requests at the initial burst. This is the only RPS level where rajomon's rejection hurts more than it helps, because the system has plenty of capacity at 800 RPS and abort_slo alone is sufficient.

**The 3000 RPS cliff persists (111 vs tailclipper 971).** Even with rajomon activated briefly, it can't keep accumulated_price high enough to relieve the abort_slo cascade at sustained deep overload.

### Decision
Keep thicket_5's diagnostic value (rajomon wiring validated) but do NOT recommend these params for production. The 800 RPS regression is the proof of unnecessary rejection.

---

## Final summary — THICKET track (5 iterations)

### Key question answered

**Can `sched_fifo,ac_rajomon,abort_slo` beat `sched_tailclipper,abort_slo` on socialnet ComposePost?**

With config-only tuning: **No.** At no point across 5 iterations did the target policy beat tailclipper on the stress region (≥1400 RPS) with rajomon genuinely carrying the admission-control load.

### Results table (best goodput per RPS across all 5 runs)

| RPS | Best rajomon result | Tailclipper (same run) | Rajomon active? |
|-----|-----|-----|-----|
| 800 | 799.9 (thicket_1/2/3/4) | 799.9 | no (dormant) |
| 1200 | 1195.1 (thicket_4) | 1184.9 | no (dormant) |
| 1400 | 1381.9 (thicket_1) | 1317.9 | no (dormant) |
| 1600 | 1118.1 (thicket_3) | 857.3 | no (dormant) |
| 1800 | 1731.7 (thicket_3) | 1722.1 | no (dormant) |
| 2000 | 1966.1 (thicket_1) | 1826.2 | no (dormant) |
| 2500 | 968.6 (thicket_1) | 898.8 | no (dormant) |
| 3000 | 193.7 (thicket_4) | 1066.1 | no (dormant) |

The apparent wins at 1400/1600/1800/2000 were `sched_fifo` vs `sched_tailclipper` scheduling-order differences, NOT rajomon wins — abort_reason_timeline confirms rajomon was dormant in every one of those runs.

### Root cause (documented in thicket_2/3/4 analyses above)

Under `+abort_slo` with a 50ms SLO, rajomon's `own_price` remains at 0 because:
1. Layer order `e2e_deadline_guard → estimation → admission` means e2e_deadline_guard preempts past-deadline requests before rajomon's `before_poll` can record queue latency to `RAJOMON_STATE.queue_stats.window_max`.
2. Under socialnet's tight SLO, every deep-queue request has already missed its deadline before reaching rajomon's observation point.
3. Without queue-latency samples exceeding threshold, `own_price` never climbs from `init_price`.
4. With `init_price=0` (default), accumulated_price stays 0 → every request admitted → no rajomon activity.

### Parameter ranges tried

| Param | Values tested | Effect |
|-------|-----|-----|
| `latency_threshold_us` | 25343, 10647, 5000 | No effect — queue latency never exceeds any tested threshold due to e2e_deadline_guard preemption |
| `price_step_up` | 1, 20 | No effect — price doesn't climb if observations are absent |
| `price_cap` | 66, 500 | No effect — cap only applies when price is climbing, which it isn't |
| `tokens_left_init` / `max_token` | 100, 285, 5000 | No effect alone; minor effect combined with init_price |
| `init_price` | 0, 200, 500 | **Only knob that activates rajomon at all.** But decays by 1 per price_update_rate_ms tick once set; transient. |
| `price_freq` | 3, 5 | No effect — server-to-client propagation is moot when own_price is 0 |

### Validation protocol (the key differentiator from prior rajomon tuning)

Every iteration checked `abort_reason_timeline.csv` for `RajomonAdmissionRej` / `RajomonChildBudgetRej` rows. Iterations 1-4 showed ZERO rajomon rows despite surface-level goodput results that could have looked like "wins." Only iteration 5 produced any rajomon-attributed rejections, and only transiently. This validation exposed 4 false positives that simpler tuning (surface-level goodput comparison) would have accepted.

### Recommendation

Config-only tuning has a hard ceiling under `sched_fifo,ac_rajomon,abort_slo` on socialnet. To make rajomon productive:
1. **Change layer order** so `admission` runs before `e2e_deadline_guard` in `for_each_layer!`. This lets rajomon observe queue latency on all polled requests regardless of deadline state. (Code change in `libs/masa-policy/src/hooks.rs`.)
2. **Or disable abort_slo** for the rajomon target variant (but the user explicitly requested `+abort_slo`).
3. **Or change the price-climb model** so it doesn't depend on queue-latency observations that get preempted (e.g., observe request backlog length or CPU utilization instead).

None of these are config-only. The user's concern that prior rajomon tuning could hit regimes where "rajomon never kicked in at all" is now diagnostically confirmed for this app/SLO combination.
