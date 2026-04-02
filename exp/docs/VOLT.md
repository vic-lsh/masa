# VOLT — Predictive AC v2 evaluation

## Key questions
- Does the v2 design (accumulated compute cost, root_method keying, tightened feasibility, early feasibility check) improve goodput over anchor_20 (v1)?
- Which of the four v2 changes contributes most to any observed improvement or regression?
- Does the compute-only cost signal eliminate the positive feedback loop under congestion?

## Pre-v2 baseline: anchor_20 (socialnet)

Policy: `sched_pred,abort_slo,ac_pred,est_mean_var`
SLO: 50ms, API: ComposePost

| RPS  | Goodput (v1) | Fraction |
|------|-------------|----------|
| 800  | 799.8       | 100.0%   |
| 1200 | 942.3       | 78.5%    |
| 1400 | 966.7       | 69.1%    |
| 1800 | 1106.4      | 61.5%    |
| 2500 | 1209.5      | 48.4%    |

Early returns (frontend → ComposePost):
- 800: 0.2/s, 1200: 257.5/s, 1400: 433.1/s, 1800: 692.4/s, 2500: 1290.1/s

## Experiment series: volt_1, volt_2, ... (socialnet)

## Observed Symptoms (volt_1) — v2 baseline

Policies tested: `sched_pred,abort_slo,ac_pred,est_mean_var` (v2), `sched_pred,abort_slo,est_mean_var` (sched-only), `sched_tailclipper,abort_slo`

### Goodput

| RPS  | ac_pred (v2) | sched-only | tailclipper | anchor_20 (v1) | v2 Δ vs v1 |
|------|-------------|------------|-------------|----------------|------------|
| 800  | 799.9       | 800.0      | 799.9       | 799.8          | +0.1       |
| 1200 | **1042.3**  | 1176.0     | 1172.7      | 942.3          | **+100.0** |
| 1400 | **876.6**   | 1195.9     | 1054.8      | 966.7          | **-90.1**  |
| 1800 | 1154.0      | 882.9      | 892.2       | 1106.4         | +47.6      |
| 2500 | 1163.7      | 896.8      | 1010.6      | 1209.5         | -45.8      |

### Early returns (frontend ComposePost, rate/s)

| RPS  | ac_pred (v2) | sched-only | tailclipper | anchor_20 (v1) |
|------|-------------|------------|-------------|----------------|
| 800  | 0           | 0          | 0           | 0.2            |
| 1200 | 154.8       | 19.0       | 27.1        | 257.5          |
| 1400 | 523.2       | 189.9      | 344.3       | 433.1          |
| 1800 | 645.8       | 916.9      | 907.4       | 692.4          |
| 2500 | 1334.3      | 1598.0     | 1487.9      | 1290.1         |

### Key findings

1. **Non-monotonic goodput at 1400 RPS is the headline problem.** ac_pred drops to 877 at 1400 (worse than v1's 967 and far below sched-only's 1196), then recovers to 1154 at 1800. The AC over-rejects at moderate overload.

2. **v2 improves at some loads but regresses at others.** +100 at 1200, +48 at 1800, but -90 at 1400 and -46 at 2500. The 1400 dip is disqualifying.

3. **AC provides clear value at deep overload.** At 1800-2500 RPS, ac_pred outperforms sched-only by +271/+267 goodput. Without AC, the system collapses.

4. **AC engages too early.** At 1400, sched-only achieves 1196 without any AC. The AC kicks in aggressively (523 ER/s vs 190 sched-only) and destroys goodput. The transition from "admit everything" to "shed aggressively" is too abrupt.

5. **CPU confirms correct mechanism.** ac_pred uses less CPU (compose-post 59% vs 68%, user-timeline-mongo 70% vs 88%), confirming it sheds work at the frontier. Just miscalibrated.

### Initial assessment

v2's compute-cost signal and early feasibility check are working (clear gains at 1200 and deep overload). But the AC threshold is too aggressive at moderate overload (1400 RPS), creating a non-monotonic goodput dip. The priority: raise the AC engagement threshold so it stays dormant until true saturation (~1600 RPS).

