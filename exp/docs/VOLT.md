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
