# ac_pred Admission Control — Iterative Tuning Log

## Context

Policy: `sched_pred,abort_slack,ac_pred,est_mean_var`  
App: Hotel, 2 APIs (Search SLO=200ms, Reservation SLO=100ms)  
RPS sweep: [100, 1000, 1600, 1800, 2000]  
Warmup: 25s, Measurement: 40s per RPS level  
Experiment dir template: `exp/hotel/in/api_2_pred_vN/` (copy of `api_2_pred` + `policy_param.json`)

Code state: after two correctness fixes committed in this session:
1. Admission check moved to `before_poll` (true ingress, not `before_child_rpc`)
2. `er_rate` EMA replaced with time-corrected EMA (`tau_er`), with `virt_er_rate` decay in `should_admit`

---

## Parameter Reference

| Param | Default | Description |
|-------|---------|-------------|
| `tau_er` | 2.0s | Time constant for `er_rate` EMA in `record_outcome`. Also controls `virt_er_rate` decay in `should_admit` for natural phase reset when idle. |
| `tau_fast` | 0.5s | Fast goodput EMA time constant. Controls speed of overload DETECTION. |
| `tau_slow` | 5.0s | Slow goodput EMA time constant. Provides the historical baseline for overload comparison. |
| `max_reject_floor` | 0.10 | **CAP** on rejection probability in healthy state. NOT a floor. `reject_prob = min(virt_er_rate, cap)` when goodput_fast >= goodput_slow. |
| `estimator_k` | 0.0 | Variance multiplier: estimate = mean + k×stddev. 0=pure mean. |

**Critical**: `max_reject_floor` is a **maximum** (cap) in the healthy state, not a minimum. When `overloaded=true`, the cap is lifted and full `virt_er_rate` applies.

---

## Baseline: api_2_pred

**Parameters**: all defaults (`tau_er=2.0, tau_fast=0.5, tau_slow=5.0, max_reject_floor=0.10, estimator_k=0.0`)

### Goodput summary

| RPS | Agg. Goodput | Fraction | Mean (timeline) | Std | Min–Max |
|-----|-------------|----------|-----------------|-----|---------|
| 100 | 98.3 | 98.3% | 98.5 | 7.0 | – |
| 1000 | 996.7 | 99.7% | 998.7 | 24.6 | – |
| 1600 | 1395.0 | 87.2% | 1400 | 29 | – |
| 1800 | 1459.2 | 81.1% | 1465 | 32 | [1406, 1560] |
| 2000 | 1273.3 | 63.7% | 1279 | 185 | [977, 1566] |

### Timeline observations

- **100/1000 RPS**: fully stable, essentially zero ER.
- **1600 RPS**: goodput 1330–1464/s; ER ~114–246/s (12% average). Stable.
- **1800 RPS**: goodput 1406–1560/s; ER ~214–421/s (17% average). Low variance (std=32). Very stable.
- **2000 RPS**: goodput oscillates **977–1566/s** with ~8–10s period. ER oscillates 328–1009/s.

### Abort reason breakdown at 2000 RPS (from abort_reason_timeline.csv)

- `LocalDeadlineExceeded`: 300–500/s at peaks (genuine SLO violations)
- `PredAdmissionRej`: 190–550/s (AC rejections — excluded from er_rate via `self_rejected` guard)
- `BeforeChildFeasibility`: negligible (<15/s)

### Root cause of 2000 RPS oscillation

The oscillation is a control loop instability driven by the binary healthy/overloaded state switch:

1. **Overloaded** (`goodput_fast < goodput_slow`): AC rejects at full `virt_er_rate` ≈ 0.45–0.55.
   Queue drains; goodput_fast recovers.
2. **Switches to Healthy** (`goodput_fast ≥ goodput_slow`): AC caps at `max_reject_floor=0.10`.
   Admitted load jumps from ~1100 to ~1800/s. Queue floods rapidly.
