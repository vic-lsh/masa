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

### Actual Outcomes (anchor_2)

**Status:** Regression ❌ — reverted

| RPS | Baseline (CoV) | anchor_2 (CoV) | Delta vs Baseline |
|-----|---------------|---------------|-------------------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1164 (4.1%) | 1190 (1.4%) | +26 |
| 1400 | 1024 (9.1%) | 1086 (18.6%) | +62 |
| 1800 | 1680 (12.7%) | **971 (19.1%)** | **-709** |
| 2500 | 1061 (25.4%) | 1028 (31.4%) | -33 |

**Key findings:**
- Faster ER backstop (α=0.3) causes **over-shedding**, making oscillations worse at every RPS ≥1400.
- **Catastrophic regression at 1800 RPS (-709).** Worst 1800 result across all experiments. ER sheds too aggressively during transient spikes → utilization craters → AC reopens violently → deeper boom-bust.
- ER rates confirm over-shedding: 206/s at 1400, 641/s at 1800, 1273/s at 2500.
- The faster ER backstop makes the control loop *more twitchy*, not less. It sheds harder during busts and creates even lower utilization readings, which cause the AC to reopen even more aggressively.
- 1200 RPS still a bright spot (CoV 1.4%) — at moderate overload, quick ER helps. But this advantage disappears at higher loads.

**Root cause insight:** Neither adjust_rate (iter 1) nor er_alpha (iter 2) can fix the oscillation because they both operate within the same flawed feedback loop. The core issue is *symmetric* adjustment: the AC opens as aggressively as it closes. After a rejection wave, low utilization triggers an equally aggressive reopen, flooding the system.

**Decision:** Revert code change. Next iteration: structural fix — asymmetric adjust rates.

---

## Iteration 3: Asymmetric AC adjustment — slow reopen, fast close (experiment anchor_3)

**Status:** Pending

### Change
Split `adjust_rate` into two rates in `policy_params.rs` and the AC logic in `predictive.rs`:
- `adjust_rate_down = 2.0` (close fast on overload — preserves baseline 1800 behavior)
- `adjust_rate_up = 0.5` (reopen slowly after rejection wave — prevents post-bust flood)

This requires modifying the budget rate adjustment code in `predictive.rs` to use the appropriate rate depending on direction.

### Hypothesis
The boom-bust cycle has an asymmetric cause: overload detection needs to be fast (to prevent SLO violations), but recovery should be gradual (to prevent the flood that triggers the next crash). The baseline uses symmetric 2.0 for both — which is why it closes well but reopens too aggressively.

With asymmetric rates (2.0 down / 0.5 up), after a rejection wave the AC ramps budget back up 4x slower than it ramps down. This prevents the "slam the door then throw it open" pattern that drives the limit cycle. The system should converge to a steady admission rate rather than oscillating between "full admit" and "full reject."

### Expected outcomes if hypothesis is correct:
1. Preserved goodput at 1800 RPS (fast close rate matches baseline behavior)
2. Reduced oscillation amplitude at all overloaded RPS (slow reopen prevents post-bust flood)
3. Slightly slower warmup transient at load transitions (acceptable tradeoff)
4. Possible slight mean goodput reduction at 2500 if slow reopen undershoots optimal admission rate

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (anchor_3)

**Status:** Keep ✅ — best AC result so far

| RPS | Baseline (CoV) | anchor_3 (CoV) | Delta vs Baseline |
|-----|---------------|---------------|-------------------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1164 (4.1%) | 1187 (3.2%) | +23 |
| 1400 | 1024 (9.1%) | **1300 (5.5%)** | **+276** |
| 1800 | 1680 (12.7%) | 1440 (10.6%) | -240 |
| 2500 | 1061 (25.4%) | **1503 (19.6%)** | **+442** |

