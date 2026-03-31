# GEYSER — Evaluate FLARE admission control implementation

## Key questions
- Does the FLARE implementation (max-util rate signal, ER backstop, probabilistic smoothing) improve or regress socialnet goodput compared to pre-FLARE ember_3 baseline?
- Can `sched_pred,abort_slo,ac_pred,est_mean_var` beat `sched_tailclipper,abort_slo` across all overload points on socialnet, hotel_search, hotel_reservation, and hotel_2api?
- If FLARE regresses, which of the three new features (max-util, ER backstop, prob smoothing) is responsible, and what parameter tuning or redesign is needed?

## Pre-FLARE reference (ember_3, socialnet)

| RPS | ac_pred | tailclipper | pred (no AC) | fifo |
|-----|---------|-------------|--------------|------|
| 800 | 800 | 800 | 800 | 800 |
| 1000 | 1000 | 1000 | 1000 | 1000 |
| 1200 | 1186 | 1182 | 1187 | 1184 |
| 1400 | 1344 | 1319 | 1194 | 1113 |
| 1600 | 1438 | 1083 | 1057 | 1018 |
| 1800 | 1497 | 1689* | 1090 | 969 |
| 2000 | 1568 | 1095 | 1080 | 1069 |
| 2500 | 1603 | 1023 | 1043 | 1020 |
| 3000 | 1582 | 990 | 1029 | 537 |

*tailclipper 1800 is bistable (lucky run)

## Experiment series: geyser_1, geyser_2, ... (socialnet)

## Observed Symptoms (geyser_1 — FLARE baseline on socialnet)

| RPS | fifo+abort | tailclipper | pred (no AC) | **ac_pred (FLARE)** | vs ember_3 |
|-----|-----------|-------------|--------------|---------------------|------------|
| 800 | 800 | 800 | 800 | 800 | 0 |
| 1000 | 1000 | 1000 | 1000 | 1000 | 0 |
| 1200 | 1189 | 1198 | 1178 | 1172 | -14 |
| 1400 | 1302 | 1287 | 1312 | 1369 | +25 |
| 1600 | 1109 | 1069 | 1084 | 1286 | **-152** |
| 1800 | 1009 | 1233 | 1122 | 1248 | **-249** |
| 2000 | 1213 | 1309 | 1102 | 1306 | **-262** |
| 2500 | 1085 | 1142 | 1117 | 1425 | **-178** |
| 3000 | 211 | 1044 | 1145 | **0** | **-1582** |

**FLARE is a significant regression from ember_3 at all overloaded RPS points (1600+).**

### ER breakdown (ac_pred)

| RPS | abort_slo | client_shed (AC) | Total ER |
|-----|-----------|------------------|----------|
| 1200 | 27.6 | 0.1 | 27.7 |
| 1400 | 30.3 | 0 | 30.3 |
| 1600 | 254.4 | 58.2 | 312.6 |
| 1800 | 371.2 | 179.5 | 550.7 |
| 2000 | 441.6 | 249.2 | 690.9 |
| 2500 | 520.1 | 549.3 | 1070.5 |
| 3000 | — | 3000 (all rejected) | 3000 |

**Smoking gun:** At 3000 RPS, ac_pred rejects ALL requests via client_shed → 0 goodput.
The admission controller enters a death spiral: ER backstop ratchets pseudo-util to 1.0,
rate collapses, probabilistic admission can't recover because budget stays at 0.

### Root cause analysis

1. **ER backstop positive feedback:** Layer 1/abort_slo ERs drive er_fraction up → pseudo_util
   saturates → rate drops → budget depletes → Layer 2 rejects more → although Layer 2 is
   excluded from ER count, the original Layer 1 ER rate stays high → rate never recovers.
2. **No safety valve:** When budget hits 0, p=0 forever (budget ≤ 0 → immediate return false).
   There's no minimum admission probability to break the death spiral.
3. **Over-shedding at 1600-2500:** Even without total collapse, client_shed rates (58-549/s)
   are too high — FLARE is rejecting requests that ember_3 would have admitted and served.

## Iteration 1: Disable ER backstop + add minimum admission floor (geyser_2)

**Status:** Pending

### Change
1. Set `ER_THRESHOLD = f64::INFINITY` to effectively disable the ER backstop (pseudo_util always 0).
2. Add a minimum admission probability floor: when budget < cost but budget > 0, ensure
   `p >= MIN_ADMIT_PROB` (e.g., 0.05) so at least 5% of requests always get through.
3. When budget ≤ 0, use `MIN_ADMIT_PROB` instead of hard-rejecting.

### Hypothesis
The ER backstop is the primary cause of the regression. At high load, Layer 1/abort_slo
naturally shed doomed requests, driving er_fraction up. The backstop interprets this as
overload and throttles the rate, but the shedding is *desirable* (doomed requests should
be shed). The backstop conflates "healthy shedding" with "system overload," causing
over-throttling. Disabling it should restore ember_3-level performance at 1600-2500 RPS.

The minimum admission floor prevents the death spiral at 3000 RPS regardless of signal
values — even if the rate signal is maximally pessimistic, some requests always get through.

### Expected outcomes if hypothesis is correct:
1. 3000 RPS: goodput recovers from 0 to something positive (>500)
2. 1600-2500 RPS: goodput improves toward ember_3 levels (~1400-1600)
3. 1200-1400 RPS: no regression (ER backstop was inactive at these loads anyway)

### Experiment design
Same config as geyser_1 (surge_8 base, ComposePost, SLO=50ms, RPS 800-3000).
Only run ac_pred and tailclipper policies to save time (fifo/pred baselines unchanged).