3. **Switches back to Overloaded**: repeat. Period ~8–10s set by `tau_slow=5s` settling time.

At 2000 RPS, system capacity ≈ 1400–1500 req/s. Stable operation requires ~25–30% rejection.
But in the "healthy" state, the 10% cap only allows 10% rejection → 1800 admitted → overload again.

**Paradox**: the oscillating baseline achieves higher AVERAGE goodput (1279/s) than a stable controller
tuned for 30% rejection would, because the peak goodput bursts (1500–1566/s during healthy periods)
offset the troughs (977–1100/s during overloaded periods).

### ER rate calibration for floor cap

- 1600 RPS: genuine ER ≈ 12% → floor should be ≤ 12% to not over-reject
- 1800 RPS: genuine ER ≈ 17% → floor should be ≤ 17%
- 2000 RPS: genuine ER ≈ 33% → ideal floor ~30–33% for stable operation

A single floor value cannot optimize all three levels simultaneously.

---

## Iteration 1 — v1: Moderate floor increase

**Experiment**: `api_2_pred_v1`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.5, "tau_slow": 5.0, "max_reject_floor": 0.20, "estimator_k": 0.0 } }
```

**Hypothesis**: Raise cap from 0.10 → 0.20 to reduce the oscillation at 2000 RPS. At 1800 RPS
(genuine ER ≈ 17%), floor=0.20 > er_rate so the cap never binds → no cost to 1800 RPS. At 2000 RPS
(genuine ER ≈ 33%), floor=0.20 reduces the "healthy burst" to 80% admission instead of 90% → smaller
oscillation amplitude.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1600 | 1399 | 26 | – | -1 |
| 1800 | 1392 | 82 | [1048, 1495] | **-73** |
| 2000 | 1349 | 210 | [840, 1562] | +70 |

**Analysis**: The hypothesis was wrong about 1800 RPS. The floor=0.20 DESTABILIZED 1800 RPS
(std jumped from 32 → 82, min dropped from 1406 → 1048). Why?

At 1800 RPS, er_rate builds to ~0.20–0.25 during oscillation cycles. With floor=0.20:
- Healthy state: `min(0.25, 0.20) = 0.20` → 80% admitted = 1440/s
- Baseline healthy state: `min(0.25, 0.10) = 0.10` → 90% admitted = 1620/s

The baseline's higher "surge" during healthy state produces higher goodput peaks (1620 admitted briefly →
queue-draining burst → 1460+ goodput). With floor=0.20, the surge is suppressed → lower goodput peaks →
worse mean. Additionally the 2000 RPS improvement (+70/s) comes at the cost of much higher variance (std
210 vs 185).

**Takeaway**: Floor cap tuning has an unexpected negative effect at moderate overload (1800 RPS). The lower
floor (baseline 0.10) enables large "healthy bursts" that produce high peak goodput, which dominates the
average. Raising the floor hurts these bursts.

---

## Iteration 2 — v2: Aggressive floor increase

**Experiment**: `api_2_pred_v2`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.5, "tau_slow": 5.0, "max_reject_floor": 0.30, "estimator_k": 0.0 } }
```

**Hypothesis**: floor=0.30 targets 2000 RPS root cause. In healthy state at 2000 RPS: cap 30%
(matching the ~33% genuine ER) → system stabilizes near capacity without oscillation. At 1800
RPS, genuine er_rate ≈ 17% < 30%, so floor never binds → `min(0.17, 0.30) = 0.17` → no cost.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1600 | 1404 | 28 | – | +4 |
| 1800 | 1439 | 34 | [1372, 1518] | **-26** |
| 2000 | 1127 | 151 | [861, 1494] | **-152** |

**Analysis**: 2000 RPS is MUCH WORSE despite higher floor stability. Why?