**Key findings:**
- **1400 RPS is the star:** +276 goodput, CoV halved (9.1% → 5.5%), no deep collapses. Range 1094-1376.
- **2500 RPS dramatically improved:** +442 goodput, CoV from 25.4% to 19.6%. Still has ~8-10s boom-bust cycles (1020-1700) but much higher floor.
- **1800 RPS still regressed (-240):** Fast close works, but slow reopen (0.5) creates 6-10s recovery troughs at ~1150.
- **ER pattern is healthy:** Rejections at frontend only. Rates: 1200: 1.1%, 1400: 7.1%, 1800: 19.9%, 2500: 39.8%.

**Decision:** Keep. Iterate on up rate.

---

## Iteration 4: Faster reopen rate (experiment anchor_4)

**Status:** Pending

### Change
Increase `adjust_rate_up` from 0.5 to 1.0 in `policy_params.rs`. Asymmetry goes from 4:1 to 2:1 (down=2.0, up=1.0).

### Hypothesis
The 1800 RPS regression is caused by slow recovery after burst suppression. At 0.5, the AC takes ~2s to double the budget — too slow for a load the system can mostly handle. At 1.0, recovery takes ~1s, matching the system's natural recovery time.

The 2:1 asymmetry should still prevent post-bust flooding while allowing faster recovery at moderate overload.

### Expected outcomes if hypothesis is correct:
1. 1800 RPS recovers toward baseline (~1600+), shorter recovery troughs
2. 1400 and 2500 maintain their gains (still asymmetric, just less so)
3. CoV improves at 1800

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (anchor_4)

**Status:** Regression ❌ — reverted

| RPS | anchor_3 (CoV) | anchor_4 (CoV) | Delta vs anchor_3 |
|-----|---------------|---------------|-------------------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1187 (3.2%) | 1188 (2.0%) | +1 |
| 1400 | 1300 (5.5%) | 1173 (9.5%) | **-127** |
| 1800 | 1440 (10.6%) | 1199 (12.6%) | **-241** |
| 2500 | 1503 (19.6%) | 1246 (19.7%) | **-257** |

**Key findings:**
- 2:1 asymmetry strictly worse than 4:1 at all overloaded RPS. Faster upward adjustment overshoots.
- The issue is NOT reopen speed — it's that AC engages too early at 1800 (util_target=0.80 triggers on handleable load).

**Decision:** Revert. anchor_3 remains best. Next: raise util_target.

---

## Iteration 5: Raise util_target from 0.80 to 0.90 (experiment anchor_5)

**Status:** Pending

### Change
Revert `adjust_rate_up` back to 0.5 (restore anchor_3 state). Increase `util_target` from 0.80 to 0.90 in `policy_params.rs`.

### Hypothesis
The 1800 RPS regression (-240 vs baseline) exists because the AC engages too early. At util_target=0.80, the system starts shedding at 1800 even though it can handle most of that load under SLO (baseline achieves 1680 = 93%).

At util_target=0.90, AC only engages when utilization exceeds 90%. At 1800 RPS this shouldn't happen (system copes). At 2500 RPS (deep overload), util still exceeds 0.90, so AC still intervenes.

Combined with 4:1 asymmetry (down=2.0, up=0.5), this should let 1800 run without AC interference while maintaining benefits at deep overload.

### Expected outcomes if hypothesis is correct:
1. 1800 RPS recovers to near-baseline (~1600+)
2. 2500 RPS maintains anchor_3 gains (~1500)
3. 1400 RPS maintains or improves (less unnecessary shedding)
4. Possible brief overload transient at activation threshold

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (anchor_5)

**Status:** Regression ❌ — reverted

| RPS | anchor_3 (CoV) | anchor_5 (CoV) | Delta vs anchor_3 |
|-----|---------------|---------------|-------------------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 |
| 1200 | 1187 (3.2%) | 1190 (1.2%) | +3 |
| 1400 | 1300 (5.5%) | 1219 (6.6%) | **-81** |
| 1800 | 1440 (10.6%) | 1269 (11.9%) | **-171** |
| 2500 | 1503 (19.6%) | 1311 (18.3%) | **-192** |

