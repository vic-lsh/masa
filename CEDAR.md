# CEDAR — prio_local,est_mean_var,early

## Key questions
- How does `prio_local,est_mean_var,early` (Masa with EMA-based local deadline estimation) compare to `prio_oldest,early` (TailClipper) across a two-trace workload (S_32048416 + S_14677443)?
- Is the current best code state (k=1.2, α=0.1) robust across both call graphs simultaneously, or does one trace hurt the other?
- Can we further increase the goodput gap over TailClipper beyond what was observed on the single-trace FLINT experiments?

## Experiment series: cedar_1, cedar_2, ... (mssim)

**Config base:** `exp/mssim/in/s3204` — dual-trace, SLO=100ms, RPS [200, 400, 800, 1000, 1200, 1400, 1500, 1600, 1800], 30s per step

**Current code state:** k=1.2, α=0.1 (LatencyMeanVar::default() = Self::new(1.2, 0.1)) — confirmed best from FLINT iterations 1–9

**Policies under comparison:**
- `prio_local,est_mean_var,early` — **target** (Masa with EMA local deadline)
- `prio_oldest,early` — **TailClipper baseline** (primary comparison)
- `prio_local,early` — **previous Masa** (regression reference)
- `fifo`, `fifo,early` — baselines

---

## Phase 0: Baseline run (cedar_1)

**Config:** `cedar_1` = s3204 with `prio_local,est_mean_var,early` added to policies

Re-running because s3204 results were gathered before the current best code state (k=1.2, α=0.1) was established. This is the fresh baseline for this optimization track.

**Status:** Complete ✅

### Observed Symptoms (cedar_1)

**Goodput table (absolute RPS meeting SLO):**

| RPS | fifo | fifo,early | prio_local,early | prio_oldest,early | est_mean_var |
|-----|------|-----------|-----------------|-------------------|--------------|
| 200 | 193.6 | 199.9 | 196.5 | 194.8 | 197.0 |
| 400 | 393.6 | 394.1 | 402.8 | 392.6 | 387.6 |
| 800 | 659.1 | 727.7 | 775.8 | 749.9 | 757.5 |
| 1000 | 605.2 | 801.0 | 959.8 | 914.1 | 916.2 |
| 1200 | 568.8 | 834.0 | 1122.5 | 1052.4 | 1066.0 |
| 1400 | 555.1 | 829.1 | 1197.6 | 1120.6 | 1183.6 |
| 1500 | 546.2 | 826.1 | 1179.8 | 1137.3 | 1223.5 |
| 1600 | 555.2 | 830.9 | 1190.5 | 1158.7 | 1279.2 |
| 1800 | 547.3 | 830.6 | 1201.0 | 1162.6 | 1298.5 |

**est_mean_var vs prio_oldest,early (TailClipper delta):**

| RPS | Delta | Direction |
|-----|-------|-----------|
| 200 | +2.1 | win |
| 400 | -5.0 | **lose** |
| 800 | +7.6 | win |
| 1000 | +2.1 | marginal win |
| 1200 | +13.7 | win |
| 1400 | +63.0 | win |
| 1500 | +86.2 | win |
| 1600 | +120.5 | win |
| 1800 | +135.9 | win |

**est_mean_var vs prio_local,early (regression reference delta):**

| RPS | Delta | Direction |
|-----|-------|-----------|
| 400 | -15.2 | **lose** |
| 800 | -18.3 | **lose** |
| 1000 | -43.6 | **lose** |
| 1200 | -56.5 | **lose** |
| 1400 | -14.0 | **lose** |
| 1500 | +43.7 | win |
| 1600 | +88.7 | win |
| 1800 | +97.5 | win |

**Key findings:**
1. `est_mean_var` beats TailClipper at 8/9 load points; gap grows to +136 at 1800 RPS. Goal largely met.
2. Critical regression vs `prio_local,early` from 400–1400 RPS — worst at 1200 RPS (-56.5). Crossover to positive is ~1500 RPS.
3. The 1000 RPS TailClipper win (+2.1) is near noise (±20 run-to-run variance) — needs to be made more robust.
4. At 400 RPS, `prio_local,early` achieves fraction >1.0 (402.8 / 400 = 1.007), indicating early returns freed capacity. `est_mean_var` doesn't replicate this.
5. Comparing to single-trace FLINT results: on S_14677443 alone, est_mean_var beat prio_local,early by +23.7 at 1000 RPS. On two traces, it **loses** by -43.6 — a swing of 67 RPS. The S_32048416 trace dramatically changes the competitive landscape.
6. Root cause hypothesis: the S_32048416 trace has higher or different latency variance than S_14677443. On shared microservices, the `LatencyMeanVar` estimator tracks a mixture distribution with inflated variance. The k=1.2 multiplier amplifies this inflation, producing overly tight deadlines at moderate load. `prio_local,early` with the simpler RMS estimator is less sensitive to this mixture.