With floor=0.30 at 2000 RPS, the system found a stable but LOW operating point (~1127/s) rather than
the oscillating high-mean baseline (1279/s). The reason: when stable at 30% rejection, admitted rate
≈ 1400/s but this is still above the SAFE throughput, so many of those admitted still miss SLO →
`LocalDeadlineExceeded` stays high → er_rate climbs higher → rejection increases → goodput suppressed.

The oscillating baseline achieves higher average because its peak periods (10% rejection → 1800
admitted → 1566/s peak goodput) outweigh the troughs. Floor=0.30 smooths these peaks away.

ER at 2000 RPS was HIGHER with floor=0.30 (mean=850/s vs baseline 671/s) despite more "stable"
rejection — confirming the SLO-miss feedback loop was amplified.

**Takeaway**: Stabilizing the controller via a high floor cap produces a LOWER equilibrium goodput
than the naturally oscillating system. The oscillation at 2000 RPS is not purely harmful; the
high-goodput peaks are necessary for good mean performance.

---

## Iteration 3 — v3: Faster slow EMA

**Experiment**: `api_2_pred_v3`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.5, "tau_slow": 2.0, "max_reject_floor": 0.10, "estimator_k": 0.0 } }
```

**Hypothesis**: Reducing `tau_slow` from 5.0 → 2.0s shrinks the oscillation period (~10s → ~3s),
meaning less time is lost per cycle. Faster goodput memory = faster healthy/overloaded transitions =
potentially higher average.

**Results**:
| RPS | Mean | Std | Δ vs baseline |
|-----|------|-----|---------------|
| 1800 | 1376 | 109 | **-89** |
| 2000 | 1166 | 196 | **-113** |

**Analysis**: Both phases worse. Faster tau_slow INCREASES oscillation frequency (shorter period)
without reducing amplitude → more time is spent in trough states per unit time. Also, with
tau_fast=0.5 and tau_slow=2.0, the separation ratio is only 4× (vs baseline's 10×) → the
healthy/overloaded signal is noisier, causing erratic switching.

**Takeaway**: tau_slow should stay ≥ 5s for stable operation. Reducing it shortens oscillation
period (bad) and reduces detection signal-to-noise ratio.

---

## Iteration 4 — v4: Slower er_rate EMA

**Experiment**: `api_2_pred_v4`
```json
{ "pred": { "tau_er": 5.0, "tau_fast": 0.5, "tau_slow": 5.0, "max_reject_floor": 0.10, "estimator_k": 0.0 } }
```

**Hypothesis**: tau_er=5.0 makes er_rate change very slowly → rejection probability changes more
smoothly → less oscillation amplitude.

**Results**:
| RPS | Mean | Std | Δ vs baseline |
|-----|------|-----|---------------|
| 1800 | 1275 | 175 | **-190** |
| 2000 | 1078 | 177 | **-201** |

**Analysis**: Dramatically worse everywhere. With tau_er=5.0:
- er_rate changes slowly → takes longer to reflect current load state
- At phase transitions (1800→2000 RPS), er_rate from the 1800 phase carries over slowly
- System spends extended time with mismatched rejection probability → prolonged bad states
- The virt_er_rate decay in `should_admit` also uses tau_er, so adaptive phase reset is also slow

**Takeaway**: tau_er=2.0 is near-optimal. Making it slower hurts adaptation speed critically.
The phase reset benefit of slower tau_er is outweighed by the cost of delayed reaction to current load.

---

## Iteration 5 — v5: Slower slow EMA

**Experiment**: `api_2_pred_v5`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.5, "tau_slow": 10.0, "max_reject_floor": 0.10, "estimator_k": 0.0 } }
```

**Hypothesis**: tau_slow=10.0 keeps goodput_slow low during recovery periods. After overload,
goodput_slow decays slowly from its overloaded level → goodput_fast exceeds it quickly → longer
"healthy" periods → more time with floor=10% → higher goodput mean.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1800 | 1479 | 55 | [1377, 1592] | **+14** |
| 2000 | 1214 | 220 | [767, 1571] | -65 |