**Key findings:**
- util_target=0.90 is too high. AC engages too late → overload builds → deep crashes (min 992 at 1800). The system's saturation is ~1400-1500 RPS; 0.90 leaves no headroom.
- 1800 got WORSE, not better (-171 vs anchor_3, -411 vs baseline). The hypothesis was wrong — baseline doesn't achieve 1680 because AC doesn't engage; baseline achieves 1680 because without AC, there's no oscillation feedback loop to create boom-bust cycles.
- All overloaded RPS regressed from anchor_3.

**Root cause insight:** The problem isn't WHEN the AC engages (threshold) but HOW HARD it corrects (gain). At 1800, util hovers ~0.82-0.85, barely above target=0.80. But the AC applies the same correction rate (adjust_rate_down=2.0) as it would at util=0.95. It over-corrects at moderate overload.

**Decision:** Revert. util_target=0.80 is correct. Next: proportional control.

---

## Iteration 6: Proportional AC control (experiment anchor_6)

**Status:** Pending

### Change
Revert `util_target` back to 0.80. Modify the budget rate adjustment in `predictive.rs` to scale the adjustment rate proportionally to the distance from `util_target`:

```rust
if effective_util > util_target {
    let excess = (effective_util - util_target) / (1.0 - util_target);  // 0 at target, 1 at util=1.0
    budget_rate *= 1.0 - adjust_rate_down * excess * elapsed;
} else {
    let slack = (util_target - effective_util) / util_target;  // 0 at target, 1 at util=0
    budget_rate *= 1.0 + adjust_rate_up * slack * elapsed;
}
```

At util=0.82, target=0.80: excess = 0.02/0.20 = 0.10 → reduce at 10% of max rate
At util=0.95, target=0.80: excess = 0.15/0.20 = 0.75 → reduce at 75% of max rate
At util=0.60, target=0.80: slack = 0.20/0.80 = 0.25 → increase at 25% of max rate

### Hypothesis
The boom-bust cycle at 1800 RPS is driven by the AC applying the same correction strength at util=0.82 as at util=0.95. At 1800 (moderate overload), util barely exceeds the target, but the AC slams the budget down at full rate (2.0/s), over-shedding before the signal can self-correct.

With proportional control, moderate overload (util=0.82) gets a gentle correction (10% of max), while deep overload (util=0.95) gets a strong correction (75% of max). This should:
- Let 1800 RPS self-regulate with minimal AC interference (gentle nudges instead of slams)
- Still aggressively protect at 2500 RPS where util is far above target
- Maintain the 4:1 asymmetry for reopen damping

### Expected outcomes if hypothesis is correct:
1. 1800 RPS significantly recovers toward baseline (1600+), as proportional control avoids over-correction
2. 1400 and 2500 maintain or improve from anchor_3 (proportional is gentler overall but still responsive at deep overload)
3. Reduced CoV at 1800 (less violent swings from gentler corrections)
4. Possible slight regression at 2500 if proportional response is too gentle at high util

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (anchor_6)

**Status:** Regression ❌ — reverted

| RPS | anchor_3 Mean (CoV) | anchor_6 Mean (CoV) | Delta Mean | Delta CoV |
|-----|---------------------|---------------------|------------|-----------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 | 0 |
| 1200 | 1187 (3.2%) | 1185 (1.8%) | -2 | **-1.4pp** |
| 1400 | 1300 (5.5%) | 1223 (7.9%) | **-77** | +2.4pp |
| 1800 | 1440 (10.6%) | 1276 (11.8%) | **-164** | +1.2pp |
| 2500 | 1503 (19.6%) | 1330 (21.6%) | **-173** | +2.0pp |

**Key findings:**
- Proportional control improved 1200 CoV (1.8%) but regressed mean+CoV at 1400-2500.
- Too gentle near threshold allows queue buildup before correcting.

**Decision:** Revert. Next: cooldown hold.

---

## Iteration 7: Cooldown hold after AC correction (experiment anchor_7)

