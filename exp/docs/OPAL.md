# OPAL — admit-freely AC with ER-based cutback

## Key questions
- Can we eliminate the 15-18s slow ramp at load transitions by defaulting to "admit freely" instead of "ramp up from low budget"?
- Does using abort_slo's early-return rate as the error signal to trigger admission control produce stable, high goodput at overload?
- Can we match or beat anchor_20's overload performance while eliminating the slow ramp?

## Experiment series: opal_1, opal_2, ... (socialnet)

## Background

anchor_20 (best ANCHOR result) achieves strong overload goodput but has a 15-18s slow ramp at every load transition. The ramp is caused by the goodput EMA starting near zero after inter-step gaps, and the budget (initial_budget_rate=5M µs/s) supporting only ~200 req/s. ANCHOR iterations 21-27 tried various ramp fixes but couldn't decouple fast ramp from overload protection — they were all reverted.

**anchor_20 results (baseline for this track):**

| RPS | Mean | CoV |
|-----|------|-----|
| 800 | 800 | 0.1% |
| 1200 | 1182 | 0.7% |
| 1400 | 1254 | 5.1% |
| 1800 | 1451 | 13.7% |
| 2500 | 1601 | 27.0% |

**anchor_20 timeline problems:**
- 800→1200 (t=51): drops to 423, takes ~10s to reach 1200
- 1200→1400 (t=101): drops to 355, takes ~12s to reach 1400
- 1400→1800 (t=151): drops to 420, takes ~17s to reach 1600
- 1800→2500 (t=201): drops to 397, takes ~17s to reach 2100+
- 1800 RPS collapse at t=189: goodput crashes from 1604 to 1046, never fully recovers
- The ramp AND the stability once it stabilizes both matter — good features of anchor_20 include that it eventually finds a stable operating point

**The new approach (user-proposed):** Instead of starting from a low budget and ramping up to discover capacity, **admit freely by default and cut back based on abort_slo's error signal.** When ER rate is low (system healthy), skip the budget check entirely. When ER rate crosses a threshold (system overloaded), engage goodput-tracking budget to restrict admission. The key insight: abort_slo already kills late requests, so it's a natural overload signal.

**Why this should work where anchor_21-27 failed:**
- anchor_21 skipped budget in explore mode but goodput_rate decayed during gaps → bad budget when entering exploit
- In this approach, goodput_rate always tracks real traffic (we admit freely), so it's always accurate when budget engages
- The brief over-admission burst at overload transitions is handled by abort_slo (kills late requests cheaply)

---

## Iteration 1: ER-gated admission — admit freely until ER signals overload (experiment opal_1)

**Status:** Regression ❌ — reverted

**Code commit:** 6c98d971

### Change
Replace budget-based `rejection_ema` with ER-rate-based mode switching:

1. Add `er_count` and `total_count` lock-free accumulators to `AdmissionController`
2. Add `er_ema` field to `BudgetState`, tracked from accumulated ER events
3. In `after_child_rpc`: for ingress (hop_count==0), if response is early-return, increment `er_count`; always increment `total_count` for any completion
4. In `should_admit`:
   - Drain ER accumulators, compute instantaneous ER rate, update `er_ema` with time-based alpha (tau)
   - Skip EMA updates (both goodput_rate and er_ema) during idle gaps (elapsed > 0.5s) to preserve state across load transitions
   - If `er_ema <= rejection_threshold` (0.10): **always admit** (skip budget check entirely)
   - If `er_ema > rejection_threshold`: use goodput-tracking budget (budget_rate = goodput_rate * (1 + probe_min))
5. Remove old budget-based `rejection_ema`

### Hypothesis
The slow ramp exists because the budget (initial_budget_rate=5M, ~200 req/s) bottlenecks admission during discovery. By skipping the budget entirely when ER rate is low, we admit at whatever rate the load generator sends. The goodput_rate EMA runs on real traffic and is always accurate.

When overload occurs (2500 RPS, capacity ~1600), abort_slo kills excess requests. ER rate rises above 0.10. Budget engages with accurate goodput_rate → budget_rate = ~1600*25000*1.05 ≈ capacity. System stabilizes.

At 1800 RPS (~7% natural ER rate, below threshold 0.10), the AC stays disengaged → system self-regulates via abort_slo → matches baseline ~1680.

Key difference from failed anchor_21: idle gap preservation keeps goodput_rate and er_ema stable across load transitions. anchor_21 had goodput_rate decay to zero during gaps.

### Expected outcomes if hypothesis is correct:
1. Near-instant ramp at all load transitions (no 15-18s climb)
2. 800/1200 at baseline levels (no AC interference, no shedding)
3. 1800 RPS recovers to near-baseline (~1650-1700) — AC stays disengaged
4. 1400/2500 maintain or exceed anchor_20 gains
5. Timeline shows stable operating points once system reaches capacity (good feature preserved)
6. Brief burst of SLO misses at overload transitions (~1-2s), acceptable

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each). Direct comparison to anchor_20.

### Actual Outcomes (opal_1)

**Status:** Regression ❌ — reverted

| RPS | anchor_20 Mean(CoV) | opal_1 Mean(CoV) | Δ Mean |
|-----|---------------------|-------------------|--------|
| 800 | 800 (0.1%) | 800 (0.0%) | 0 |
| 1200 | 1182 (0.7%) | 1177 (4.6%) | -5 |
| 1400 | 1254 (5.1%) | 934 (24.3%) | **-320** |
| 1800 | 1451 (13.7%) | 613 (61.7%) | **-838** |
| 2500 | 1601 (27.0%) | 746 (59.9%) | **-855** |