**Analysis**: Small improvement at 1800 RPS (1479 vs 1465, +1%), but worse at 2000 RPS (1214 vs 1279)
with higher variance (std 220 vs 185). The longer goodput_slow "memory" helps slightly at moderate
overload (1800 RPS) but hurts at severe overload (2000 RPS) where the sluggish slow EMA creates
poorly-timed state transitions.

**Takeaway**: tau_slow=10.0 is marginally better at 1800 RPS but notably worse at 2000 RPS.
Not worth the tradeoff.

---

## Iteration 6 — v6: Lower floor cap

**Experiment**: `api_2_pred_v6`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.5, "tau_slow": 5.0, "max_reject_floor": 0.05, "estimator_k": 0.0 } }
```

**Hypothesis**: floor=0.05 allows more aggressive "healthy state" bursts (95% admitted vs 90% at
baseline) → higher goodput peaks → higher mean at 2000 RPS.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1800 | 1425 | 73 | [1062, 1568] | -40 |
| 2000 | 1241 | 208 | [878, 1594] | -38 |

**Analysis**: Slightly worse everywhere. The lower floor slightly destabilizes 1800 RPS (more variance)
and barely helps 2000 RPS. The difference between 5% and 10% admission surges is not large enough
to materially change outcomes. The system behavior is primarily driven by the healthy/overloaded
switching, not the exact floor value within this range.

**Takeaway**: floor=0.10 appears to be in a reasonable range. Small floor changes have modest effects.

---

## Iteration 7 — v7: Slower fast EMA ⭐ Best at 2000 RPS

**Experiment**: `api_2_pred_v7`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 1.0, "tau_slow": 5.0, "max_reject_floor": 0.10, "estimator_k": 0.0 } }
```

**Hypothesis**: tau_fast=1.0 makes the goodput_fast EMA less reactive to momentary fluctuations.
The healthy/overloaded state flips less frequently → longer healthy burst periods at 2000 RPS →
higher mean goodput.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1800 | 1384 | 134 | [972, 1501] | **-81** |
| 2000 | **1494** | **131** | [960, 1616] | **+215 (+17%)** |

**Analysis**: Dramatic improvement at 2000 RPS (+17%), at the cost of significant 1800 RPS regression.

Why tau_fast=1.0 helps at 2000 RPS:
- Slower goodput_fast → the controller stays in "healthy" state for longer during recovery periods
- During healthy state at 2000 RPS: 90% admission (1800 admitted/s) → goodput peaks at 1550–1616
- These extended high-goodput periods dominate the average → mean rises from 1279 to 1494
- Trough (overloaded) periods have goodput 960–1100, similar to baseline troughs
- Net effect: longer peaks + similar troughs = higher mean

Why tau_fast=1.0 hurts at 1800 RPS:
- Slower detection of overload onset → queue builds larger before AC responds
- Once overloaded, the correction overshoots → deeper troughs → high variance (std=134 vs 32)
- 1800 RPS is at moderate overload where the SPEED of detection matters more

**Takeaway**: tau_fast=1.0 is the single best parameter change found for maximum 2000 RPS goodput
(+215/s, +17%). The tradeoff is -81/s at 1800 RPS with much higher variance. Best choice if
2000 RPS is the priority use case.

---

## Iteration 8 — v8: Conservative estimator