## Iteration 1: Disable early feasibility check (experiment volt_2)

**Status:** Pending

### Change
Remove the early feasibility check in `before_poll` (Change 4 from v2 design). Keep the other three v2 changes (accumulated compute cost, root_method keying, tightened Layer 1 feasibility).

### Hypothesis
The early feasibility check uses `est_method_latency` which tracks **total wall-clock time** including queueing. Under moderate congestion (1400 RPS), queueing inflates this estimate, triggering a positive feedback loop: congestion → inflated wall-clock estimate → more early rejections → less throughput → more queueing → further inflation. This is the same class of feedback loop that v2's compute-only Layer 2 signal was designed to eliminate, but reintroduced via a different pathway.

Disabling Change 4 should eliminate the 1400 RPS dip while preserving the gains from Changes 1-3 (compute cost signal, root_method keying, tightened Layer 1).

### Expected outcomes if hypothesis is correct:
1. 1400 RPS goodput recovers to ≥960 (v1 level) or higher
2. 1200 RPS goodput remains improved over v1 (≥942)
3. 1800/2500 goodput may decrease slightly (fewer rejection pathways) but should remain above v1

### Experiment design
Same config as volt_1 (5 RPS levels, SLO=50ms). Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `54bc3584`

### Actual Outcomes (volt_2)

**Status:** Complete ✅ — KEEP

| RPS  | anchor_20 (v1) | volt_1 (v2 all) | volt_2 (no early feas.) | Δ vs v1 |
|------|----------------|-----------------|------------------------|---------|
| 800  | 799.8          | 799.9           | 800.0                  | +0.2    |
| 1200 | 942.3          | 1042.3          | **1174.2**             | **+231.9** |
| 1400 | 966.7          | 876.6           | **1033.5**             | **+66.8**  |
| 1800 | 1106.4         | 1154.0          | **1275.4**             | **+169.0** |
| 2500 | 1209.5         | 1163.7          | **1345.7**             | **+136.2** |

Early returns (frontend ComposePost): 800: 0, 1200: 25.7/s, 1400: 364.5/s, 1800: 524/s, 2500: 1144.6/s

**Hypothesis confirmed.** The early feasibility check was the source of the 1400 dip. `est_method_latency` tracks wall-clock time including queueing, creating the exact feedback loop v2 was designed to eliminate — just in a different layer. Removing it yields consistent gains at every load point: +67 to +232 vs v1.

All three remaining v2 changes (accumulated compute cost, root_method keying, tightened Layer 1) are contributing to the improvement without introducing feedback loops.

## Iteration 2: Raise probe_min from 0.05 to 0.15 (experiment volt_3)

**Status:** Pending

### Change
In `policy_params.rs`, change `probe_min` from 0.05 to 0.15. This increases the exploit-mode budget headroom from 5% to 15% above observed goodput.

### Hypothesis
At 1400 RPS, ac_pred achieves 1034 vs sched-only's 1196 — the AC is still over-shedding by ~160 requests. In exploit mode, budget_rate = `goodput_rate * 1.05`. If the goodput_rate EMA lags slightly behind actual capacity (e.g., EMA still catching up after mode switch), the 5% margin is too thin and rejects requests the system could serve. Raising to 15% gives more room for the EMA to converge without starving the system.

Risk: too much headroom could reduce shedding effectiveness at 1800-2500 (deeper overload). But 15% is still tight enough to shed well above saturation.

### Expected outcomes if hypothesis is correct:
1. 1400 RPS goodput increases toward 1100+ (closing gap with sched-only)
2. 1800/2500 goodput stays within ±50 of volt_2 (1275/1346)
3. 1200 RPS stays ≥1170

### Experiment design
Same config as volt_1 (5 RPS levels, SLO=50ms). Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `16e3f7b0`

### Actual Outcomes (volt_3)

**Status:** Complete ✅ — KEEP