**Key findings:**
1. **Sub-saturation ramp eliminated** — 800→1200 transitions instantly. This part works.
2. **Catastrophic oscillation at overload** — bang-bang control loop: ER < 0.10 → admit freely → flood → ER > 0.10 → budget engages at depressed goodput_rate → ER drops → budget disengages → flood again. Period ~8-10s.
3. **Collapses to ~91 goodput** at 1800 RPS (min). The oscillation troughs are near-total: the budget locks in at a terrible level from the crashed goodput_rate, then disengages and floods.
4. **Root cause:** The binary switch between "admit freely" and "budget-constrained" guarantees limit-cycle oscillation at any load above saturation. Each mode creates the conditions for transitioning to the other.

**Decision:** Revert code. The ER infrastructure (er_count, total_count, er_ema) is correct — the problem is the binary mode switching.

---

## Iteration 2: One-way latch — budget stays engaged once triggered (experiment opal_2)

**Status:** Regression ❌ — reverted

**Code commit:** 680343fe

### Change
Add a `budget_engaged: bool` latch to BudgetState. Once ER rate crosses the threshold, the budget stays engaged permanently until the next idle gap. During idle gaps, reset er_ema to 0 and budget_engaged to false — each load level gets a fresh start with free admission.

Specific changes to `should_admit`:
1. During idle gap (elapsed > 0.5s): reset `er_ema = 0.0` and `budget_engaged = false` (clean start for next load level). Preserve `goodput_rate` (accurate capacity estimate from previous step).
2. If `!budget_engaged` and `er_ema <= rejection_threshold`: always admit (skip budget).
3. If `er_ema > rejection_threshold`: set `budget_engaged = true` (one-way latch).
4. If `budget_engaged`: use goodput-tracking budget (`budget_rate = goodput_rate * (1 + probe_min)`). Do NOT disengage even if er_ema drops.

### Hypothesis
opal_1's oscillation was caused by the budget repeatedly disengaging when ER dropped, causing floods. With the one-way latch:

1. Each load level starts with free admission (instant ramp, no budget bottleneck).
2. If the load is sub-saturation: ER rate stays near zero → budget never engages → admits freely → matches baseline.
3. If the load is above saturation: ER rate rises → budget engages within ~0.5s → goodput_rate is accurate (from real traffic) → budget_rate = capacity * 1.05 → stable admission. Budget stays engaged → no cycling.
4. The brief initial over-admission (~0.5s before budget engages) is handled by abort_slo.
5. After an idle gap (load transition): reset → free admission again → instant ramp for next load level.

The goodput_rate preservation across gaps is critical: when entering a new higher load (e.g., 1400→1800), the preserved goodput_rate from 1400 (~1400*25000=35M) means the budget, if engaged, starts at 35M*1.05 ≈ 1470 req/s — enough to handle 1800 minus the excess that abort_slo catches.