**Experiment**: `api_2_pred_v8`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.5, "tau_slow": 5.0, "max_reject_floor": 0.10, "estimator_k": 1.0 } }
```

**Hypothesis**: estimator_k=1.0 makes `abort_slack` more conservative (estimate = mean + 1×stddev).
Fewer spurious `BeforeChildFeasibility` early returns → lower er_rate → less AC rejection → higher goodput.

**Results**:
| RPS | Mean | Std | Δ vs baseline |
|-----|------|-----|---------------|
| 1800 | 1317 | 161 | **-148** |
| 2000 | 1184 | 195 | **-95** |

**Analysis**: Much worse. The conservative estimator doesn't just reduce false abort_slack — it
appears to change the estimator behavior in ways that degrade performance. At high load, more
conservative estimates mean `abort_slack` fires less → more requests queue up → more LocalDeadlineExceeded
→ er_rate driven by genuine SLO failures, which are now WORSE because we stopped doing early
returns preventively. The `BeforeChildFeasibility` aborts were actually protective at high load;
removing them causes deeper SLO violations.

**Takeaway**: estimator_k=0.0 (pure mean) is correct. Conservative (higher k) estimates reduce
the protective effect of `abort_slack` and worsen outcomes. The current estimator setting is good.

---

## Iteration 9 — v9: tau_fast=1.0 + tau_slow=10.0 combination

**Experiment**: `api_2_pred_v9`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 1.0, "tau_slow": 10.0, "max_reject_floor": 0.10, "estimator_k": 0.0 } }
```

**Hypothesis**: Combining v7's benefit (tau_fast=1.0 for longer healthy periods) with v5's idea
(tau_slow=10.0 for lower goodput_slow reference) might extend healthy periods even more,
improving 2000 RPS while tau_slow=10.0 might stabilize 1800 RPS.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1800 | 1376 | 123 | [861, 1474] | -89 |
| 2000 | 1135 | 177 | [826, 1508] | **-144** |

**Analysis**: Dramatically WORSE than v7 alone (1135 vs 1494 at 2000 RPS). The combination is
NOT additive — it's destructive. The root cause:

With tau_slow=10.0, `goodput_slow` retains the high value from the 1800 RPS phase for a long time
into the 2000 RPS measurement window. At the start of 2000 RPS, goodput_slow ≈ 1460 (carried from
1800 phase). With tau_fast=1.0, goodput_fast also updates slowly. But the initial overload at 2000
RPS causes goodput_fast to stay ≤ goodput_slow for the first ~10s of the measurement → prolonged
"overloaded" state → high rejection → poor goodput for the first half of measurement.

With tau_slow=5.0 (v7), goodput_slow decays faster → system finds its operating point sooner.
The combination tau_fast=1.0 + tau_slow=10.0 creates a cross-phase contamination problem.

**Takeaway**: tau_slow=5.0 + tau_fast=1.0 (v7) is the right combination for 2000 RPS. tau_slow=10.0
pairs poorly with tau_fast=1.0 because it introduces phase carry-over that hurts the measurement window.

---

## Iteration 10 — v10: Intermediate tau_fast=0.75

**Experiment**: `api_2_pred_v10`
```json
{ "pred": { "tau_er": 2.0, "tau_fast": 0.75, "tau_slow": 5.0, "max_reject_floor": 0.10, "estimator_k": 0.0 } }
```

**Hypothesis**: tau_fast=0.75 is a compromise between baseline (0.5, good at 1800) and v7 (1.0,
good at 2000). Expected to be intermediate at both RPS levels.

**Results**:
| RPS | Mean | Std | Min–Max | Δ vs baseline |
|-----|------|-----|---------|---------------|
| 1800 | 1331 | 150 | [926, 1525] | **-134** |
| 2000 | 1223 | 221 | [853, 1555] | -56 |

**Analysis**: Worse than baseline at 1800 RPS (-134) AND worse than v7 at 2000 RPS (1223 vs 1494).
This is a "worst of both worlds" outcome. The improvement at 2000 RPS only appears clearly at
tau_fast=1.0; at tau_fast=0.75, 1800 RPS is already significantly degraded without the 2000 RPS gain.

The response surface appears non-linear: there's a threshold effect where tau_fast must reach
approximately 1.0 to get the extended-healthy-period benefit at 2000 RPS, while any increase
from 0.5 immediately degrades 1800 RPS performance.