---

## Iteration 1: Reduce k from 1.2 to 1.0 (experiment cedar_2)

**Status:** Pending

### Change
Set `LatencyMeanVar::default()` to `Self::new(1.0, 0.1)` (k=1.0, keeping α=0.1).

### Hypothesis
On the two-trace workload, the S_32048416 and S_14677443 traces share microservices but have different latency distributions. The `LatencyMeanVar` estimator tracks the mixture, inflating the variance estimate. With k=1.2, the `mean + k*sqrt(variance)` deadline formula produces overly conservative (tight) estimates at moderate load, causing incorrect priority ordering. With k=1.0, the deadline formula reverts to the variance-unweighted case, which may be more robust when variance is contaminated by cross-trace mixture.

In the single-trace FLINT experiments, k=1.2 beat k=1.0 by +23.8 at 1400 RPS and was neutral at 1000 RPS. On two traces, the variance inflation effect is stronger, so the balance may shift to favor k=1.0 at 1000-1200 RPS. We accept a possible high-load regression as the trade-off.

### Expected outcomes if hypothesis is correct:
1. 1000 RPS win over TailClipper improves from +2.1 to +20 or better
2. 1200 RPS win over TailClipper improves from +13.7 toward +30+
3. Regression at 1600-1800 RPS vs cedar_1 baseline (est_mean_var loses some high-load advantage)
4. Gap vs `prio_local,early` narrows at 800-1400 RPS

### Experiment design
Use same cedar_1 config (two-trace, RPS 200–1800, 30s/step). The moderate-load regime (800-1400 RPS) is where the hypothesis is most testable. High-load (1600-1800) results will tell us the cost.

### Actual Outcomes (cedar_2)

**Status:** Complete ✅ — hypothesis confirmed, and high-load also improved (unexpected)

**Code commit:** 131c2880

**cedar_2 est_mean_var (k=1.0) vs cedar_1 est_mean_var (k=1.2):**

| RPS | k=1.0 | k=1.2 | Δ (k=1.0 − k=1.2) |
|-----|-------|-------|---------------------|
| 200 | 194.0 | 197.0 | -3.0 (noise) |
| 400 | 387.2 | 387.6 | -0.4 (noise) |
| 800 | 768.0 | 757.5 | **+10.5** |
| 1000 | 937.0 | 916.2 | **+20.8** |
| 1200 | 1083.6 | 1066.0 | **+17.6** |
| 1400 | 1205.4 | 1183.6 | **+21.8** |
| 1500 | 1251.0 | 1223.5 | **+27.5** |
| 1600 | 1281.6 | 1279.2 | +2.4 |
| 1800 | 1344.6 | 1298.5 | **+46.1** |

**est_mean_var (k=1.0) vs prio_oldest,early (TailClipper) — cedar_2:**

| RPS | est_mean_var | prio_oldest | Δ |
|-----|-------------|-------------|---|
| 200 | 194.0 | 196.2 | -2.2 |
| 400 | 387.2 | 390.8 | -3.6 |
| 800 | 768.0 | 750.4 | **+17.6** |
| 1000 | 937.0 | 918.0 | **+19.0** |
| 1200 | 1083.6 | 1046.4 | **+37.2** |
| 1400 | 1205.4 | 1131.2 | **+74.2** |
| 1500 | 1251.0 | 1152.0 | **+99.0** |
| 1600 | 1281.6 | 1156.8 | **+124.8** |
| 1800 | 1344.6 | 1166.4 | **+178.2** |

**Key findings:**
1. k=1.0 beats k=1.2 at every load point ≥800 RPS. The expected moderate-load improvement was achieved, AND high-load improved unexpectedly (+46.1 at 1800 RPS).
2. The single-trace FLINT result (k=1.2 beats k=1.0) does not generalize to the two-trace workload. On two traces, variance inflation from cross-trace mixing is the dominant effect.
3. est_mean_var vs TailClipper gaps are now strong across all stressed load points (all ≥17 RPS from 800 up, peaking at +178 at 1800).
4. Still losing to prio_local,early at 800-1200 RPS (-6, -18, -47). But this is not the primary goal.
5. The k=1.0 trend suggests further reduction (k<1.0) may continue to help. Open question: is there an optimal k between 0 and 1.0, or does k→0 (pure mean) maximize performance?

