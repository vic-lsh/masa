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

### Actual Outcomes (opal_8)

**Status:** Regression ❌

**Code commit:** b2e5b115

| RPS | anchor_20 Mean(CoV) | opal_6 Mean | opal_8 Mean | opal_8 ER/s | Δ vs anchor_20 |
|-----|---------------------|-------------|-------------|-------------|----------------|
| 800 | 800 (0.1%) | 785 | 799 | 0.8 | -1 |
| 1200 | 1182 (0.7%) | 1003 | 1158 | 42 | -24 |
| 1400 | 1254 (5.1%) | 985 | 984 | 416 | **-270** |
| 1800 | 1451 (13.7%) | 982 | **0** | 1800 | **-1451** |
| 2500 | 1601 (27.0%) | 90 | **0** | 2500 | **-1601** |

**Key findings:**
1. **Sub-saturation improved** — 1200 RPS: 1158 vs opal_6's 1003 (+155). AIMD keeps limit at cap (100) when gradient > 0.70, allowing full admission.
2. **Complete collapse at 1800 and 2500** — 0 goodput, 100% ER. When load jumps above capacity, initial_limit=100 admits everything. The multiplicative decrease (×0.9 per 250ms) is far too slow — takes 1.75s to halve the limit. By then, queued requests have inflated latencies to catastrophic levels. All requests miss SLO.
3. **Irrecoverable death spiral:** Once latency is catastrophically high, gradient stays near 0, limit shrinks to floor (10). But floor=10 is too low to generate enough completions for recovery — the few admitted requests see artificially low latency (no contention) but the limit is stuck at floor because the additive increase (+5 per 250ms = 20/s) starting from 10 is too slow relative to the load.
4. **Root cause: "start high, shrink down" is fundamentally wrong at overload transitions.** The initial burst of over-admission at transition (e.g., 1400→1800) floods the system before AIMD can react. This is the same problem as opal_1/2's "admit freely" approach but in concurrency form.

---

## Summary and Conclusions (Iterations 6-8: Concurrency-Based AC)

### Results table

| Iter | Key change | 800 | 1200 | 1400 | 1800 | 2500 |
|------|-----------|-----|------|------|------|------|
| **anchor_20** | **baseline (rate-based)** | **800** | **1182** | **1254** | **1451** | **1601** |
| opal_6 | Vegas concurrency, factor=1.0 | 785 | 1003 | 985 | 982 | 90 |
| opal_7 | Vegas, factor=5.0, floor=10 | 800 | 476 | 0 | 0 | 0 |
| opal_8 | AIMD, periodic updates | 799 | 1158 | 984 | 0 | 0 |

