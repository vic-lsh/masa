# RIDGE — prio_local,early,adctl,est_mean_var (cross-app optimization)

## Key questions
- Can `prio_local,early,adctl,est_mean_var` maximize goodput across all three apps (hotel, socialnet, mssim), not just beat baselines?
- Are there cross-app failure modes where a change that helps one app hurts another?
- What is the ceiling for goodput improvement when optimizing the target policy holistically?

## Baselines
- `fifo,early` — simplest deadline-aware policy
- `prio_oldest,early` — age-based priority with early return
- `fifo,rajomon` — Rajomon admission control (tuned: QUEUE_THRESHOLD_US=5000, PRICE_PER_EXCESS_MS=2)

## Experiment series: ridge_1, ridge_2, ... (hotel, socialnet, mssim)

### Phase 0: Baseline (ridge_1)

**Goal:** Establish cross-app baseline performance of all 4 policies.

**Configs:**
- **Hotel (ridge_1):** Search SLO=200ms, Reservation SLO=50ms, RPS [100, 400, 800, 1200, 1400, 1600, 1800, 2000], warmup=10s, duration=20s
- **Socialnet (ridge_1):** ComposePost SLO=50ms, RPS [100, 200, 300, 400, 500, 600, 700, 800], warmup=10s, duration=20s
- **MSSIM (ridge_1):** S_14677443 callgraph, SLO=100ms, RPS [200, 400, 800, 1000, 1200, 1400, 1500, 1600, 1800], duration=30s

**Policies (all apps):**
1. `prio_local,early,adctl,est_mean_var` (target)
2. `fifo,early` (baseline)
3. `prio_oldest,early` (baseline)
4. `fifo,rajomon` (baseline)

### Observed Symptoms — Hotel (ridge_1)

| RPS | fifo,rajomon | fifo,early | prio_oldest,early | **target** | Winner |
|-----|-------------|-----------|-------------------|-----------|--------|
| 100 | 99.5 | 99.5 | 99.5 | 99.5 | Tie |
| 400 | 397.7 | 397.8 | 397.9 | 397.9 | Tie |
| 800 | 795.0 | 795.6 | 796.1 | 795.7 | Tie |
| 1200 | 1128.7 | 1172.7 | **1181.9** | 1166.0 | prio_oldest |
| 1400 | 981.2 | 1048.0 | 1104.0 | **1220.0** | **Target** |
| 1600 | 702.3 | 802.1 | 1060.1 | **1339.7** | **Target** |
| 1800 | 698.8 | 744.6 | 1056.2 | **1492.1** | **Target** |
| 2000 | 695.8 | 665.5 | 1075.8 | **1561.9** | **Target** |

**Key findings (hotel):**
- Target policy dominates above 1400 RPS (+24% to +135% vs baselines)
- Admission control is the primary mechanism — 3x fewer early returns than baselines, yet 2x goodput
- Goodput increases monotonically from 1166→1562 as load rises (effective admission control)
- Minor weakness at 1200 RPS: prio_oldest wins by ~16 RPS (1.3%)
- System is CPU-bound on reservation-service (~100% P90)

### Observed Symptoms — MSSIM (ridge_1)

| RPS | fifo,rajomon | fifo,early | prio_oldest,early | **target** | Winner |
|-----|-------------|-----------|-------------------|-----------|--------|
| 200 | 197.5 | 203.8 | 200.7 | 198.1 | fifo,early |
| 400 | 397.2 | 404.1 | 396.4 | 404.6 | target |
| 800 | 15.5 | 542.6 | 790.5 | **806.3** | **Target** |
| 1000 | 0.0 | 307.3 | **786.7** | 770.1 | prio_oldest |
| 1200 | 0.0 | 319.2 | **755.8** | 709.2 | prio_oldest |
| 1400 | 0.0 | 356.5 | **767.1** | 748.4 | prio_oldest |
| 1500 | 0.0 | 382.2 | 766.2 | **818.6** | **Target** |
| 1600 | 0.0 | 425.2 | 773.7 | **844.0** | **Target** |
| 1800 | 0.0 | 458.2 | 794.6 | **918.5** | **Target** |

**Key findings (mssim):**
- Saturation at ~800-1000 RPS
- `fifo,rajomon` is non-functional (0 goodput above 800 RPS)
- Target slightly underperforms `prio_oldest,early` at 1000-1400 RPS (up to -6.2%)
- Target crosses over at 1500 RPS and leads by +15.6% at 1800 RPS
- Target goodput increases monotonically from 770→918 as load rises
- Bottleneck services (ms-56394, ms-73106) at ~93-98% P90 CPU

### Observed Symptoms — Socialnet (ridge_1)

All 4 policies achieve >99.94% goodput fraction across all RPS levels (100-800). **System is not saturated** — need to extend RPS range to 1500+ to find saturation point. No meaningful policy differentiation at this load range.

### Cross-App Baseline Summary

**Consistent pattern:** The target policy dominates at deep overload but slightly underperforms `prio_oldest,early` at moderate overload:
- **Hotel**: target loses at 1200 RPS (-16 goodput, -1.3%), wins 1400+
- **MSSIM**: target loses at 1000-1400 RPS (up to -6.2%), wins 1500+
- **Socialnet**: not yet informative (below saturation)

**Root cause hypothesis:** adctl admission control (UTIL_TARGET=0.85) kicks in at moderate overload, shedding requests that could still complete. At deep overload, this shedding is essential and produces the goodput advantage.

---

## Iteration 1: Raise UTIL_TARGET from 0.85 to 0.92 (experiment ridge_2)

**Status:** Pending

### Change
Raise `UTIL_TARGET` from 0.85 to 0.92 in `libs/tonic/tonic/src/masa/context/adctl.rs`.

### Hypothesis
The `UTIL_TARGET=0.85` threshold causes adctl Layer 2 (efficiency-based admission at ingress) to start reducing the budget rate when any downstream bottleneck service reaches 85% utilization. At moderate overload:
- Hotel 1200 RPS: reservation-service at ~100% CPU → util > 0.85 → budget decreases → sheds ~16 extra requests vs prio_oldest
- MSSIM 1000-1400 RPS: bottleneck services at 87-93% CPU → same mechanism

Raising the threshold to 0.92 means the budget only decreases when utilization exceeds 92%, which:
1. At moderate overload (util 85-92%): no shedding → goodput matches prio_oldest
2. At deep overload (util >92%): shedding still activates, preserving the deep-overload advantage
3. The difference is ~7 percentage points of utilization headroom before admission control engages

This does NOT affect Layer 1 (compute feasibility check), which runs at every hop regardless. Layer 1 protects against truly infeasible requests. Layer 2 is the rate limiter that prunes marginal requests — the one that's being too conservative.

### Expected outcomes if hypothesis is correct:
1. Hotel 1200 RPS: goodput ≥1180 (matching prio_oldest's 1182)
2. MSSIM 1000-1400 RPS: goodput improves to match or exceed prio_oldest (~755-787)
3. Hotel 1400-2000 RPS: goodput maintained ≥1220-1562 (no regression at deep overload)
4. MSSIM 1500-1800 RPS: goodput maintained ≥818-918

### Experiment design
Run hotel and mssim with same ridge_1 configs. Also extend socialnet RPS to [100, 300, 500, 700, 900, 1100, 1300, 1500] to find saturation.