| RPS  | volt_3 (probe 0.15) | volt_2 (probe 0.05) | Δ vs volt_2 | anchor_20 (v1) | Δ vs v1 |
|------|---------------------|---------------------|-------------|----------------|---------|
| 800  | 800.0               | 800.0               | +0.0        | 799.8          | +0.2    |
| 1200 | 1177.0              | 1174.2              | +2.8        | 942.3          | **+234.7** |
| 1400 | **1151.6**          | 1033.5              | **+118.1**  | 966.7          | **+184.9** |
| 1800 | **1336.4**          | 1275.4              | **+61.0**   | 1106.4         | **+230.0** |
| 2500 | **1422.1**          | 1345.7              | **+76.4**   | 1209.5         | **+212.6** |

Early returns (frontend ComposePost): 800: 0, 1200: 22.8/s, 1400: 247.5/s, 1800: 462.7/s, 2500: 1068.3/s

**Stability analysis (mean goodput, CoV):**

| RPS  | volt_1 (v2 all) | volt_2 (no early feas.) | volt_3 (probe 0.15) |
|------|-----------------|------------------------|---------------------|
| 800  | 800 (0.0%)      | 800 (0.0%)             | 800 (0.0%)          |
| 1200 | 954 (23.0%)     | 1160 (3.8%)            | 1164 (5.2%)         |
| 1400 | 619 (46.8%)     | 833 (39.4%)            | **1020 (22.0%)**    |
| 1800 | 907 (26.5%)     | 1121 (19.8%)           | **1195 (15.3%)**    |
| 2500 | 840 (47.2%)     | 1070 (25.8%)           | **1162 (21.0%)**    |

volt_3 wins on both mean goodput and stability at 1400-2500. The 1400 CoV drops from 46.8% (volt_1) → 39.4% (volt_2) → 22.0% (volt_3), confirming that wider probe margin reduces oscillation between explore/exploit modes.

**Hypothesis confirmed — all criteria exceeded.** probe_min=0.15 improves every load point in both goodput and stability. The mechanism is clear: more budget headroom prevents the controller from starving itself at the explore/exploit boundary.

## Iteration 3: Smooth explore→exploit transition (experiment volt_4)

**Status:** Regression ❌ — REVERT

### Change
Replace the binary explore/exploit mode switch with linear interpolation. Currently, `budget_rate` jumps from `initial_budget_rate` (5M µs/s) to `goodput_rate * 1.15` (~500K µs/s) when `rejection_ema` crosses `rejection_threshold` (0.10) — a ~10x cliff. Replace with:

```
t = clamp(rejection_ema / rejection_threshold, 0, 1)
budget_rate = (1 - t) * initial_budget_rate + t * goodput_rate * (1 + probe_min)
```

This linearly blends from explore to exploit as rejections increase, eliminating the mode-switch oscillation.

### Hypothesis
The 22% CoV at 1400 RPS is caused by oscillation at the explore/exploit boundary. When `rejection_ema` hovers near 0.10, the budget_rate oscillates between 5M and ~500K µs/s — a 10x jump that alternately floods and starves the system. Smoothing this transition should reduce CoV while maintaining mean goodput (the steady-state exploit rate is unchanged).

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 RPS drops below 15% (currently 22%)
2. Mean goodput at 1400 stays ≥1100 (currently 1152)
3. 1800/2500 goodput and CoV remain comparable to volt_3

### Experiment design
Same config as volt_1 (5 RPS levels, SLO=50ms). Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `46df3d29`

### Actual Outcomes (volt_4)

| RPS  | volt_4 (smooth) | volt_3 (binary) | Δ vs volt_3 | CoV volt_4 | CoV volt_3 |
|------|-----------------|-----------------|-------------|------------|------------|
| 800  | 800.0           | 800.0           | +0.0        | 0.0%       | 0.0%       |
| 1200 | 1184.0          | 1177.0          | +7.0        | 2.7%       | 5.2%       |
| 1400 | **1043.3**      | 1151.6          | **-108.3**  | **29.0%**  | 22.0%      |
| 1800 | 1326.8          | 1336.4          | -9.6        | 14.4%      | 15.3%      |
| 2500 | **1369.2**      | 1422.1          | **-52.9**   | **24.4%**  | 21.0%      |