### What worked
1. **Concurrency gating concept is sound at sub-saturation.** All three iterations perform well at 800 RPS. opal_8 achieves 1158 at 1200 (vs anchor_20's 1182) — close to optimal.
2. **No slow ramp at sub-saturation transitions.** The initial_limit=100 provides instant full admission at loads below capacity.

### What didn't work
1. **Vegas proportional formula (opal_6, opal_7):** Equilibrium = (factor / (1-gradient))^2 is quadratically sensitive to gradient. factor=1.0 → limit too low (~25, capping at 985 req/s). factor=5.0 → limit too high (~278, no shedding). No factor value works across load regimes.
2. **AIMD periodic updates (opal_8):** "Start high, shrink down" causes irrecoverable over-admission at overload transitions. The multiplicative decrease (×0.9/250ms) cannot react fast enough — the system floods before the limit drops.
3. **All three iterations fail at 1800+ RPS.** The concurrency limiter either restricts too much (opal_6: 982) or too little (opal_7/8: 0).

### Fundamental findings

**1. Latency-based concurrency limiting has a structural cold-start/transition problem.** When load suddenly exceeds capacity, the limiter must admit excess traffic before it can observe the latency inflation that signals congestion. By the time the signal arrives, the queue catastrophe has already begun. Rate-based approaches (anchor_20) don't have this problem because the budget immediately constrains admission rate.

**2. The gradient signal (min_latency / avg_latency) conflates processing variance with congestion.** In Socialnet, natural per-request latency variation means avg >> min even at zero load. The gradient is structurally biased below 1.0, making it unreliable as a congestion signal. TCP Vegas works because network propagation delay is nearly constant; microservice processing time is not.

**3. Concurrency limiting doesn't directly control throughput.** A concurrency limit of L admits L/avg_latency req/s, but avg_latency changes with the number admitted. This creates a coupled feedback loop: admitting more → higher latency → lower apparent throughput → but the LIMIT hasn't changed. Rate-based limiting directly controls throughput, making it more predictable.

### Remaining directions (not yet tested)
1. **Start low, grow up:** initial_limit=10 instead of 100. Avoids over-admission at transitions. Risk: slow ramp (the original problem we were trying to solve).
2. **Hybrid: rate-based budget for overload + concurrency gate for discovery.** Use anchor_20's goodput-tracking budget when ER is high, concurrency gate when ER is low. Combines fast overload response with instant ramp.
3. **Per-API concurrency limits:** Instead of one global limit, per-downstream-service limits based on observed per-service latency. Would give cleaner signals.

### Code state
Current code has AIMD concurrency limiter (commit b2e5b115). Should be reverted to anchor_20 baseline for clean state if pursuing other directions.

---

## Iteration 9: Freeze goodput_rate across idle gaps — minimal anchor_20 fix (experiment opal_9)

**Status:** Pending

**Code commit:** TBD

### Change
Revert to anchor_20's token-bucket admission controller (undo all concurrency limiter changes). Then add a single targeted fix to `should_admit`:

```rust
// Skip goodput EMA update during idle gaps — preserves rate across load transitions.
if elapsed > 0.0 && elapsed <= 0.5 {
    let instant_rate = drained / elapsed;
    let alpha = 1.0 - (-elapsed / p.tau).exp();
    state.goodput_rate += alpha * (instant_rate - state.goodput_rate);
}
```

When `elapsed > 0.5s` (idle gap between load steps), the goodput_rate EMA is frozen at its previous value instead of decaying toward 0. Everything else in anchor_20 is unchanged.

### Hypothesis
anchor_20's slow ramp (15-18s) is caused by goodput_rate decaying to near-zero during idle gaps between load steps. When the new step starts, `budget_rate = goodput_rate * 1.05 ≈ 0`, so the token bucket admits almost nothing. The budget slowly refills as completions trickle in, taking 15+ cycles to discover capacity.

By freezing goodput_rate across gaps:
- After 800 RPS step: goodput_rate ≈ 800 * 25000 = 20M µs/s
- At 1200 RPS transition: budget_rate = 20M * 1.05 = 21M µs/s → admits ~840 req/s immediately
- Wait — that's still below 1200. But the system is in explore mode (rejection_ema should be low after a gap). In explore mode, budget_rate = initial_budget_rate = 5M µs/s (~200 req/s). The preserved goodput_rate only matters in exploit mode.

**Correction:** The slow ramp happens because in explore mode, `initial_budget_rate = 5M` caps admission at ~200 req/s regardless of previous goodput. The fix needs to also use the preserved goodput_rate in explore mode.

Updated change: In explore mode, use `max(initial_budget_rate, goodput_rate * (1.0 + probe_max))` instead of just `initial_budget_rate`. This way:
- If goodput_rate is preserved from previous step: budget_rate = goodput_rate * 2.0 (generous)
- If goodput_rate is cold (first step): budget_rate = initial_budget_rate (fallback)

### Expected outcomes if hypothesis is correct:
1. Near-instant ramp at all load transitions (budget starts at previous capacity × 2.0)
2. 800/1200: full admission, no shedding
3. 1400/1800/2500: same overload performance as anchor_20 (the token bucket mechanism is unchanged)
4. No oscillation — this is anchor_20's proven-stable control loop with one gap fix
5. Ramp from 800→1200: budget = 20M*2.0 = 40M → admits ~1600 req/s instantly (1200 < 1600, all admitted)
6. Ramp from 1400→1800: budget = 1400*25K*2.0 = 70M → admits ~2800 req/s (1800 < 2800, all admitted)

### Experiment design (opal_9)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_9)

**Status:** Regression ❌ — iterate

**Code commit:** ecb9bdb6

| RPS | anchor_20 Mean(CoV) | opal_9 Mean | opal_9 ER/s | Δ vs anchor_20 |
|-----|---------------------|-------------|-------------|----------------|
| 800 | 800 (0.1%) | 800 | 0 | 0 |
| 1200 | 1182 (0.7%) | 956 | 244 | **-226** |
| 1400 | 1254 (5.1%) | 1049 | 351 | **-205** |
| 1800 | 1451 (13.7%) | 1184 | 615 | **-267** |
| 2500 | 1601 (27.0%) | 1227 | 1270 | **-374** |

**Key findings:**
1. **Gap-freeze fix (change A) is fine.** The regression is caused entirely by the explore mode change (change B).
2. **Explore mode with goodput_rate * 2.0 causes bang-bang oscillation.** Near saturation, `rejection_ema` oscillates around the 0.10 threshold. Budget jumps between explore (goodput * 2.0) and exploit (goodput * 1.05) — a 1.9x swing. This creates admission oscillation → rejection oscillation → mode switching oscillation.
3. **Over-shedding at all overloaded RPS.** The oscillation means the system spends significant time in exploit mode with a depressed goodput_rate (from the over-admission burst during explore).
4. **Root cause confirmed:** The explore/exploit budget ratio (2.0/1.05 = 1.9x) is too large. Need a smaller explore multiplier.

**Decision:** Reduce probe_max to shrink the mode-switching jump.

---

## Iteration 10: Reduce probe_max to 0.2 — gentle explore mode (experiment opal_10)

**Status:** Pending

**Code commit:** TBD

### Change
In `policy_params.rs`, change `probe_max` from 1.0 to 0.2. This changes the explore mode budget from `goodput_rate * 2.0` to `goodput_rate * 1.2`.

The gap-freeze fix from opal_9 is retained. The explore mode still uses `max(goodput_rate * (1 + probe_max), initial_budget_rate)`.

### Hypothesis
opal_9's oscillation was caused by the 1.9x budget jump between explore and exploit modes. With probe_max=0.2:
- Explore budget = goodput_rate * 1.2
- Exploit budget = goodput_rate * 1.05
- Jump ratio = 1.2/1.05 = 1.14x — small enough that mode switching doesn't cause significant admission swings

**Ramp speed from 800→1200:**
- goodput_rate preserved at ~20M from 800 step
- Explore budget = 20M * 1.2 = 24M → ~960 req/s initially
- At 960 admitted: goodput converges to ~24M in ~1s (tau=1.0)
- Next cycle: budget = 24M * 1.2 = 28.8M → ~1152 req/s
- Next: ~1382 req/s → all 1200 admitted. Total ramp: ~2-3s vs anchor_20's 15s.

**Overload behavior:** At 2500 RPS, once rejection_ema > 0.10, exploit budget = goodput_rate * 1.05. This is identical to anchor_20's exploit mode. The 1.14x jump when cycling back to explore should not cause meaningful instability.

### Expected outcomes if hypothesis is correct:
1. 800/1200: full admission, faster ramp than anchor_20
2. 1400: near anchor_20 or better (gentle explore, no over-admission)
3. 1800: close to anchor_20 (exploit mode dominates at overload)
4. 2500: close to anchor_20 (same exploit behavior)
5. No mode-switching oscillation (1.14x jump is too small to cause instability)

### Experiment design (opal_10)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_10)

**Status:** Regression ❌ — iterate

**Code commit:** 3bde3136

| RPS | anchor_20 Mean(CoV) | opal_9 Mean | opal_10 Mean | Δ vs anchor_20 |
|-----|---------------------|-------------|--------------|----------------|
| 800 | 800 (0.1%) | 800 | 800 | 0 |
| 1200 | 1182 (0.7%) | 956 | 950 | **-232** |
| 1400 | 1254 (5.1%) | 1049 | 1036 | **-218** |
| 1800 | 1451 (13.7%) | 1184 | 1176 | **-275** |
| 2500 | 1601 (27.0%) | 1227 | 1242 | **-359** |

**Key findings:**
1. **probe_max reduction had no effect** — opal_10 ≈ opal_9 (within noise). The mode-switching jump ratio was not the cause.
2. **The gap-freeze fix itself is the cause of the regression.** Preserving stale goodput_rate from the previous step causes the system to start with a budget locked to previous (lower) capacity, constraining admission even in explore mode.
3. **ER rates remain unreasonably high.** 250/s at 1200 RPS (20.8%) vs anchor_20's 8/s (0.7%). The stale goodput_rate causes the system to oscillate between explore/exploit with inadequate budget at each step's start.

**Decision:** Revert all gap-freeze changes. Try a different approach: increase initial_budget_rate instead.

---

## Iteration 11: Increase initial_budget_rate from 5M to 50M (experiment opal_11)

**Status:** Pending

**Code commit:** TBD

### Change
Revert to pure anchor_20 code (undo gap-freeze, explore formula change, and probe_max change). Then make a single parameter change: `initial_budget_rate` from 5_000_000 to 50_000_000.

This is the simplest possible fix for the slow ramp. The explore mode budget becomes 50M µs/s (~2000 req/s at 25µs cost) instead of 5M (~200 req/s). No other mechanism changes.

### Hypothesis
anchor_20's slow ramp is bottlenecked by the explore mode budget: `initial_budget_rate = 5M → ~200 req/s`. Every load transition starts in explore mode (rejection_ema decays during the gap), and the budget admits only 200 req/s regardless of system capacity.

With 50M: explore mode admits ~2000 req/s from cold start. At every transition:
- 800 RPS: all admitted (800 < 2000) ✓
- 1200 RPS: all admitted (1200 < 2000) ✓
- 1400 RPS: all admitted (1400 < 2000) ✓
- 1800 RPS: most admitted (1800 < 2000) ✓
- 2500 RPS: 2000 admitted initially, some ER, rejection_ema rises, enters exploit at goodput * 1.05

The token bucket's max_burst (50M * 0.005 = 250K µs ≈ 10 requests) naturally limits the instantaneous admission burst, preventing the queue catastrophe that killed opal_1/2.

**Why this won't cause over-admission problems:** The initial_budget_rate only applies in explore mode (rejection_ema < 0.10). Once overloaded, exploit mode takes over at goodput * 1.05 — identical to anchor_20. The burst is limited by max_burst_secs, not by the rate itself.

### Expected outcomes if hypothesis is correct:
1. Near-instant ramp at all load transitions (2000 req/s explore budget vs 200)
2. 800/1200/1400: full admission, matching or exceeding anchor_20
3. 1800/2500: matching anchor_20 (exploit mode is unchanged)
4. Overload stability preserved (exploit mode, rejection_ema, goodput tracking all unchanged)

### Experiment design (opal_11)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_11)

**Status:** Regression ❌ — iterate

**Code commit:** 3df2281c

| RPS | anchor_20 Mean(CoV) | opal_11 Mean | opal_11 ER/s | Δ vs anchor_20 |
|-----|---------------------|--------------|--------------|----------------|
| 800 | 800 (0.1%) | 800 | 0 | 0 |
| 1200 | 1182 (0.7%) | 1188 | 12 | +6 |
| 1400 | 1254 (5.1%) | 1232 | 167 | **-22** |
| 1800 | 1451 (13.7%) | 1400 | 400 | **-51** |
| 2500 | 1601 (27.0%) | 1486 | 1012 | **-115** |

**Key findings:**
1. **50M over-admits at overload.** 50M µs/s → ~2000 req/s in explore. At 2500 RPS, the system floods with 2000 req/s (capacity ~1600) during the first seconds before exploit mode kicks in. The excess 400 req/s causes queue buildup and SLO misses, costing 115 goodput.
2. **Marginal ramp improvement at 1200** (+6). The slow ramp costs ~65 goodput at 1200 (from anchor_20's timeline: drops to 423, takes 10s to reach 1200). Fixing the ramp is worth ~65 at most — not a huge gain.
3. **Regression grows with overload severity.** -22 at 1400, -51 at 1800, -115 at 2500. The over-admission burst is more damaging at higher loads.
4. **The optimal initial_budget_rate should match capacity** (~1600 req/s = ~40M µs/s). Below capacity: instant ramp. At capacity: natural admission limit, no flooding.

**Decision:** Revert. Try initial_budget_rate = 40M (capacity-matched).

---

## Iteration 12: initial_budget_rate = 40M — capacity-matched explore budget (experiment opal_12)

**Status:** Pending

**Code commit:** TBD

### Change
Revert to pure anchor_20 code. Change `initial_budget_rate` from 5_000_000 to 40_000_000.

40M µs/s ÷ 25000 µs/req ≈ 1600 req/s — matching Socialnet ComposePost capacity.

### Hypothesis
initial_budget_rate should match system capacity:
- **Below 40M (anchor_20 at 5M):** Explore budget bottlenecks admission at ~200 req/s → 15s slow ramp.
- **Above 40M (opal_11 at 50M):** Explore budget admits above capacity → initial over-admission burst → SLO misses → goodput loss.
- **At 40M:** Explore budget admits up to ~1600 req/s (capacity). All sub-saturation loads get instant ramp. At overload, the budget naturally limits admission to ~capacity, preventing flooding. Token bucket max_burst (40M * 0.005 = 200K µs ≈ 8 requests) limits instantaneous burst.

**Expected dynamics at each load:**
- 800/1200/1400 RPS: all < 1600, instant admission, stays in explore. No ramp needed.
- 1800 RPS: 1600 admitted in explore, ~200 excess rejected. rejection_ema rises → exploit mode within 1-2s. exploit budget = goodput * 1.05 ≈ 1680 → slightly above capacity. Mild over-admission handled by abort_slo.
- 2500 RPS: 1600 admitted in explore → right at capacity → no flooding (unlike 50M). Exploit kicks in quickly.

### Expected outcomes if hypothesis is correct:
1. 800/1200/1400: instant ramp, matching or exceeding anchor_20
2. 1800: similar to anchor_20 (no flooding from explore, exploit takes over quickly)
3. 2500: similar to anchor_20 (explore budget at capacity → no over-admission)
4. Net: significant improvement at 1200-1400, no regression at 1800-2500

### Experiment design (opal_12)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_12)

**Status:** Mixed — 1400 improved, 1800 regressed

**Code commit:** 37cf682e

| RPS | anchor_20 Mean(CoV) | opal_11 Mean (50M) | opal_12 Mean(CoV) (40M) | Δ vs anchor_20 |
|-----|---------------------|--------------------|-------------------------|----------------|
| 800 | 800 (0.1%) | 800 | 800 (0.0%) | 0 |
| 1200 | 1182 (0.7%) | 1188 | 1188 (0.9%) | +6 |
| 1400 | 1254 (5.1%) | 1232 | **1306 (6.7%)** | **+52** |
| 1800 | 1451 (13.7%) | 1400 | 1282 (28.8%) | **-169** |
| 2500 | 1601 (27.0%) | 1486 | 1487 (40.5%) | **-114** |

**Key findings:**
1. **1400 RPS is the best result in the entire OPAL track** — +52 vs anchor_20. The 40M explore budget allows instant ramp to 1400 without over-admission.
2. **1800 RPS is the worst result** — -169 vs anchor_20, CoV=28.8%. The 40M explore budget (1600 req/s) sits right at the explore/exploit boundary at 1800 RPS. The system bounces between explore (1600 admitted) and exploit (1680 admitted), causing instability.
3. **2500 identical to opal_11** — the explore budget doesn't matter at 2500 since exploit mode dominates.
4. **No single initial_budget_rate works across all loads:**
   - 5M (anchor_20): slow ramp but stable
   - 40M (opal_12): fast ramp, great at 1400, oscillates at 1800
   - 50M (opal_11): fast ramp, over-admits at 1800/2500

---

## Summary and Conclusions (Iterations 9-12: Ramp Acceleration)

### Results table

| Iter | Key change | 800 | 1200 | 1400 | 1800 | 2500 |
|------|-----------|-----|------|------|------|------|
| **anchor_20** | **baseline** | **800** | **1182** | **1254** | **1451** | **1601** |
| opal_9 | gap-freeze + explore×2.0 | 800 | 956 | 1049 | 1184 | 1227 |
| opal_10 | gap-freeze + explore×1.2 | 800 | 950 | 1036 | 1176 | 1242 |
| opal_11 | initial_budget_rate=50M | 800 | 1188 | 1232 | 1400 | 1486 |
| opal_12 | initial_budget_rate=40M | 800 | 1188 | **1306** | 1282 | 1487 |

### What worked
1. **Increasing initial_budget_rate speeds up the sub-saturation ramp.** Both 40M and 50M achieve +6 at 1200 and better at 1400 compared to anchor_20's slow ramp.
2. **40M initial_budget_rate at 1400 is a genuine win** (+52 vs anchor_20). The capacity-matched explore budget provides instant ramp without over-admission.

### What didn't work
1. **Gap-freeze (opal_9/10):** Preserving stale goodput_rate caused regression at ALL overloaded loads, regardless of the explore multiplier. The stale rate locked the system into exploit mode with the wrong budget.
2. **Any initial_budget_rate at 1800 RPS:** 40M causes explore/exploit oscillation (budget at boundary). 50M over-admits. Neither matches anchor_20.
3. **Any initial_budget_rate at 2500 RPS:** Both 40M and 50M show -114 vs anchor_20. The initial over-admission burst in explore mode costs goodput at deep overload.

### Fundamental finding: no fixed explore budget can be optimal across loads

The explore mode budget (`initial_budget_rate`) determines the admission rate during capacity discovery. A fixed value creates a tradeoff:
- **Too low (5M):** Slow ramp at sub-saturation (15-18s). Stable at overload.
- **Too high (50M):** Instant ramp but over-admits at overload transitions.
- **At capacity (40M):** Fast ramp, but oscillates at loads near capacity (explore/exploit boundary).

The root cause: a fixed explore budget cannot adapt to the current load level. It either under-admits at sub-saturation or over-admits at overload. anchor_20's 5M is conservative but safe. Any increase risks overload regression.

### Potential next directions (not yet tried)
1. **Adaptive explore budget:** Instead of a fixed rate, use `max(initial_budget_rate, goodput_rate * (1 + probe_max))` BUT with probe_max tuned to avoid the opal_9 oscillation. The gap-freeze approach failed because it preserved the wrong state; a fresh approach using only the goodput_rate (not gap detection) might work.
2. **Faster tau in explore mode:** Instead of changing the budget, make the goodput EMA converge faster (smaller tau) during explore. The budget stays at 5M initially, but goodput_rate updates faster from incoming completions, speeding the ramp.
3. **Accept the tradeoff:** anchor_20's slow ramp is a benchmark artifact (the load generator pauses between steps). In production, load changes are gradual, and the 5% probe naturally tracks capacity. The slow ramp may not matter in practice.

### Code state
Current code has initial_budget_rate=40M (commit 37cf682e). This should be reverted to anchor_20's 5M for clean state, unless the 1400 improvement (+52) is worth the 1800 regression (-169).

---

## Iteration 13: Faster goodput EMA — tau=0.3s (experiment opal_13)

**Status:** Pending

**Code commit:** TBD

### Change
Revert to pure anchor_20 code. Change `tau` from 1.0 to 0.3 in `PredParams`. Everything else unchanged.

### Hypothesis
The slow ramp is driven by goodput_rate convergence speed. After a load transition, the system enters exploit mode (budget = goodput_rate * 1.05). With tau=1.0s, the EMA takes ~3-5 tau = 3-5s to converge to the new throughput. During this time, goodput_rate (and thus the budget) lags behind actual capacity.

With tau=0.3s: convergence in ~1-1.5s. The budget tracks actual throughput 3x faster. The ramp from the dynamic floor (~2 requests) to capacity takes ~5s instead of ~15s.

**Risk analysis:** Faster tau means more volatile goodput_rate at steady state. But the noise per sample is only 1.7x higher (sqrt(3)). At 1600 completions/s, the averaging effect of many samples should keep the budget stable. The rejection_ema (alpha=0.01, ~100 decisions to converge) provides a separate, slower stability signal.

**Key difference from opal_5's asymmetric tau:** opal_5 used slow decay (tau_down=5s) to prevent goodput_rate from dropping during overload. That INFLATED goodput_rate, causing persistent over-admission. This change uses fast decay (tau=0.3) equally in both directions — tighter at overload, not looser.

### Expected outcomes if hypothesis is correct:
1. 800/1200: faster ramp, +30-60 goodput at 1200 from reduced ramp time
2. 1400: similar or better than anchor_20
3. 1800: similar to anchor_20 (exploit mode behavior unchanged structurally, just faster tracking)
4. 2500: similar to anchor_20
5. CoV may increase slightly (more responsive = more volatile)

### Experiment design (opal_13)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (opal_13)

**Status:** Regression ❌ — iterate

**Code commit:** 2d0268e4

| RPS | anchor_20 Mean(CoV) | opal_13 Mean | opal_13 ER/s | Δ vs anchor_20 |
|-----|---------------------|--------------|--------------|----------------|
| 800 | 800 (0.1%) | 800 | 0.2 | 0 |
| 1200 | 1182 (0.7%) | 1122 | 78 | **-60** |
| 1400 | 1254 (5.1%) | 1209 | 190 | **-45** |
| 1800 | 1451 (13.7%) | 1372 | 428 | **-79** |
| 2500 | 1601 (27.0%) | 1483 | 1017 | **-118** |

**Key findings:**
1. **Faster tau made things worse at every load point.** The more volatile goodput_rate over-reacts to transient latency dips → budget crashes → premature shedding.
2. **Ramp hypothesis disproven at 1200.** Expected the biggest gain here (sub-saturation, ramp-dominated). Instead -60 — the steady-state volatility cost exceeds the ramp speedup benefit.
3. **ER rates elevated across the board.** 78/s at 1200 (6.5% shedding) vs anchor_20's ~8/s (0.7%). The faster EMA triggers budget contraction from transient noise.

**Decision:** Revert tau to 1.0. Try asymmetric tau (fast up, slow down) to get the ramp benefit without the volatility.

---

## Iteration 14: Asymmetric tau — fast rise (0.3s), slow decay (2.0s) (experiment opal_14)

**Status:** Pending

**Code commit:** TBD

### Change
Revert to pure anchor_20 code. In `should_admit`, replace the fixed tau with asymmetric tau:

```rust
let tau = if instant_rate >= state.goodput_rate {
    0.3   // fast rise — rapid discovery
} else {
    2.0   // slow decay — ride out transient dips
};
let alpha = 1.0 - (-elapsed / tau).exp();
state.goodput_rate += alpha * (instant_rate - state.goodput_rate);
```

### Hypothesis
opal_13 showed that symmetric fast tau (0.3s) hurts because transient goodput dips crash the budget. The solution: decouple rise and decay.

**Fast rise (tau=0.3s):** After a load transition, goodput_rate converges up to the new capacity in ~1s. The exploit budget (goodput * 1.05) reaches the right level quickly. Ramp from 200 req/s to 1200 takes ~3-5s instead of 15s.

**Slow decay (tau=2.0s):** When goodput dips transiently (burst of SLO misses, GC pause, etc.), goodput_rate holds steady. The budget doesn't crash. The system rides out the dip and recovers.

**Why this might work where opal_5 failed:** opal_5 paired slow decay with ER-based probe (concurrency limiter). The ER signal was too delayed to compensate for inflated goodput_rate → persistent over-admission. anchor_20 uses budget exhaustion for overload feedback, which is **instantaneous**: when the budget runs dry, the next request is rejected immediately. No delay. So even if goodput_rate stays high, the budget exhaustion prevents over-admission in real-time.

**Risk:** At deep overload, slow decay means goodput_rate stays inflated for ~4-6s (2-3 tau). During this time, exploit budget = inflated_goodput * 1.05 → higher than optimal → more admission → more SLO misses. But the budget is still finite — it runs out faster than it refills, and rejection_ema rises to handle the excess.

### Expected outcomes if hypothesis is correct:
1. 1200: +30-60 vs anchor_20 from faster ramp (fast rise eliminates the 10s ramp period)
2. 1400: similar or better (fast rise + no volatility from slow decay)
3. 1800: similar to anchor_20 (slow decay keeps budget stable, budget exhaustion handles overload)
4. 2500: similar to anchor_20 (same mechanism)
5. No regression at any load point

### Experiment design (opal_14)
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

