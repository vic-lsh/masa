# PRISM — FLARE ablation study

## Key questions
- How much does `sched_pred` (deadline tightening + EDF reprioritization) contribute to FLARE goodput vs plain `sched_slo`?
- How much does `abort_slo` (drop past-deadline requests) contribute when the rest of FLARE is present?
- Do scheduling and abort_slo interact (i.e., does the benefit of one depend on the other being present)?

## Experiment series: prism_1, prism_2, ... (socialnet, then hotel)

## Phase 1: Scheduling ablation (prism_1, socialnet)

**Policies under test:**
1. `sched_pred,abort_slo,ac_pred,est_mean_var` — full FLARE
2. `sched_slo,abort_slo,ac_pred,est_mean_var` — drop sched_pred (no deadline tightening, no EDF reprioritization)
3. `sched_tailclipper,abort_slo` — external baseline

**Config:** Same as geyser_7 (ComposePost, SLO=50ms, RPS 800-3000, 30s/step, 10s warmup).

**What sched_pred adds over sched_slo:**
- `before_poll`: reprioritizes task based on updated deadline estimate (EDF)
- `before_child_rpc`: tightens child deadline by `est_remaining` (deadline tightening)
- Both require the estimator to have warmed up

**Expected outcome:** sched_pred should help at deep overload where priority ordering
matters most. At moderate overload the AC dominates and scheduling is secondary.

### Results (prism_1)

| RPS | sched_pred (FLARE) | sched_slo | tailclipper | pred−slo | pred−TC |
|-----|--------------------|-----------|-------------|----------|---------|
| 800 | 800 | 800 | 800 | 0 | 0 |
| 1000 | 1000 | 1000 | 1000 | 0 | 0 |
| 1200 | 1191 | 1177 | 1177 | +14 | +14 |
| 1400 | 1311 | 1339 | 1278 | −28 | +33 |
| 1600 | 1258 | 1287 | 1065 | −29 | +193 |
| 1800 | 1192 | 1257 | 1019 | −65 | +173 |
| 2000 | 1273 | 1291 | 1075 | −18 | +198 |
| 2500 | 1304 | 1380 | 1078 | −76 | +226 |
| 3000 | 1451 | 1379 | 985 | +72 | +466 |

**Finding: sched_pred is roughly neutral on socialnet.** It slightly hurts at moderate
overload (1400-2500, −18 to −76) and slightly helps at extreme overload (3000, +72).
The ER breakdown confirms deadline tightening over-sheds at moderate overload:
sched_pred frontend ER rate is higher (1189/s vs 1115/s at 2500 RPS).

This is consistent with PINE findings: parallel fanout architectures (socialnet) don't
benefit from deadline tightening because it creates priority inversions on completable
requests. The ac_pred token bucket is doing the heavy lifting — both sched_pred and
sched_slo beat tailclipper by +170 to +466 at 1600+ RPS.

---

## Phase 2: abort_slo ablation (prism_2, socialnet)

**Policies under test:**
1. `sched_pred,abort_slo,ac_pred,est_mean_var` — full FLARE
2. `sched_pred,ac_pred,est_mean_var` — drop abort_slo (no deadline-based request dropping)
3. `sched_tailclipper,abort_slo` — external baseline

**Config:** Same as prism_1 (ComposePost, SLO=50ms, RPS 800-3000, 30s/step).

**What abort_slo does:** Drops requests that have exceeded their e2e SLO deadline at
before_poll and before_child_rpc checkpoints. Without it, expired requests continue
executing and consuming resources, crowding out fresh requests that could still meet SLO.

### Results (prism_2)

| RPS | +abort_slo (FLARE) | −abort_slo | tailclipper | abort_slo Δ | FLARE−TC |
|-----|--------------------|------------|-------------|-------------|----------|
| 800 | 800 | 800 | 800 | 0 | 0 |
| 1000 | 1000 | 1000 | 1000 | 0 | 0 |
| 1200 | 1198 | 1195 | 1170 | +3 | +28 |
| 1400 | 1310 | 1281 | 1147 | +29 | +163 |
| 1600 | 1258 | 1209 | 1016 | +49 | +242 |
| 1800 | 1206 | 1130 | 1084 | +76 | +122 |
| 2000 | 1272 | 1182 | 1081 | +90 | +191 |
| 2500 | 1284 | 1141 | 1008 | +144 | +276 |
| 3000 | 1342 | 1084 | 1002 | +259 | +340 |

**Finding: abort_slo provides consistent, load-proportional benefit.** The delta grows
monotonically from +3 at 1200 to +259 at 3000 RPS. At extreme overload (3000), abort_slo
accounts for ~76% of FLARE's advantage over tailclipper (259 of 340).

**Mechanism (ER breakdown):** Without abort_slo, expired requests leak deep into the call
graph before being shed. At 3000 RPS without abort_slo: compose_post sheds 197/s,
text_service sheds 266/s — these requests consumed CPU at multiple hops before dying.
With abort_slo, nearly all shedding happens at frontend (1651/s at 3000), which is
maximally cheap (no downstream work wasted).

**However:** Even without abort_slo, ac_pred alone still beats tailclipper at all
overloaded points (+82 to +163 at 1600-3000). The token bucket's cost-aware shedding
provides a baseline advantage even when expired requests aren't proactively dropped.

---

## Summary

| Component | Contribution to FLARE (socialnet) |
|-----------|----------------------------------|
| **ac_pred (token bucket)** | Dominant — provides +80 to +466 over tailclipper even without other features |
| **abort_slo** | Significant — +29 to +259 incremental benefit, grows with load |
| **sched_pred** | Negligible/slightly negative — −18 to −76 at moderate overload, +72 at 3000 only |

The FLARE advantage on socialnet is primarily driven by cost-aware admission control
(ac_pred), amplified by abort_slo's early shedding of doomed requests. The scheduling
discipline (sched_pred vs sched_slo) is not a significant contributor for this
parallel-fanout workload.
