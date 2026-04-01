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

**Status:** Pending

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