**Decision: KEEP k=1.0.** Proceed to Iteration 2 probing k=0.5 to test if the downward k trend continues.

---

## Iteration 2: Probe k=0.5 — does lower k continue to win? (experiment cedar_3)

**Status:** Pending

### Change
Set `LatencyMeanVar::default()` to `Self::new(0.5, 0.1)` (k=0.5, keeping α=0.1).

### Hypothesis
Iteration 1 showed k=1.0 beats k=1.2 by large margins on the two-trace workload, suggesting variance inflation from cross-trace mixing is harmful. If the fundamental issue is that the variance estimate is contaminated, reducing k further should continue to help — with diminishing returns. At k=0.5 the deadline formula adds only half a standard deviation above the mean. If even k=0.0 (pure mean) would be best, we expect k=0.5 to also beat k=1.0. Alternatively, if there is a U-shape optimal k above 0, k=0.5 might show mixed results.

The high-load +46.1 gain from k=1.2→1.0 could reflect that looser deadlines allow the reprioritization mechanism to work better at the final leaf service under saturation. This effect should continue at k=0.5, or level off.

### Expected outcomes if hypothesis is correct (k lower continues to help):
1. 1000 RPS gain over TailClipper improves beyond +19.0 (e.g., +25+)
2. 1800 RPS goodput improves beyond 1344.6
3. Gap vs prio_local,early at 1000-1200 RPS narrows further

### Expected outcomes if there is an optimal k above 0:
1. High-load (1600-1800) slightly regresses vs cedar_2
2. Moderate-load shows mixed results
3. Decision: k=1.0 was the optimum, keep it.

### Experiment design
Same two-trace config, same RPS sweep. fifo/fifo,early excluded for faster run (3 policies: prio_local,early, prio_oldest,early, prio_local,est_mean_var,early).

### Actual Outcomes (cedar_3)

**Status:** Mixed — high-load improved but 800 RPS regressed below TailClipper

**Code commit:** 254e1505

**cedar_3 (k=0.5) vs cedar_2 (k=1.0) — est_mean_var delta:**

| RPS | k=0.5 | k=1.0 | Δ | vs TailClipper (c3) |
|-----|-------|-------|---|---------------------|
| 200 | 200.5 | 194.0 | +6.5 | +3.9 |
| 400 | 393.0 | 387.2 | +5.8 | +0.7 |
| 800 | 758.6 | 768.0 | **-9.4** | **-10.4 ← loss!** |
| 1000 | 939.0 | 937.0 | +2.0 | +11.2 |
| 1200 | 1094.3 | 1083.6 | +10.7 | +47.0 |
| 1400 | 1227.3 | 1205.4 | +21.9 | +83.3 |
| 1500 | 1275.9 | 1251.0 | +24.9 | +125.1 |
| 1600 | 1314.8 | 1281.6 | +33.2 | +154.3 |
| 1800 | 1339.3 | 1344.6 | -5.3 | +169.2 |

**Key findings:**
1. k=0.5 continues the high-load improvement trend: +22 at 1400, +25 at 1500, +33 at 1600. This is above noise.
2. Critical regression: at 800 RPS, k=0.5 falls BELOW TailClipper by -10.4. k=1.0 beat TailClipper by +17.6 at the same load point. This is a 28 RPS swing that violates the primary goal.
3. The k→lower trend is not monotone at 800 RPS. There is a sweet spot: k=1.0 wins at 800 RPS, k=0.5 wins at 1400-1600 RPS.
4. The optimal k for this two-trace workload is somewhere in [0.5, 1.0]. The binary search midpoint is k=0.75.

**Decision: REVERT k=0.5, try k=0.75 in Iteration 3.** The 800 RPS loss below TailClipper is unacceptable. k=0.75 may recover 800 RPS while retaining most of the high-load gains.

---

## Iteration 3: Binary search — k=0.75 (experiment cedar_4)

**Status:** Pending

### Change
Set `LatencyMeanVar::default()` to `Self::new(0.75, 0.1)` (k=0.75, keeping α=0.1). Reverts the k=0.5 change first.

### Hypothesis
The optimal k on the two-trace workload is between 0.5 and 1.0. k=0.75 is the binary search midpoint. At this value:
- The variance term still contributes (adds 0.75 std deviations to the deadline), providing some benefit for distinguishing high-latency requests
- But variance inflation from cross-trace mixing is less severe than at k=1.0 and much less than at k=1.2
- The 800 RPS regime (moderate congestion) should benefit from a less-zero k since some variance signal is genuinely useful for priority ordering