**Status:** Pending

### Change
Add a 1-second cooldown hold. When AC transitions from reducing (util > target) to would-increase (util < target), hold budget_rate steady for 1s before increasing. New fields in BudgetState: `was_reducing`, `cooldown_until`. New param: `cooldown_secs: 1.0`.

Base: anchor_3 state (adjust_rate_down=2.0, adjust_rate_up=0.5, util_target=0.80).

### Hypothesis
The limit cycle's critical moment is the transition from "correcting" to "recovering." The AC immediately starts ramping up when util drops below 0.80, but the system hasn't settled yet. A 1s hold gives in-flight requests time to complete and queues to drain before admission increases.

### Expected outcomes if hypothesis is correct:
1. Reduced CoV at all overloaded RPS (the hold breaks the oscillation cycle)
2. Maintained or slightly reduced mean goodput (conservative hold time)
3. Favorable variance-mean tradeoff

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).

### Actual Outcomes (anchor_7)

**Status:** Neutral — reverted (negligible effect)

| RPS | anchor_3 Mean(CoV) | anchor_7 Mean(CoV) | Δ Mean | Δ CoV |
|-----|---------------------|---------------------|--------|-------|
| 800 | 800 (0.0%) | 800 (0.0%) | 0 | 0 |
| 1200 | 1180 (3.2%) | 1186 (1.8%) | +6 | -1.4pp |
| 1400 | 1246 (5.5%) | 1287 (6.7%) | +41 | +1.2pp |
| 1800 | 1336 (10.6%) | 1331 (11.1%) | -5 | +0.5pp |
| 2500 | 1345 (19.7%) | 1333 (20.9%) | -12 | +1.2pp |

**Key findings:**
- Cooldown is essentially neutral. Slight CoV improvement at 1200 (-1.4pp), slight regression everywhere else.
- The oscillation is NOT driven by the immediate rebound transition — the feedback loop operates at a deeper level.

**Root cause insight:** Investigating the token bucket: `max_burst_secs=0.005` with `budget_rate=5M µs/s` gives a burst buffer of only 25,000 µs = ~1 request at typical 20-30ms child costs. This makes admission binary — any momentary rate mismatch causes immediate rejection. The oscillation may be driven by this razor-thin buffer: when budget runs dry, ALL requests are rejected until the next refill tick, creating a bursty admit/reject pattern.

**Decision:** Revert cooldown (not worth the complexity). Next: increase burst buffer.

---

## Iteration 8: Increase burst buffer (experiment anchor_8)

**Status:** Pending

### Change
Revert cooldown. Increase `max_burst_secs` from 0.005 to 0.1 in `policy_params.rs`. This increases the token bucket capacity from ~1 request to ~5 requests at typical 20-30ms child costs.

Base: anchor_3 state (adjust_rate_down=2.0, adjust_rate_up=0.5, util_target=0.80).

### Hypothesis
With a ~1-request burst buffer, the AC operates as a binary gate: budget full → admit, budget empty → reject ALL until next refill. This creates bursty admit/reject cycles even when the rate is close to optimal — a small rate undershoot causes complete rejection until the next tick.

With a ~5-request buffer (max_burst_secs=0.1), the AC can absorb short-term mismatches. If the rate is slightly below the incoming load, the buffer drains gradually rather than hitting zero immediately. This transforms the AC from a binary gate into a smoother metering system:
- Budget rate slightly too low → buffer slowly drains → occasional rejections → gradual rate increase
- Budget rate slightly too high → buffer slowly fills → steady admission → gradual rate decrease

### Expected outcomes if hypothesis is correct:
1. Reduced CoV at all overloaded RPS (smoother admission instead of binary gate)
2. Maintained or improved mean goodput (fewer wasted rejections from binary gating)
3. Possible regression at 2500 if larger buffer allows too many requests through during overload spikes

### Experiment design
Same config as cp_simple (800, 1200, 1400, 1800, 2500 RPS, SLO=50ms, 60s each).