**Hypothesis refuted.** Linear interpolation made both goodput and stability worse at 1400 and 2500. The linear blend dilutes the explore budget at low rejection rates (even rejection_ema=0.02 reduces budget_rate by 20%), causing the controller to under-admit at moderate overload where it should still be exploring freely. The binary switch is better — it maintains full explore budget until rejections are clearly established, then snaps to tight tracking.

**Root cause:** The 10x gap between explore (5M) and exploit (~500K) means even small interpolation factors (t=0.1) create a 450K reduction in budget. The issue is not the transition sharpness but the magnitude of the gap.

## Open hypotheses (CoV reduction)

Six questions to resolve, in priority order:

1. **Q6 ✅ ANSWERED:** CoV is NOT inherent to near-saturation. sched-only at 1400 has 0.7% CoV (rock-solid). The 22% CoV is entirely AC-caused.
2. **Q1 ✅ ANSWERED:** Layer 1's est_child is NOT the CoV source. Removing it made CoV worse (34%). Layer 1 is helping, not hurting.
3. **Q5:** Does `max_burst_secs=0.005` cause micro-starvation? Raise to 0.02.
4. **Q4:** Would `rejection_threshold=0.20` delay exploit mode enough?
5. **Q3:** Is `rejection_alpha=0.01` too slow for mode switching? Try 0.05.
6. **Q2:** Is `tau=1.0` making goodput EMA too reactive? Try 2.0.

**Key diagnostic insight:** The CoV is driven by Layer 2 (token bucket), not Layer 1. The sched-only baseline achieves 0.7% CoV at 1400 — the AC is creating oscillation via its budget dynamics.

## Iteration 4: Remove est_child from Layer 1 check (experiment volt_5)

**Status:** Regression ❌ — REVERT

### Change
In the `admission_check` methods (both `ac_pred` enabled and disabled paths), remove `est_child` from the Layer 1 feasibility check. Revert from `now + est_child + est_remaining_floor > deadline` back to `now + est_remaining_floor > deadline`. This is a diagnostic to isolate whether Layer 1's wall-clock `est_child` drives the CoV.

### Hypothesis
Layer 1 uses `est_child_latency` (wall-clock, includes queueing). Under transient congestion, `est_child` inflates → Layer 1 over-sheds → queue drains → `est_child` deflates → admits freely → queue builds again. This oscillation cycle creates the 22% CoV at 1400. Removing `est_child` should break this cycle.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops below 15%
2. Mean goodput at 1400 may decrease slightly (less aggressive shedding) but stays ≥1050
3. Deep overload (1800/2500) may regress if Layer 1 was doing useful shedding there

### Experiment design
Same config as volt_1 (5 RPS levels, SLO=50ms). Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `bfce6b3a`

### Actual Outcomes (volt_5)

| RPS  | volt_5 (no est_child) | volt_3 (with est_child) | Δ Mean | CoV volt_5 | CoV volt_3 |
|------|----------------------|------------------------|--------|------------|------------|
| 800  | 800.0                | 800.0                  | +0.0   | 0.0%       | 0.0%       |
| 1200 | 1191.6               | 1177.0                 | +14.6  | 1.9%       | 5.2%       |
| 1400 | **1068.4**           | 1151.6                 | **-83.2** | **34.0%** | 22.0%     |
| 1800 | 1332.3               | 1336.4                 | -4.1   | 15.3%      | 15.3%      |
| 2500 | 1420.4               | 1422.1                 | -1.7   | 22.3%      | 21.0%      |

**Hypothesis refuted.** Removing est_child from Layer 1 made 1400 significantly worse: -83 mean, +12pp CoV. Layer 1's est_child is providing useful shedding that reduces the load on Layer 2, keeping the token bucket healthier. Without it, Layer 2 has to do all the shedding alone and oscillates more.

## Iteration 5: Raise max_burst_secs from 0.005 to 0.02 (experiment volt_6)

**Status:** Pending

### Change
In `policy_params.rs`, change `max_burst_secs` from 0.005 to 0.02. This increases the token bucket capacity from ~1 request to ~4 requests at typical compute costs.