**Takeaway**: The tau_fast sensitivity is highly nonlinear. tau_fast=0.5 and tau_fast=1.0 are
distinct operating regimes. Values between them don't interpolate smoothly — they suffer the 1800
RPS cost without getting the full 2000 RPS benefit. No compromise value exists.

---

## Summary Table

| Version | tau_er | tau_fast | tau_slow | floor | k | GP@1600 (std) | GP@1800 (std) | GP@2000 (std) | Sum@1800+@2000 |
|---------|--------|----------|----------|-------|---|---------------|---------------|---------------|----------------|
| baseline | 2.0 | 0.5 | 5.0 | 0.10 | 0.0 | 1400 (29) | **1465 (32)** | 1279 (185) | 2744 |
| v1 | 2.0 | 0.5 | 5.0 | 0.20 | 0.0 | 1399 (26) | 1392 (82) | 1349 (210) | 2741 |
| v2 | 2.0 | 0.5 | 5.0 | 0.30 | 0.0 | 1404 (28) | 1439 (34) | 1127 (151) | 2566 |
| v3 | 2.0 | 0.5 | 2.0 | 0.10 | 0.0 | 1391 (30) | 1376 (109) | 1166 (196) | 2542 |
| v4 | 5.0 | 0.5 | 5.0 | 0.10 | 0.0 | 1398 (28) | 1275 (175) | 1078 (177) | 2353 |
| v5 | 2.0 | 0.5 | 10.0 | 0.10 | 0.0 | 1393 (27) | 1479 (55) | 1214 (220) | 2693 |
| v6 | 2.0 | 0.5 | 5.0 | 0.05 | 0.0 | 1379 (28) | 1425 (73) | 1241 (208) | 2666 |
| **v7** | 2.0 | **1.0** | 5.0 | 0.10 | 0.0 | 1389 (29) | 1384 (134) | **1494 (131)** | **2877** |
| v8 | 2.0 | 0.5 | 5.0 | 0.10 | 1.0 | 1400 (24) | 1317 (161) | 1184 (195) | 2501 |
| v9 | 2.0 | 1.0 | 10.0 | 0.10 | 0.0 | 1392 (24) | 1376 (123) | 1135 (177) | 2511 |
| v10 | 2.0 | 0.75 | 5.0 | 0.10 | 0.0 | 1403 (37) | 1331 (150) | 1223 (221) | 2554 |

**Best at 1800 RPS**: baseline (mean=1465, std=32 — highly stable)  
**Best at 2000 RPS**: v7 tau_fast=1.0 (mean=1494, std=131 — +17% vs baseline)  
**Best combined (sum@1800+@2000)**: v7 (2877 vs baseline 2744, +5%)

---

## Key Insights

### 1. The oscillation at 2000 RPS is NOT purely harmful

The baseline "oscillates" between goodput ~977 and ~1566 at 2000 RPS. This oscillation achieves
**higher mean goodput** (1279/s) than any stable operating point found. The high-goodput burst periods
(10% rejection → 1800 admitted → 1500+ goodput) contribute more to the mean than the troughs cost.
Smoothing the oscillation (v2: floor=0.30) actually produced LOWER goodput (1127/s) by removing these
beneficial peaks.

### 2. The floor cap is a blunt instrument

`max_reject_floor` was designed to prevent over-rejection in healthy states. But:
- It suppresses the "healthy burst" that drives good mean goodput at heavy overload
- Raising it past the genuine ER rate destabilizes moderate overload (1800 RPS)
- The optimal floor appears to be low (0.05–0.10), allowing aggressive healthy surges

### 3. tau_fast is the most impactful parameter for 2000 RPS

tau_fast=1.0 (v7) produced +17% goodput at 2000 RPS. Mechanism: slower goodput_fast → less
frequent state flipping → longer healthy burst periods → more time serving at high throughput.
This comes at the cost of 1800 RPS stability because slower detection allows overload to build deeper.