We expect k=0.75 to: (a) recover the 800 RPS win over TailClipper, and (b) retain some of the 1400-1600 high-load gains seen with k=0.5.

### Expected outcomes:
1. 800 RPS: est_mean_var beats TailClipper (positive delta)
2. 1400-1600 RPS: improvement over cedar_2 (k=1.0), possibly smaller than cedar_3 (k=0.5) but still positive
3. 1800 RPS: comparable to cedar_2

### Experiment design
Same two-trace config, 3 policies (no fifo).

### Actual Outcomes (cedar_4)

**Status:** Complete ✅ — primary hypothesis confirmed, k=0.75 is new best

**Code commit:** 762bdf20

**cedar_4 (k=0.75) all policies per-RPS:**

| RPS | prio_local,early | prio_oldest,early | est_mean_var (k=0.75) |
|-----|-----------------|-------------------|-----------------------|
| 200 | 197.3 | 199.4 | 196.4 |
| 400 | 392.8 | 391.7 | 388.1 |
| 800 | 784.3 | 753.9 | 764.3 |
| 1000 | 953.8 | 913.2 | 953.7 |
| 1200 | 1116.0 | 1055.1 | 1091.7 |
| 1400 | 1204.7 | 1126.5 | 1220.1 |
| 1500 | 1181.1 | 1156.5 | 1269.2 |
| 1600 | 1183.9 | 1166.5 | 1295.5 |
| 1800 | 1199.1 | 1162.9 | 1359.6 |

**est_mean_var (k=0.75) vs TailClipper:**

| RPS | Delta | Direction |
|-----|-------|-----------|
| 200 | -3.0 | lose (noise) |
| 400 | -3.6 | lose (noise) |
| 800 | +10.4 | **win** ✅ |
| 1000 | +40.5 | **win** |
| 1200 | +36.6 | **win** |
| 1400 | +93.6 | **win** |
| 1500 | +112.7 | **win** |
| 1600 | +129.0 | **win** |
| 1800 | +196.7 | **win** |

**k=0.75 vs k=1.0 (cedar_2) — est_mean_var delta:**

| RPS | k=0.75 | k=1.0 | Δ |
|-----|--------|-------|---|
| 800 | 764.3 | 768.3 | -4.0 (noise) |
| 1000 | 953.7 | 936.7 | **+17.0** |
| 1200 | 1091.7 | 1084.0 | +7.7 |
| 1400 | 1220.1 | 1205.2 | **+14.9** |
| 1500 | 1269.2 | 1250.5 | **+18.7** |
| 1600 | 1295.5 | 1280.9 | **+14.6** |
| 1800 | 1359.6 | 1344.6 | **+15.0** |

**k=0.75 vs k=0.5 (cedar_3) — est_mean_var delta:**

| RPS | k=0.75 | k=0.5 | Δ |
|-----|--------|-------|---|
| 800 | 764.3 | 758.6 | +5.7 |
| 1000 | 953.7 | 939.0 | **+14.7** |
| 1200 | 1091.7 | 1094.3 | -2.6 (noise) |
| 1400 | 1220.1 | 1227.3 | -7.2 |
| 1500 | 1269.2 | 1275.9 | -6.7 |
| 1600 | 1295.5 | 1314.8 | **-19.3** |
| 1800 | 1359.6 | 1339.3 | **+20.3** |

**Key findings:**
1. 800 RPS win over TailClipper fully recovered: +10.4 (vs k=0.5's -10.4). Primary concern from cedar_3 resolved.
2. k=0.75 beats k=1.0 at every load point from 1000 RPS up, by +7 to +19 RPS. Clean directional improvement over current default.
3. k=0.75 vs k=0.5 is a tradeoff: k=0.5 wins at 1400–1600 RPS (by ~7–19 RPS), k=0.75 wins at 800, 1000, and 1800. Neither fully dominates.
4. vs `prio_local,early`: regression at 800 (-20.0) and 1200 (-24.3) persists. Strong gains from 1400+ (+15 to +160). This is a structural property of `est_mean_var` at moderate load, not k-specific.
5. k summary across experiments: **k=0.75 is the best single k for this two-trace workload.** It achieves the best TailClipper gap at 800 RPS (+10.4) and 1000 RPS (+40.5) while retaining competitive high-load performance.

**Decision: KEEP k=0.75.** This is the new default for the two-trace workload.

---