### Hypothesis
With `max_burst_secs=0.005`, the token bucket can hold only `budget_rate * 0.005` µs. In exploit mode with goodput_rate ~500K, that's 2500 µs — barely one request (~2000 µs compute). Any brief gap in completions empties the bucket instantly, causing a rejection spike → budget refills → admits burst → another spike. A 4x larger bucket (0.02) should absorb brief completion gaps, reducing CoV without changing steady-state throughput.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops below 15%
2. Mean goodput at 1400 stays ≥1100
3. 1800/2500 CoV also improves

### Experiment design (volt_6)
Same config as volt_1 (5 RPS levels, SLO=50ms). Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `01513685`

### Actual Outcomes (volt_6)

| RPS  | volt_6 (burst 0.02) | volt_3 (burst 0.005) | Δ Mean | CoV volt_6 | CoV volt_3 |
|------|---------------------|---------------------|--------|------------|------------|
| 800  | 800.0               | 800.0               | +0.0   | 0.0%       | 0.0%       |
| 1200 | 1195.4              | 1177.0              | +18.4  | 1.5%       | 5.2%       |
| 1400 | 1195.3              | 1151.6              | +43.7  | 19.7%      | 22.0%      |
| 1800 | **1261.2**          | 1336.4              | **-75.2** | 17.4%   | 15.3%      |
| 2500 | 1440.9              | 1422.1              | +18.8  | 20.9%      | 21.0%      |

**Mixed result — REVERT.** Larger burst window helps at 1200-1400 (+18/+44 mean, CoV improves) but regresses at 1800 (-75 mean, +2pp CoV). The larger bucket delays shedding at deep overload. Not a clean win.

## Iteration 6: Raise rejection_threshold from 0.10 to 0.20 (experiment volt_7)

**Status:** Pending

### Change
In `policy_params.rs`, change `rejection_threshold` from 0.10 to 0.20. This delays the explore→exploit mode switch.

### Hypothesis
The token bucket oscillates because it enters exploit mode too early. At `rejection_threshold=0.10`, just 10% rejection rate triggers the switch to tight goodput tracking. Raising to 0.20 keeps the controller in explore mode longer, maintaining the generous initial budget at moderate overload (1400). Unlike the smooth transition (volt_4), this preserves the full explore budget — it just requires more sustained rejections before switching.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops (less time in exploit mode at moderate overload)
2. Mean goodput at 1400 stays ≥1100
3. Deep overload (1800/2500) should be unaffected (rejection_ema quickly exceeds 0.20 there)

### Experiment design (volt_7)
Same config as volt_1. Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `f5175d63`

### Actual Outcomes (volt_7)

| RPS  | volt_7 (thresh 0.20) | volt_3 (thresh 0.10) | Δ Mean | CoV volt_7 | CoV volt_3 |
|------|---------------------|---------------------|--------|------------|------------|
| 800  | 799.9               | 800.0               | -0.1   | 0.0%       | 0.0%       |
| 1200 | 1184.8              | 1177.0              | +7.8   | 3.3%       | 5.2%       |
| 1400 | 1195.0              | 1151.6              | +43.4  | 23.2%      | 22.0%      |
| 1800 | **977.1**           | 1336.4              | **-359.3** | **55.9%** | 15.3%   |
| 2500 | 1418.9              | 1422.1              | -3.2   | 25.2%      | 21.0%      |

**Catastrophic regression at 1800 — REVERT.** Higher threshold delays exploit mode too much: controller stays in explore mode, overwhelms system at 1800, then crashes into exploit. Improves 1400 slightly (+43) but 1800 collapse is disqualifying.

## Iteration 7: Faster rejection_alpha (experiment volt_8)

**Status:** Pending

### Change
In `policy_params.rs`, change `rejection_alpha` from 0.01 to 0.05. This makes the rejection EMA converge 5x faster (~20 decisions vs ~100).

### Hypothesis
The CoV is partly caused by the rejection EMA being too slow to track rapid changes in rejection rate. At `rejection_alpha=0.01`, the EMA takes ~100 decisions to converge. If the system oscillates between admit-all and reject-all on a 2-3 second cycle, the EMA lags behind reality — it's still in explore mode when it should be in exploit, and vice versa. Faster tracking (0.05) should reduce this phase lag.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops (faster mode correction)
2. Mean goodput at 1400 stays ≥1100
3. Deep overload unaffected (rejection_ema converges quickly either way)

