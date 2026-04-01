# ANCHOR — sched_pred,abort_slo,ac_pred,est_mean_var stability

## Key questions
- Can we make ac_pred achieve stable, high goodput under sustained high load instead of oscillating?
- What is the primary feedback loop causing boom-bust cycles, and what parameter changes dampen it?
- Can we achieve monotonically decreasing goodput fraction as RPS increases (eliminate the 1400 < 1800 anomaly)?

## Experiment series: anchor_1, anchor_2, ... (socialnet)

## Observed Symptoms (cp_simple)

Baseline run with policy `sched_pred,abort_slo,ac_pred,est_mean_var`, SLO=50ms, ComposePost API.

### Aggregated goodput

| RPS | Goodput | Fraction | StdDev | Min | Max | Range | CoV |
|-----|---------|----------|--------|-----|------|-------|-----|
| 800 | 800 | 100% | 0.2 | 799.5 | 800.5 | 1.0 | 0.0% |
| 1200 | 1164 | 97% | 47 | 999 | 1201 | 202 | 4.1% |
| 1400 | 1024 | **73%** | 94 | 810 | 1309 | 500 | 9.1% |
| 1800 | 1680 | 93% | 213 | 988 | 1801 | 813 | 12.7% |
| 2500 | 1061 | 42% | 269 | 599 | 2120 | 1521 | 25.4% |

### Key observations

1. **Non-monotonic goodput:** 1400 RPS (73%) is worse than 1800 RPS (93%). The system over-sheds at moderate overload.
2. **Boom-bust at 2500 RPS:** Clear ~10-12 second oscillation cycle — goodput surges to 1300-1560, crashes to 600-700, recovers. Classic AC limit-cycle.
3. **1400 RPS mode-switching:** A sharp crash at t=131s drops goodput from ~1080 to ~870, followed by chaotic oscillations. Recovers near end of interval.
4. **Warmup transient at 1800 RPS:** Takes ~5s to ramp from 952 to steady state ~1680.

### Root cause analysis

The `ac_pred` admission controller has mismatched timescales across its feedback signals:

| Component | Response rate | Role |
|-----------|-------------|------|
| Budget rate adjustment | `adjust_rate=2.0/s` (can ±100%/s) | Primary admission gate |
| Latency estimator (up) | α=0.05 EMA (~20 obs to converge) | Cost estimation |
| Latency estimator (down) | α=0.2 EMA (~5 obs to converge) | Cost estimation |
| ER tracker | α=0.05 EMA | Overload backstop |
| Bottleneck util | Instant update + 2s staleness decay | Utilization signal |

The budget rate controller reacts ~40x faster than the ER tracker, causing overshoot/undershoot cycles. When utilization exceeds `util_target=0.80`, the budget shrinks aggressively. When rejections lower utilization, it expands aggressively. The ER backstop (α=0.05) is too slow to dampen these swings.

## Open hypotheses

1. **Reduce adjust_rate** (2.0 → 0.5): Directly dampen the fastest feedback loop
2. **Increase er_alpha** (0.05 → 0.2): Speed up the overload backstop to match budget rate timescale
3. **Lower util_target** (0.80 → 0.70): More headroom before corrections kick in
4. **Add rate-of-change smoothing** to budget adjustments (EMA on the rate itself)
5. **Fix estimator asymmetry** (α_down=0.2 deflates too fast after rejections lower load)

---

## Iteration 1: Dampen budget rate adjustment (experiment anchor_1)

**Status:** Code committed, experiment pending

### Change
Reduce `adjust_rate` from 2.0 to 0.5 in `policy_params.rs`. This limits budget rate changes to ±50%/s instead of ±200%/s, directly dampening the primary oscillation driver.

### Hypothesis
The boom-bust cycle is driven by the budget rate controller overreacting to utilization spikes. At 2.0/s, a 500ms spike above util_target=0.80 cuts the budget by ~63%. A subsequent 500ms of underload then doubles it back. This whiplash is the primary oscillation mechanism.

At 0.5/s, the same 500ms spike only cuts the budget by ~22% — a gentler correction that should converge rather than oscillate. The tradeoff is slower response to genuine load changes, but since we're targeting stability at sustained load, this is acceptable.

### Expected outcomes if hypothesis is correct:
1. Reduced CoV at all overloaded RPS levels (1200+)
2. Monotonic goodput fraction (1400 should no longer be worse than 1800)
3. Elimination of boom-bust cycles at 2500 RPS (no more 10-12s oscillation period)
4. Possibly slower warmup transient (acceptable tradeoff)

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each). This directly tests whether the oscillation amplitude decreases at each load point.

### Actual Outcomes (anchor_1)

**Status:** Regression ❌ — reverted

| RPS | Baseline Goodput (CoV) | anchor_1 Goodput (CoV) | Delta |
|-----|----------------------|----------------------|-------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1164 (4.1%) | 1195 (1.4%) | +31 |
| 1400 | 1024 (9.1%) | 1188 (18.2%) | +164 |
| 1800 | 1680 (12.7%) | **1178 (18.3%)** | **-502** |
| 2500 | 1061 (25.4%) | 1331 (22.7%) | +270 |

**Key findings:**
- Reducing adjust_rate from 2.0 to 0.5 slowed oscillation *frequency* but not *amplitude*. The system still boom-busts; it just does it more slowly.
- **Catastrophic regression at 1800 RPS (-502, -30%).** The slower controller oscillates around a worse operating point. The baseline's fast adjust_rate was actually good at finding the right admission level at this load.
- Improved 1200 (+31, CoV 1.4%) and 2500 (+270, CoV 22.7%), but the 1800 regression makes this a net loss.
- The oscillation is a **limit cycle**, not a damping problem — simply reducing controller gain does not eliminate it.

**Root cause insight:** The AC doesn't know its own rejections cause the utilization drop. It sees low util → admits more → overload → rejects → low util → cycle repeats. The ER backstop (α=0.05) is too slow to signal "we're actively shedding" before the budget reopens.

**Decision:** Revert code change, keep insight.

---

## Iteration 2: Speed up ER backstop (experiment anchor_2)

**Status:** Pending

### Change
Revert `adjust_rate` back to 2.0. Increase `er_alpha` from 0.05 to 0.3 in `policy_params.rs`. This makes the ER tracker respond 6x faster to early return events.

### Hypothesis
The limit cycle is driven by the AC reopening admission after a rejection wave because the ER tracker (α=0.05) is too slow to maintain high pseudo_util during the "queue drain" phase. By the time the ER signal rises, the AC has already re-admitted a burst.

At α=0.3, the ER tracker reaches pseudo_util saturation (ema ≥ er_threshold=0.2) after ~2-3 ER events instead of ~5-6. This should keep the AC constrained during rejection-induced low-utilization periods, preventing the premature reopening that triggers the next boom cycle.

The fast adjust_rate=2.0 is actually beneficial (it found the right level at 1800 RPS in baseline), so we keep it — we just need the ER backstop to be fast enough to prevent overshoot on the rebound.

### Expected outcomes if hypothesis is correct:
1. Preserved goodput at 1800 RPS (near baseline ~1680)
2. Reduced oscillation amplitude at all overloaded RPS levels
3. Faster convergence after load transitions (ER signal catches up to AC decisions)
4. Possible slight reduction in mean goodput at moderate overload (1200-1400) if ER backstop is too aggressive

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).