### Expected outcomes if hypothesis is correct:
1. Instant ramp at all load transitions (same as opal_1's success at sub-saturation)
2. 800/1200: no AC interference (ER rate below threshold → admits freely)
3. 1400: either admits freely (if natural ER < 10%) or budget engages and stabilizes
4. 1800: admits freely (ER rate ~7%, below 10% threshold) → matches baseline ~1680
5. 2500: budget engages after ~0.5s → stabilizes at capacity → no oscillation
6. No collapses — the one-way latch prevents the disengagement that caused opal_1's crashes

### Experiment design (opal_2)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_2)

**Status:** Regression ❌ — reverted

| RPS | anchor_20 Mean(CoV) | opal_2 Mean(CoV) | Δ vs anchor_20 |
|-----|---------------------|-------------------|----------------|
| 800 | 800 (0.1%) | 800 (0.0%) | 0 |
| 1200 | 1182 (0.7%) | 1178 (2.6%) | -4 |
| 1400 | 1254 (5.1%) | 1130 (18.4%) | **-124** |
| 1800 | 1451 (13.7%) | 1593 (22.1%) | +142 (misleading — oscillating 808-1801) |
| 2500 | 1601 (27.0%) | 693 (70.6%) | **-908** |

**Key findings:**
1. **One-way latch did not prevent oscillation.** The idle-gap reset creates a new limit cycle: budget engages → over-throttles → reset → flood → engage again. At 2500, collapses to 102 goodput (as bad as opal_1).
2. **1200 falsely triggered** — even ~1.2% ER rate eventually pushes er_ema above threshold.
3. **1800 mean improvement is misleading** — oscillating between ~808 and ~1800 with 22.1% CoV. The peaks pull the mean up but behavior is unstable.
4. **Root cause:** The idle-gap-based reset mechanism is itself a source of oscillation. Additionally, the initial "admit freely" phase at deep overload causes a queue catastrophe that depresses goodput_rate, and the budget locks in at a terrible level.

**Lesson learned:** Engineering for idle gaps is wrong — they're a benchmark artifact, not a real workload property. Also, any approach that admits freely at deep overload causes a queue catastrophe that poisons subsequent budget calculations.

**Decision:** Revert code. Stop special-casing idle gaps. Need a fundamentally different approach.

---

## Iteration 3: Continuous ER-proportional probe — budget always on, ER scales the probe factor (experiment opal_3)

**Status:** Pending

### Change
Keep the budget always on (no "skip budget" mode). Replace the binary explore/exploit switching with continuous ER-proportional probe scaling:

```
probe = probe_max * max(0, 1.0 - er_ema / er_saturate)
budget_rate = goodput_rate * (1.0 + probe)
```

Parameters:
- `probe_max = 1.0` (2x goodput when no ER)
- `er_saturate = 0.20` (ER rate at which probe reaches 0; budget = goodput_rate)
- `probe_min` is effectively 0 (at er_ema ≥ 0.20, budget = goodput_rate exactly)

ER tracking: Same as opal_1/2 (er_count, total_count accumulators from after_child_rpc, time-based alpha EMA in should_admit).

No idle-gap special casing. No mode switching. No latch. Just continuous proportional control.

Remove `rejection_ema` entirely (replaced by `er_ema`). Remove `rejection_threshold` usage for mode switching (continuous, no threshold). Remove explore/exploit branching.

### Hypothesis
opal_1 and opal_2 failed because of binary mode switching (admit-freely vs budget-constrained). The sharp transition guarantees oscillation — each mode creates conditions for the other.

Continuous proportional control eliminates the mode switch. The budget_rate responds smoothly to ER rate:
- er_ema = 0 (sub-saturation): probe = 1.0, budget_rate = goodput_rate * 2.0. Generous.
- er_ema = 0.07 (1800 RPS, ~7% natural ER): probe = 0.65, budget_rate = goodput_rate * 1.65. Still generous — admits most of 1800.
- er_ema = 0.10: probe = 0.50, budget_rate = goodput_rate * 1.50.
- er_ema = 0.20+ (deep overload): probe = 0, budget_rate = goodput_rate. Tightest.

The feedback loop is inherently stable: ER increase → probe decrease → less admission → less ER. Small perturbations cause small responses. No cliff edges.

ANCHOR iteration 12 tried linear interpolation but it failed because the transition was "too gradual" with budget-based rejection_ema (indirect signal). ER-based er_ema measures ACTUAL system overload, making the proportional response more meaningful.

**What about the slow ramp?** At cold start, goodput_rate = initial_budget_rate = 5M µs/s. probe = 1.0 (no ER). budget_rate = 10M → 400 req/s at 25ms. Still a slow ramp. But once running, when load increases gradually (production scenario), the budget tracks: goodput_rate converges to current capacity, probe stays ~1.0, budget_rate = 2x capacity → handles load increases up to 2x without restriction. The ramp from 400 to 800 is ~3-4s (doubling per cycle).

### Expected outcomes if hypothesis is correct:
1. No oscillation at any RPS (continuous control → no mode switch cliff)
2. 800/1200: budget generous enough for full admission (probe ~1.0)
3. 1400: budget tightens slightly, goodput near anchor_20 or better
4. 1800: probe ~0.65, budget admits most of 1800, closer to baseline than anchor_20
5. 2500: probe ~0, tight tracking, anchor_20-level goodput or better
6. Cold-start ramp improved but not eliminated (~4-5s vs 15s)

### Experiment design (opal_3)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_3)

**Status:** Mixed — partial success, iterate

**Code commit:** 91f1632e

| RPS | anchor_20 Mean(CoV) | opal_3 Mean(CoV) | Δ Mean | Δ CoV |
|-----|---------------------|-------------------|--------|-------|
| 800 | 800 (0.1%) | 800 (0.0%) | 0 | -0.1pp |
| 1200 | 1182 (0.7%) | 1183 (1.9%) | +1 | +1.2pp |
| 1400 | 1254 (5.1%) | 1187 (18.6%) | **-67** | +13.5pp |
| 1800 | 1451 (13.7%) | 1006 (24.8%) | **-445** | +11.1pp |
| 2500 | 1601 (27.0%) | 905 (30.8%) | **-696** | +3.8pp |

**Key findings:**
1. **Ramp eliminated at sub-saturation** — 800→1200 instant, 1200→1400 instant. Validates the user's core hypothesis.
2. **No mode-switching oscillation** — continuous probe eliminates bang-bang cycling. Remaining oscillation is from goodput-tracking feedback.
3. **er_saturate=0.20 is too low** — at 1800, ER rate quickly exceeds 20%, clamping probe to 0 → budget = goodput_rate * 1.0 → positive feedback death spiral.

**Decision:** Keep opal_3 code, tune er_saturate higher.

---

## Iteration 4: Increase er_saturate to 0.50 + add probe floor (experiment opal_4)

**Status:** Pending

### Change
Two changes in should_admit:
1. `er_saturate`: 0.20 → 0.50. Probe reaches 0 only at 50% ER rate.
2. Probe formula with floor: `probe = probe_min + (probe_max - probe_min) * max(0, 1 - er_ema / er_saturate)`. Budget always has ≥5% margin.

Updated probe curve:
- er_ema = 0: probe = 1.0
- er_ema = 0.10: probe = 0.86
- er_ema = 0.25: probe = 0.53
- er_ema = 0.50+: probe = 0.05 (floor)

### Hypothesis
opal_3's regression was caused by probe going to 0 at moderate ER rates (>20%). With er_saturate=0.50 and probe_min=0.05:
- At 1800 RPS: with budget restricting admission, the actual ER rate should be much lower than opal_3's uncontrolled 33%. Budget stays generous → system self-regulates.
- At 2500 RPS: higher ER → probe ~0.30-0.53 → budget admits 1.30-1.53x goodput → gradual probing.
- probe_min=0.05 prevents the feedback death spiral at any ER level.

### Expected outcomes:
1. 800/1200: unchanged (probe ~1.0)
2. 1400: improved (probe stays higher)
3. 1800: significant recovery toward baseline
4. 2500: significant recovery from opal_3
5. Lower CoV at all overloaded RPS

### Experiment design (opal_4)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_4)

**Status:** Mixed — 2500 improved, 1800 worse

**Code commit:** 1ad4dd67

| RPS | anchor_20 Mean(CoV) | opal_3 Mean(CoV) | opal_4 Mean(CoV) | Δ vs anchor_20 |
|-----|---------------------|-------------------|-------------------|----------------|
| 800 | 800 (0.1%) | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1182 (0.7%) | 1183 (1.9%) | 1175 (3.0%) | -7 |
| 1400 | 1254 (5.1%) | 1187 (18.6%) | 1127 (19.7%) | **-127** |
| 1800 | 1451 (13.7%) | 1006 (24.8%) | 974 (28.2%) | **-477** |
| 2500 | 1601 (27.0%) | 905 (30.8%) | **1320 (20.4%)** | -281 |

**Key findings:**
1. **2500 dramatically improved** (+415 vs opal_3, CoV 30.8%→20.4%). Wider er_saturate and probe floor prevent feedback death spiral.
2. **1800 slightly worse** (-31 vs opal_3). Higher probe at 1800 causes repeated over-admission floods: starts at 1800 for 3.5s, drops to 1200 range, crashes to 572-700 periodically.
3. **Core tension:** Higher er_saturate helps 2500 (probe stays positive) but hurts 1800 (probe stays too high, causing over-admission).
4. The goodput-tracking feedback loop is the remaining problem: goodput dip → budget drops → less admission → deeper dip. The ER probe can't fix this because ER responds to overload, not to the AC's own feedback.

**Decision:** Keep opal_4 code, add asymmetric goodput tau to break feedback loop.

---

## Iteration 5: Asymmetric goodput EMA — slow decay breaks feedback loop (experiment opal_5)

**Status:** Pending

### Change
Make the goodput_rate EMA asymmetric: fast rise (tau=1.0s), slow decay (tau=5.0s). When instantaneous goodput drops below the EMA, the EMA decays 5x slower, preventing transient dips from crashing the budget.

```rust
let tau = if instant_rate >= state.goodput_rate {
    p.tau        // 1.0s — fast discovery
} else {
    p.tau * 5.0  // 5.0s — slow decay
};
```

Combined with opal_4's ER-proportional probe (er_saturate=0.50, probe floor=0.05).

### Hypothesis
The remaining oscillation at 1800/2500 is driven by the goodput-tracking feedback loop: a transient goodput dip causes budget_rate to drop, restricting admission, causing further goodput drops. The ER probe modulates the margin but can't fix the underlying budget instability.

Asymmetric tau addresses this directly: the slow downward decay prevents transient dips from cascading. When goodput dips briefly (e.g., burst of SLO misses), goodput_rate holds steady, budget holds steady, admission stays stable, and goodput recovers.

ANCHOR iteration 24 proved this works: asymmetric tau achieved 1725 at 1800 RPS (4.7% CoV) — better than baseline. It failed at 2500 because the slow decay prevented tracking genuine overload. But with the ER-proportional probe, the probe factor handles overload tightening independently of goodput_rate: at high ER, probe drops, reducing the budget even if goodput_rate stays inflated.

**The synergy:** Asymmetric tau stabilizes the budget at moderate overload (breaks feedback loop). ER-proportional probe tightens at deep overload (compensates for inflated goodput_rate).

### Expected outcomes:
1. 800/1200: unchanged (probe ~1.0, no ER)
2. 1400: improved stability (slow decay absorbs dips)
3. 1800: significantly better than opal_4 — slow decay prevents post-flood crashes
4. 2500: maintained or improved from opal_4 — ER probe handles overload, slow decay stabilizes between oscillation peaks
5. CoV should improve at all overloaded RPS

### Experiment design (opal_5)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_5)

**Status:** Regression ❌ — reverted

**Code commit:** 720b318c (reverted)

| RPS | opal_4 Mean(CoV) | opal_5 Mean(CoV) | Δ Mean |
|-----|-------------------|-------------------|--------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1175 (3.0%) | 1185 (2.2%) | +9 |
| 1400 | 1127 (19.7%) | 964 (36.9%) | **-163** |
| 1800 | 974 (28.2%) | 714 (51.3%) | **-260** |
| 2500 | 1320 (20.4%) | 616 (73.5%) | **-704** |

**Key finding:** Slow goodput_rate decay keeps the budget inflated during overload, perpetuating over-admission. Even at probe_min=0.05, an inflated goodput_rate at 50M+ admits ~2100 req/s when capacity is ~1600. The excess creates a persistent queue catastrophe. The ER probe cannot compensate because the goodput_rate inflation is the dominant term.

**Why ANCHOR iteration 24 worked but opal_5 didn't:** anchor_24 used asymmetric tau WITHOUT ER-based probe — it used the old budget-based rejection_ema. Budget rejections are instantaneous (the budget runs dry → immediate rejection), while ER signals are delayed (request must enter the system, process, and miss SLO). The instantaneous budget signal provided fast enough feedback to prevent queue buildup. The ER signal is too slow to pair with slow goodput decay.

**Decision:** Revert. Asymmetric tau + ER probe is not a viable combination.

---

## Summary and Conclusions (Iterations 1-5)

### Results table

| Iter | Key change | 800 | 1200 | 1400 | 1800 | 2500 |
|------|-----------|-----|------|------|------|------|
| **anchor_20** | **baseline** | **800 (0.1%)** | **1182 (0.7%)** | **1254 (5.1%)** | **1451 (13.7%)** | **1601 (27.0%)** |
| opal_1 | ER-gated admit-freely | 800 (0.0%) | 1177 (4.6%) | 934 (24.3%) | 613 (61.7%) | 746 (59.9%) |
| opal_2 | + one-way latch + gap reset | 800 (0.0%) | 1178 (2.6%) | 1130 (18.4%) | 1593 (22.1%) | 693 (70.6%) |
| opal_3 | continuous probe, er_sat=0.20 | 800 (0.0%) | **1183 (1.9%)** | 1187 (18.6%) | 1006 (24.8%) | 905 (30.8%) |
| opal_4 | er_sat=0.50, probe floor | 800 (0.0%) | 1175 (3.0%) | 1127 (19.7%) | 974 (28.2%) | 1320 (20.4%) |
| opal_5 | + asymmetric tau | 800 (0.0%) | 1185 (2.2%) | 964 (36.9%) | 714 (51.3%) | 616 (73.5%) |

### What worked
1. **ER-based probe eliminates the slow ramp at sub-saturation.** All opal iterations achieve instant transitions at 800→1200 and 1200→1400 (vs anchor_20's 10-15s ramp). This validates the user's core hypothesis about using ER to gate tightening.
2. **Continuous probe (opal_3+) eliminates mode-switching oscillation.** opal_1/2's bang-bang cycling is gone. Remaining oscillation is from the goodput-tracking feedback loop, not the probe mechanism.
3. **Probe floor prevents feedback death spiral.** opal_4's probe_min=0.05 dramatically improved 2500 (905→1320, CoV 30.8%→20.4%).

### What didn't work
1. **Admitting freely at overload** (opal_1/2): Queue catastrophe from uncontrolled admission. The initial flood overwhelms the system before the ER signal can respond.
2. **Binary mode switching** (opal_1/2): Bang-bang oscillation between admit-freely and budget-constrained. Neither mode creates stable equilibrium at overload.
3. **er_saturate too low** (opal_3): At 0.20, moderate overload (1800, ~30% ER) clamps probe to 0 → feedback death spiral.
4. **Asymmetric goodput tau** (opal_5): Inflated goodput_rate during overload causes persistent over-admission. ER signals are too delayed to compensate.

### Fundamental finding: ER signals are too slow for budget control

The core issue across all 5 iterations: **ER (early return) signals have inherent delay** (request must enter the system, be processed, and miss SLO before the ER is observed). Budget-based signals (anchor_20's rejection_ema) are instantaneous (budget runs dry → immediate rejection). This makes budget-based mode switching inherently more stable at overload.

ER signals are better for the *direction* of the probe (are we overloaded or not?) but too slow for the *timing* of budget changes. The goodput-tracking budget needs fast feedback to prevent queue buildup during overload transitions — and budget exhaustion provides exactly that.

### Remaining tension: slow ramp vs overload stability

anchor_20's budget-based approach is stable at overload but has a slow ramp. The ER-based approach eliminates the ramp but is unstable at overload. No configuration in 5 iterations achieved both instant ramp AND stable overload behavior.

A potential hybrid: use anchor_20's budget-based rejection_ema for exploit/explore switching (fast feedback, stable), but use ER rate to set the explore-mode budget generously (goodput_rate * f(er_ema) instead of fixed initial_budget_rate). This would speed up the ramp without sacrificing overload stability.

### Code state
Reverted to opal_4 code (commit 1ad4dd67) then reverted opal_5. Current code has the ER-proportional probe with er_saturate=0.50 and probe floor. This should be reverted to anchor_20 baseline for clean state.

---

## Next direction: Concurrency-based admission control

### Why concurrency limiting instead of rate limiting

All OPAL iterations (and ANCHOR iterations 11-27) used a **rate-based** admission controller: a token bucket where the refill rate tracks observed goodput. This architecture has two structural problems:

1. **Feedback death spiral:** `budget_rate = goodput_rate * (1 + probe)`. When goodput dips transiently, budget_rate drops, restricting admission, causing goodput to drop further. Every OPAL iteration suffered from this at overload.

2. **Slow discovery:** The budget can only admit as fast as goodput_rate allows, and goodput_rate can only grow as fast as the budget admits. This circular dependency is why the ramp takes 15-18s — each cycle increases admission by ~5%.

A **concurrency-based** admission controller avoids both problems by controlling a fundamentally different variable: not the *rate* of admission but the *number of requests in flight simultaneously*.

### How it works

Replace the token bucket with a concurrency limiter:

```
state:
    inflight: AtomicU32      // current in-flight request count
    limit: f64               // adaptive concurrency limit
    min_latency: f64         // observed no-load latency (floor)

on admission check (ingress only):
    if inflight >= limit:
        return REJECT
    inflight += 1
    return ADMIT

on completion (ingress only):
    inflight -= 1
    update latency estimate from this request's observed latency
    gradient = min_latency / current_avg_latency
    // gradient ≈ 1.0 when no queueing, < 1.0 when overloaded
    limit = limit * gradient + headroom
```

### Why this solves both problems

**No feedback death spiral.** The concurrency limit is a structural property ("how many requests can the system handle simultaneously"), not a derivative of output. When the controller restricts admission, the limit doesn't shrink — it holds at whatever the latency signal says is right. Contrast with rate-based: restricting admission reduces goodput, which reduces the budget, which restricts further.

**Fast discovery via latency signal.** Latency is immediate: admit one request too many → latency increases → you know instantly. No need to wait for the request to miss SLO (ER delay) or for a goodput EMA to converge (rate delay). AIMD-style discovery (increase limit by 1 per successful completion) reaches optimal concurrency in ~1 second:
- At 1600 req/s capacity, 25ms avg latency: optimal concurrency ≈ 40
- Starting from limit=10, +1 per completion at 25ms: reaches 40 in ~30 completions = **0.75s**

**Overload rejection is instant and prevents queue buildup.** At 2500 RPS with limit=40: 40 requests in flight, 1600 completions/s, excess **rejected immediately at the gate** before entering the processing pipeline. No queue catastrophe, no wasted processing. The token bucket also rejects at the gate, but its rate calculation is coupled to a slow-moving EMA.

### Latency-based limit adaptation (Vegas-style)

The limit adapts using the ratio of no-load latency to current latency, similar to TCP Vegas:

```
gradient = min_latency / smoothed_avg_latency
new_limit = current_limit * gradient + queue_allowance
```

- `gradient ≈ 1.0` (latency ≈ min): system has headroom → limit can grow
- `gradient < 1.0` (latency inflated by queueing): system overloaded → limit shrinks proportionally
- `min_latency`: the observed floor latency when the system is not congested. Can be tracked as a running minimum with periodic decay to avoid stale floors.
- `queue_allowance`: small additive term (~sqrt(limit)) that allows gradual probing above the current equilibrium.

The proportional response (`limit * gradient`) is inherently stable: if you overshoot, latency increases, gradient drops, limit drops. If you undershoot, latency stays at floor, gradient stays at 1, limit grows. Small perturbations cause small corrections — no cliff edges, no oscillation.

### Cost heterogeneity

The main tradeoff vs. the token bucket: concurrency limits don't naturally account for per-API cost differences (a cheap 5ms API and an expensive 50ms API both consume one concurrency slot). Options:

1. **Weighted concurrency:** Each request consumes `est_child_cost / baseline_cost` slots. Expensive APIs consume more of the limit. This preserves cost-awareness while using concurrency as the control variable.
2. **Per-API limits:** Separate concurrency limits per downstream API. More complex but precise.
3. **Ignore for now:** Socialnet's ComposePost is the only API under test. Add cost-awareness later if needed for Hotel (which has diverse API costs).

### What to keep from anchor_20

- **Layer 1 (deadline feasibility check):** Unchanged. Independent of the admission mechanism.
- **Latency estimator infrastructure:** The `est_child_latency` map provides per-API latency estimates that feed the concurrency limiter's min_latency and cost weighting.
- **Goodput tracking:** Can still track goodput_rate for observability/logging, just don't use it to set the admission rate.

### Implementation sketch

In `predictive.rs`, replace `AdmissionController` (token bucket) with `ConcurrencyLimiter`:

```rust
struct ConcurrencyLimiter {
    inflight: AtomicU32,
    state: Mutex<LimiterState>,
}

struct LimiterState {
    limit: f64,                    // current concurrency limit
    min_latency_us: f64,           // observed no-load latency floor
    avg_latency_us: f64,           // EMA of recent latencies
    last_update: Instant,
}
```

The `should_admit` check becomes: `inflight.load() < limit`. On admission: `inflight.fetch_add(1)`. On completion (after_child_rpc): `inflight.fetch_sub(1)` + update latency + adjust limit.

### Expected advantages over OPAL/ANCHOR approaches

1. **Sub-second discovery** (vs 15s ramp): AIMD grows limit by 1 per completion
2. **No feedback death spiral** at overload: limit based on latency ratio, not output rate
3. **Instant overload rejection**: concurrent count exceeds limit → reject before processing
4. **Inherently stable**: proportional control via latency gradient, no mode switching needed

---

## Iteration 6: Vegas-style concurrency limiter baseline test (experiment opal_6)

**Status:** Pending

**Code commit:** ba29123f

### Change
Replace the token-bucket `AdmissionController` with `ConcurrencyLimiter` as designed above. Key implementation details:
- `should_admit`: atomically increment `inflight`; admit if pre-increment < `limit`, else decrement and reject
- `record_completion(latency_us)`: decrement inflight, update min_latency (instant adopt of new lows, slow upward decay with alpha=0.001), update avg_latency EMA (tau=0.5s), adapt limit via `limit = limit * (min_latency / avg_latency) + queue_allowance_factor * sqrt(limit)`
- `record_drop()`: decrement inflight without latency update (early-return/error requests have atypical latency)
- Initial limit: 100.0 (generous start to avoid slow ramp)
- Queue allowance factor: 1.0

### Hypothesis
The Vegas-style concurrency limiter should solve both structural problems of the rate-based token bucket:

1. **No feedback death spiral:** The limit is based on latency ratio (min/avg), not output rate. Restricting admission doesn't shrink the limit — it should hold at whatever latency says is right.
2. **Fast discovery:** At 1600 req/s capacity with 25ms latency, optimal concurrency ≈ 40. Starting from limit=100 (generous), the limiter should quickly converge downward via gradient < 1.0 when overloaded, and hold steady when not.
3. **Instant overload rejection:** At 2500 RPS, once limit stabilizes at ~40, excess requests are rejected immediately at the gate.

The initial_limit=100 is deliberately generous — at 25ms avg latency, 100 concurrent slots supports ~4000 req/s, well above the 2500 max test load. This means no artificial slow ramp at any load level.

### Expected outcomes if hypothesis is correct:
1. 800/1200: full admission, no shedding (limit stays at or above needed concurrency)
2. 1400: near-full admission, slight tightening if latency rises
3. 1800: limit converges to ~45 (1800 * 25ms), admits most traffic. Should match or beat anchor_20's 1451.
4. 2500: limit converges to ~40 (capacity * avg_latency), excess rejected instantly. Should match or beat anchor_20's 1601.
5. No slow ramp at any transition — limit starts at 100, adjusts smoothly
6. Low CoV — proportional control should be inherently stable (no oscillation)

### Experiment design (opal_6)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each). Direct comparison to anchor_20 baselines.

### Actual Outcomes (opal_6)

**Status:** Regression ❌ — iterate

**Code commit:** ba29123f

| RPS | anchor_20 Mean(CoV) | opal_6 Mean | opal_6 ER/s | Δ vs anchor_20 |
|-----|---------------------|-------------|-------------|----------------|
| 800 | 800 (0.1%) | 785 | 15.2 | **-15** |
| 1200 | 1182 (0.7%) | 1003 | 197.1 | **-179** |
| 1400 | 1254 (5.1%) | 985 | 415.3 | **-269** |
| 1800 | 1451 (13.7%) | 982 | 817.7 | **-469** |
| 2500 | 1601 (27.0%) | 90 | 2409.8 | **-1511** |

**Key findings:**
1. **Catastrophic regression at every load point.** Even at 800 RPS (below saturation), the limiter sheds 15 req/s.
2. **Goodput capped at ~985 regardless of offered load** (1200-1800 all converge to ~982-985). The concurrency limit has converged far too low.
3. **Complete collapse at 2500 RPS** — 90 goodput, 96.4% of requests early-returned. Classic death spiral: high latency → low gradient → limit shrinks to near-zero → few requests admitted → those see low latency → but min_latency already ratcheted down → no recovery.
4. **Root cause: `gradient = min_latency / avg_latency` is structurally biased below 1.0.** Even at low load, avg_latency >> min_latency due to natural processing time variance. With factor=1.0 and gradient=0.5 (avg = 2*min), equilibrium limit = `(factor / (1 - gradient))^2 = 4`. This caps throughput far below capacity.

**Equilibrium analysis:** At steady state, `limit * gradient + factor * sqrt(limit) = limit`, so `limit = (factor / (1 - gradient))^2`. With gradient=0.5, factor=1.0: limit=4 (~160 req/s). This explains the ~985 cap — gradient is likely ~0.7-0.8 at low load, giving limit ≈ 9-25, just enough for ~985 at observed latencies.

**Decision:** Keep code, tune parameters. The mechanism is correct but the queue_allowance_factor is far too low relative to the natural gradient bias.

---

## Iteration 7: Increase queue_allowance_factor + add limit floor (experiment opal_7)

**Status:** Pending

**Code commit:** TBD

### Change
Two parameter changes in `ConcurrencyLimiter`:
1. `queue_allowance_factor`: 1.0 → 5.0. At the equilibrium equation `limit = (factor / (1 - gradient))^2`:
   - gradient=0.5 (avg = 2*min, healthy): limit = (5/0.5)^2 = **100** → supports ~4000 req/s
   - gradient=0.33 (avg = 3*min, moderate overload): limit = (5/0.67)^2 = **55** → supports ~2200 req/s
   - gradient=0.20 (avg = 5*min, deep overload): limit = (5/0.8)^2 = **39** → supports ~1560 req/s (near actual capacity)
2. Add `limit_floor = 10.0`: limit can never drop below 10, preventing the death spiral at 2500 RPS.

### Hypothesis
opal_6 failed because queue_allowance_factor=1.0 is too small to compensate for the natural gradient bias (avg_latency is always significantly higher than min_latency, even at low load). The limit equation shows the equilibrium is extremely sensitive to this factor (quadratic relationship).

With factor=5.0:
- At sub-saturation (gradient ~0.5-0.7): equilibrium limit 50-100, generous enough for full admission
- At moderate overload (gradient ~0.33): limit ~55, still above capacity concurrency (~40)
- At deep overload (gradient ~0.20): limit ~39, close to capacity concurrency → meaningful shedding

The limit floor prevents the catastrophic 2500 RPS collapse: even at gradient→0, limit stays ≥10 → admits ~400 req/s → system processes them → latencies recover → gradient rises → limit recovers.

### Expected outcomes if hypothesis is correct:
1. 800/1200: full or near-full admission (limit equilibrium well above needed concurrency)
2. 1400: improved significantly, near anchor_20 levels
3. 1800: significant recovery — limit ~55 supports most of 1800
4. 2500: no collapse — limit ~39 provides meaningful shedding, goodput should approach capacity (~1600)
5. Overall: should close the gap with anchor_20, especially at high loads

### Experiment design (opal_7)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_7)

**Status:** Regression ❌ — iterate

**Code commit:** 1ea45182

| RPS | anchor_20 Mean(CoV) | opal_6 Mean | opal_7 Mean | opal_7 ER/s | Δ vs anchor_20 |
|-----|---------------------|-------------|-------------|-------------|----------------|
| 800 | 800 (0.1%) | 785 | 800 | 0 | 0 |
| 1200 | 1182 (0.7%) | 1003 | 476 | 724 | **-706** |
| 1400 | 1254 (5.1%) | 985 | 0 | 1400 | **-1254** |
| 1800 | 1451 (13.7%) | 982 | 0 | 1800 | **-1451** |
| 2500 | 1601 (27.0%) | 90 | 0 | 2500 | **-1601** |

**Key findings:**
1. **Catastrophically worse than opal_6 at all overloaded loads.** At 1400+ RPS, 100% of requests are early-returned.
2. **Root cause: limit grew too high, allowing over-admission.** With factor=5.0, the equilibrium at healthy gradient (0.8) is (5/0.2)^2 = 625 (capped at... uncapped). The limit grows far above what the system needs, so all requests are admitted. At saturation loads (1200+), admitting all traffic causes latency inflation → mass SLO misses → 100% ER. Note: opal_6 at 1200 had ~200 ER/s with restrictive limits; opal_7 has 724 ER/s because it admits all 1200 into an overloaded system.
3. **Opposite failure from opal_6:** opal_6 limit too low (factor=1.0, equilibrium ~25) → over-restriction. opal_7 limit too high (factor=5.0, equilibrium ~278) → under-restriction. No factor value works across all loads.
4. **Fundamental issue with Vegas proportional formula:** Equilibrium = (factor / (1-gradient))^2 is extremely sensitive to gradient. A 10% gradient change causes 4x equilibrium change. The formula cannot find a stable operating point across varying loads.

**Decision:** Revert queue_allowance_factor=5.0. Replace the proportional Vegas formula with AIMD (Additive Increase, Multiplicative Decrease) which has a naturally stable equilibrium.

---

## Iteration 8: AIMD adaptation replaces Vegas proportional formula (experiment opal_8)

**Status:** Pending

**Code commit:** TBD

### Change
Replace the Vegas proportional limit adaptation with AIMD + periodic updates:

1. Add `last_limit_update: Instant` to `LimiterState`
2. In `record_completion`: update latency EMA only, do NOT update limit
3. Trigger limit update from `record_completion` when ≥250ms elapsed since last update:
   - gradient = min_latency / avg_latency
   - If gradient ≥ 0.70 (healthy): limit = min(limit + 5.0, max_limit)
   - Else (congested): limit = max(limit * 0.90, floor)
4. Parameters:
   - initial_limit = 100 (unchanged)
   - max_limit = 100 (cap, prevents runaway growth)
   - floor = 10 (prevents death spiral)
   - gradient_threshold = 0.70
   - additive_increase = 5.0 per update (20/s → reaches cap from floor in ~5s)
   - multiplicative_decrease = 0.90 (gentle: 10% per step → halves in ~7 steps = 1.75s)
   - update_interval = 250ms

### Hypothesis
The Vegas proportional formula fails because its equilibrium is quadratically sensitive to gradient — no single factor value works across load regimes. AIMD avoids this by having a fundamentally different control structure:

**Why AIMD should work:**
- Additive increase is bounded (+5/step, capped at 100): can't over-grow
- Multiplicative decrease is gentle (×0.9): takes 1.75s to halve, gives system time to stabilize
- Periodic updates (250ms): smooths out transient latency spikes
- Threshold-based: only acts on sustained gradient changes, not noise

**Expected dynamics at each load:**
- 800/1200 RPS (sub-saturation): gradient ~0.8 > 0.70 → limit grows to cap (100). All admitted. ✓
- 1800 RPS (above saturation): gradient oscillates around 0.70. Limit oscillates between ~30 and ~45. Average ~37 concurrent → ~1500 req/s admitted → goodput ~1400-1500 after abort_slo. Close to anchor_20's 1451.
- 2500 RPS (deep overload): gradient < 0.70 → limit shrinks toward 30-40. Similar dynamics to 1800 but with more shedding. Goodput ~1400-1600.

**Key difference from Vegas:** AIMD's equilibrium is where the system naturally transitions between "healthy" and "congested" — a structural property of the workload, not a sensitive function of parameters.

### Expected outcomes if hypothesis is correct:
1. 800/1200: full admission, matching anchor_20 (limit at cap, no restriction)
2. 1400: near-full admission, goodput ≥ 1200
3. 1800: significant improvement over opal_6's 982 — target 1300-1500
4. 2500: significant improvement over both opal_6 (90) and opal_7 (0) — target 1300-1600
5. No death spiral at any RPS
6. Moderate oscillation acceptable; CoV should be < 30%

### Experiment design (opal_8)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