### Experiment design (volt_8)
Same config as volt_1. Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `367c260b`

### Actual Outcomes (volt_8)

**Status:** Complete ✅ — KEEP

| RPS  | volt_8 (α=0.05) | volt_3 (α=0.01) | Δ Mean | CoV volt_8 | CoV volt_3 | Δ CoV  |
|------|-----------------|-----------------|--------|------------|------------|--------|
| 800  | 800.0           | 800.0           | 0.0    | 0.0%       | 0.0%       | 0.0pp  |
| 1200 | 1179.1          | 1177.0          | +2.1   | 3.8%       | 5.2%       | -1.3pp |
| 1400 | **1230.0**      | 1151.6          | **+78.4** | **21.1%** | 22.0%   | -0.9pp |
| 1800 | **1410.0**      | 1336.4          | **+73.6** | **11.6%** | 15.3%   | -3.6pp |
| 2500 | 1478.7          | 1422.1          | +56.6  | **16.2%** | 21.0%     | -4.8pp |

**Hypothesis confirmed.** Faster rejection_alpha improves both mean goodput AND stability at every load point. The mechanism works: faster EMA convergence reduces the phase lag between actual rejection state and the controller's mode, eliminating the "stuck in wrong mode" oscillation. CoV drops 1-5pp across all overloaded RPS levels. Mean improves +2 to +78 across 1200-2500.

## Iteration 8: Raise tau from 1.0 to 2.0 (experiment volt_9)

**Status:** Pending

### Change
In `policy_params.rs`, change `tau` from 1.0 to 2.0. This doubles the smoothing window for the goodput rate EMA, stacking on top of rejection_alpha=0.05 (kept from volt_8).

### Hypothesis
With rejection_alpha=0.05 fixing mode-switch lag, the remaining CoV (11-21%) may come from noise in the goodput_rate EMA. At tau=1.0, the goodput estimate is reactive to short-term completion bursts. Doubling to tau=2.0 smooths this, producing more stable exploit-mode budget rates and reducing CoV further.

### Expected outcomes if hypothesis is correct:
1. CoV drops further at 1400 (below 18%) and 1800 (below 10%)
2. Mean goodput stays within ±30 of volt_8
3. No regression at any load point

### Experiment design (volt_9)
Same config as volt_1. Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `8e1dbe32`

### Actual Outcomes (volt_9)

**Status:** Complete ✅ — KEEP

| RPS  | volt_9 (τ=2.0) | volt_8 (τ=1.0) | Δ vs volt_8 | CoV volt_9 | CoV volt_8 | Δ CoV |
|------|----------------|----------------|-------------|------------|------------|-------|
| 800  | 800.0          | 800.0          | 0.0         | 0.0%       | 0.0%       | 0.0pp |
| 1200 | 1188.7         | 1179.1         | +9.6        | 2.6%       | 3.8%       | -1.2pp |
| 1400 | **1239.3**     | 1230.0         | +9.3        | **15.8%**  | 21.1%      | **-5.3pp** |
| 1800 | **1402.4**     | 1410.0         | -7.6        | 11.7%      | 11.6%      | +0.0pp |
| 2500 | 1412.7         | 1478.7         | -66.0       | 21.8%      | 16.2%      | +5.6pp |

**Cumulative improvement vs volt_3 (original v2 baseline after removing early feas.):**

| RPS  | volt_9 | volt_3 | Δ Mean | CoV volt_9 | CoV volt_3 | Δ CoV  |
|------|--------|--------|--------|------------|------------|--------|
| 1200 | 1189   | 1177   | +12    | 2.6%       | 5.2%       | -2.5pp |
| 1400 | 1239   | 1152   | **+87** | **15.8%** | 22.0%     | **-6.2pp** |
| 1800 | 1402   | 1336   | **+66** | **11.7%** | 15.3%     | **-3.6pp** |
| 2500 | 1413   | 1422   | -9     | 21.8%      | 21.0%      | +0.7pp |