### 4. tau_fast sensitivity is nonlinear (threshold effect)

tau_fast=0.75 did NOT produce a proportional improvement — it hurt 1800 RPS like tau_fast=1.0
but barely improved 2000 RPS. The benefit only materializes at ~tau_fast=1.0. There is no smooth
compromise in the range [0.5, 1.0].

### 5. tau_slow and tau_er should stay near defaults

- tau_slow=2.0 shortens oscillation periods (bad)
- tau_slow=10.0 gives marginal +14/s at 1800, worse at 2000
- tau_slow=5.0 (baseline) is the right balance
- tau_er=5.0 is too slow for adaptation (much worse everywhere)
- tau_er=2.0 (baseline) is near-optimal for responsiveness

### 6. estimator_k=0.0 (pure mean) is correct

Increasing k to 1.0 reduced `abort_slack` firing, but the protective early returns from `abort_slack`
were preventing GENUINE SLO violations. Making the estimator more conservative allowed deeper queue
buildup → worse LocalDeadlineExceeded rate → worse goodput.

---

## Recommendations

**If optimizing for 2000 RPS (heavy overload priority)**:
Use **v7 params** (`tau_fast=1.0`, all else at defaults). Achieves +17% goodput at 2000 RPS (1494 vs
1279) with slightly better stability (std=131 vs 185). Tradeoff: 1800 RPS drops 5.5% and becomes
much more variable (std=134 vs 32).

**If optimizing for 1800 RPS (moderate overload priority)**:
Keep **baseline params** (all defaults). Best stable performance at 1800 RPS (1465, std=32).

**If optimizing across all load levels**:
v7 has the highest combined @1800+@2000 sum (2877 vs 2744 baseline), so it's the best overall by
aggregate goodput if both load levels are weighted equally. The 1800 RPS instability with v7 is
a real concern for production.

---

## Open Questions / Future Directions

1. **Adaptive tau_fast**: Can the system switch between tau_fast=0.5 (moderate overload) and
   tau_fast=1.0 (severe overload) automatically based on the current RPS signal? Would require
   code changes but could give the best of both worlds.

2. **Continuous healthy/overloaded interpolation**: Replace the binary healthy/overloaded switch
   with a continuous signal: `reject_prob = virt_er_rate * f(goodput_ratio)` where
   `f(goodput_fast/goodput_slow)` smoothly transitions between full and capped rejection.
   This would eliminate the cliff at the healthy/overloaded boundary.

3. **Separate AC for different API types**: Search (SLO=200ms) and Reservation (SLO=100ms) have
   different characteristics. A per-API AC controller with separate state could reduce cross-API
   interference.

4. **Per-service vs per-request er_rate**: Currently all requests share one AC controller at the
   frontend. If er_rate were per-method (Search vs Reservation), each API could tune independently.

---

## How to Continue in Next Session

If context runs out before all analysis is complete:

1. **Experiments already run and recorded**: baseline + v1–v10, all in `exp/hotel/plots/api_2_pred_vN/`
2. **Key result to communicate**: v7 (tau_fast=1.0, all else default) is best overall (+17% at 2000 RPS)
3. **Resume command**: All done — this was the last iteration (v10). Analysis is complete.
4. **If re-running is needed**:
   ```bash
   cd /mnt/data/jiexiao/masaA
   uv run -m exp_runner run hotel api_2_pred_vN --plot
   ```
5. **Analysis script**: See the python3 inline scripts in this session to re-generate comparison tables
6. **This file**: `/mnt/data/jiexiao/masaA/ac_pred_tuning.md`
7. **Experiment input dirs**: `exp/hotel/in/api_2_pred_vN/policy_param.json` (N=1..10)
8. **Experiment output dirs**: `exp/hotel/plots/api_2_pred_vN/0/goodput/goodput_timeline.csv`