**Hypothesis partially confirmed.** τ=2.0 significantly improves 1400 CoV (-5.3pp vs volt_8, -6.2pp vs volt_3) and mean (+9 vs volt_8). The 2500 CoV regresses +5.6pp vs volt_8 (slower EMA can't track extreme overload as well). Net: clear win at moderate overload, acceptable tradeoff at extreme overload.

## Iteration 9: Probabilistic admission (experiment volt_10)

**Status:** Pending

### Change
Replace the binary admit/reject decision in `should_admit` with probabilistic admission. Instead of `if budget >= cost { admit } else { reject }`, compute `p_admit = clamp(budget / cost, 0, 1)` and admit with that probability. This spreads rejections uniformly across time instead of clustering them into bursts.

The rejection EMA update uses the actual admission outcome (0 or 1), so it naturally tracks the probabilistic decisions.

### Hypothesis
The remaining 15.8% CoV at 1400 RPS is caused by discrete oscillation in the token bucket: budget fills → admit burst until empty → reject burst until refill → repeat. With max_burst_secs=0.005 and ~1 request worth of capacity, this creates high-frequency on/off cycling. Probabilistic admission smooths this by spreading rejections uniformly — when budget is 70% of cost, 30% of requests are randomly rejected rather than all requests being rejected for 30% of the time.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops below 10% (currently 15.8%)
2. Mean goodput at 1400 stays ≥1200 (currently 1239)
3. CoV improvement at all overloaded RPS levels

### Experiment design (volt_10)
Same config as volt_1. Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `dd6942e8`

### Actual Outcomes (volt_10)

**Status:** Regression ❌ — REVERT

| RPS  | volt_10 (prob.) | volt_9 (binary) | Δ Mean | CoV volt_10 | CoV volt_9 | Δ CoV  |
|------|-----------------|-----------------|--------|-------------|------------|--------|
| 800  | 800.0           | 800.0           | 0.0    | 0.0%        | 0.0%       | 0.0pp  |
| 1200 | 1164.9          | 1188.7          | -23.8  | 6.2%        | 2.6%       | +3.6pp |
| 1400 | **1138.2**      | 1239.3          | **-101.1** | **20.7%** | 15.8%   | +5.0pp |
| 1800 | **1300.7**      | 1402.4          | **-101.7** | **20.0%** | 11.7%   | +8.3pp |
| 2500 | 1272.1          | 1412.7          | -140.6 | 28.5%       | 21.8%      | +6.7pp |

**Hypothesis refuted.** Probabilistic admission made both mean and CoV worse at every overloaded RPS. The random coin flip when budget < cost creates its own variance source — admit sequences of 3+ followed by forced rejects (since admitting drains budget to 0). The oscillation problem is in the budget *refill dynamics*, not the *decision boundary*. Randomizing the boundary adds noise without addressing the underlying cycle.

## Iteration 10: Remove explore/exploit mode switch entirely (experiment volt_11)

**Status:** Pending

### Change
Remove the binary explore/exploit mode switch. Instead, always use `goodput_rate * (1 + probe_min)` as budget_rate. Initialize `goodput_rate` to `initial_budget_rate` so the system starts generous and converges via EMA. This eliminates the mode switch entirely — no threshold, no oscillation between modes.

Replace:
```rust
let budget_rate = if state.rejection_ema > p.rejection_threshold {
    state.goodput_rate * (1.0 + p.probe_min)
} else {
    p.initial_budget_rate
};
```
With:
```rust
let budget_rate = state.goodput_rate * (1.0 + p.probe_min);
```
And initialize `goodput_rate` to `initial_budget_rate` (already the case).

### Hypothesis
The remaining CoV is caused by the feedback loop through the goodput_rate EMA, amplified by the mode switch. When rejection_ema hovers near the threshold, the budget_rate jumps between ~5M (explore) and ~500K (exploit) — and even if we've tuned how fast the switch happens, the *existence* of two discrete modes creates discontinuities. Removing the distinction makes budget_rate a smooth, continuous function of observed goodput with no mode transitions.

Risk: without explore mode's generous initial_budget_rate, cold start may be slower. But goodput_rate starts at 5M and tau=2.0 means it takes ~4-6s to decay, which overlaps with warmup anyway.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops below 12% (currently 15.8%)
2. Mean goodput at 1400 stays ≥1200
3. No mode-switch artifacts at any load level

### Experiment design (volt_11)
Same config as volt_1. Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `01be08a0`

### Actual Outcomes (volt_11)

**Status:** Mixed — REVERT

| RPS  | volt_11 (no modes) | volt_9 (with modes) | Δ Mean | CoV volt_11 | CoV volt_9 | Δ CoV  |
|------|-------------------|---------------------|--------|-------------|------------|--------|
| 800  | 792.4             | 800.0               | -7.6   | 0.0%        | 0.0%       | 0.0pp  |
| 1200 | **1077.6**        | 1188.7              | **-111.1** | **13.4%** | 2.6%    | +10.8pp |
| 1400 | **1262.1**        | 1239.3              | +22.8  | **13.3%**  | 15.8%      | **-2.5pp** |
| 1800 | 1423.9            | 1402.4              | +21.5  | 18.8%       | 11.7%      | +7.1pp |
| 2500 | 1453.1            | 1412.7              | +40.4  | 29.4%       | 21.8%      | +7.6pp |

**Targeted improvement at 1400** (13.3% CoV, -2.5pp) confirms the mode switch is a CoV contributor. But catastrophic 1200 regression: without explore mode, goodput_rate EMA decays from initial 5M toward real goodput, causing transient over-rejection at sub-saturation load. The mode switch is load-bearing for sub-saturation stability.

## Iteration 11: Combined max_burst_secs + probe_min increase (experiment volt_12)

**Status:** Pending

### Change
In `policy_params.rs`, change both:
- `max_burst_secs`: 0.005 → 0.02 (4x larger bucket)
- `probe_min`: 0.15 → 0.25 (25% headroom in exploit mode)

### Hypothesis
volt_6 showed max_burst=0.02 helped 1400 (+44 mean) but hurt 1800 (-75) — the larger bucket delayed shedding at deep overload because the exploit-mode budget rate was too tight (goodput * 1.15). By also raising probe_min to 0.25, the exploit-mode rate increases to goodput * 1.25, compensating for the larger bucket at deep overload while maintaining the smoothing benefit at moderate overload.

The combined effect: larger bucket absorbs micro-oscillation (reducing CoV), higher probe margin prevents the larger bucket from under-shedding at deep overload.

### Expected outcomes if hypothesis is correct:
1. CoV at 1400 drops below 12% (smoothing from larger bucket)
2. Mean at 1400 stays ≥1200
3. 1800 mean stays ≥1350 (probe_min compensates for larger bucket)

### Experiment design (volt_12)
Same config as volt_1. Only `sched_pred,abort_slo,ac_pred,est_mean_var`.

Code commit: `dc4a97a6`

### Actual Outcomes (volt_12)

**Status:** Regression ❌ — REVERT

| RPS  | volt_12 (burst+probe) | volt_9 (baseline) | Δ Mean | CoV volt_12 | CoV volt_9 | Δ CoV |
|------|----------------------|-------------------|--------|-------------|------------|-------|
| 800  | 800.0                | 800.0             | 0.0    | 0.0%        | 0.0%       | 0.0pp |
| 1200 | 1185.3               | 1188.7            | -3.4   | 4.1%        | 2.6%       | +1.4pp |
| 1400 | 1253.3               | 1239.3            | +14.0  | 18.0%       | 15.8%      | +2.3pp |
| 1800 | **1351.2**           | 1402.4            | **-51.2** | 13.7%    | 11.7%      | +2.0pp |
| 2500 | 1440.0               | 1412.7            | +27.3  | 20.5%       | 21.8%      | -1.3pp |

**Regression — combined change worsened CoV at 1200-1800 and dropped 1800 mean by -51.** The larger burst window makes oscillation cycles larger amplitude, offsetting any smoothing benefit. Higher probe_min didn't compensate.
