# SURGE — sched_pred,abort_slo,ac_pred,est_mean_var (socialnet)

## Key questions
- Can we make socialnet's Masa policy (sched_pred,abort_slo,ac_pred,est_mean_var) plateau at system saturation goodput under overload, matching the behavior seen in other apps?
- Why does socialnet's parallel fanout architecture cause the Masa policy to collapse similarly to baselines above ~1400 RPS?
- What algorithmic or parameter changes to scheduling, estimation, admission control, or load shedding can smooth the overload transition for parallel-fanout call graphs?

## Experiment series: surge_1, surge_2, ... (socialnet)

## Prior art
- **test_2** (old feature flags): Masa excelled at 1800 RPS (1729 goodput, 96%) but underperformed at 1400-1600 (81.5%, 68.8%). Only 69 early returns at 1800 vs 256 at 1400 — admission control works at deep overload but not moderate overload.
- **apex_1** (new flags): Showed desired plateau at 1800-2000 (1540, 1704) but may have been a lucky run.
- **apex_14** (new flags, latest): All policies collapse above 1400 RPS. sched_pred matches baselines — no plateau behavior.

## Observed Symptoms (surge_1 baseline)

| RPS | sched_fifo,abort_slo | sched_tailclipper,abort_slo | sched_pred,abort_slo,ac_pred,est_mean_var | Best |
|-----|---------------------|---------------------------|------------------------------------------|------|
| 800 | 800 (100%) | 800 (100%) | 800 (100%) | Tie |
| 1000 | 1000 (100%) | 1000 (100%) | 1000 (100%) | Tie |
| 1200 | 1195 (99.6%) | 1183 (98.6%) | 1193 (99.4%) | FIFO |
| 1400 | 1334 (95.3%) | 1218 (87.0%) | 1270 (90.7%) | **FIFO (+64)** |
| 1600 | 1066 (66.6%) | 1051 (65.7%) | 1096 (68.5%) | **pred (+30)** |
| 1800 | 979 (54.4%) | 1131 (62.8%) | 1191 (66.1%) | **pred (+212)** |
| 2000 | 1204 (60.2%) | 1069 (53.5%) | 1153 (57.7%) | FIFO (+51) |

**Critical finding: ac_pred admission control is COMPLETELY INACTIVE.**
- Zero client_shed events across all RPS levels
- All early returns are from abort_slo (e2e deadline expired in queue, `ColorService=None`)
- The token bucket never rejects because utilization never exceeds UTIL_TARGET=0.92

**Root cause analysis:**
1. **UTIL_TARGET=0.92 is too high.** Text-service P90 CPU is 91.2%, compose-post P90 is 88.3%. The utilization signal hovers just below 0.92, so the token bucket refill rate keeps INCREASING.
2. **Token bucket rate inflates during warmup.** Starting at 10M µs/s, during 800-1200 RPS (util<<0.92), the rate climbs to the max (100M µs/s). By the time 1400+ RPS arrives, the budget is enormous.
3. **By the time util > 0.92, it's too late.** Queue has already built up. Requests expire via abort_slo before reaching before_child_rpc where ac_pred runs.
4. **Oscillation at 1600 RPS observed live:** goodput swings between 635 and 1519/s every ~2 seconds. The feedback loop is unstable.

**CPU bottleneck:** text-service (P90: 91.2%, max: 98.1%) and compose-post (P90: 88.3%, max: 96.6%). System saturates at ~1200-1400 RPS.

## Open hypotheses
1. **UTIL_TARGET too high (CONFIRMED)**: Token bucket never throttles because util stays below 0.92. Lowering to ~0.80 should trigger throttling before queue collapse.
2. **Token bucket rate should not grow unboundedly during warmup**: The rate climbs to 100M µs/s during underload, making it slow to respond when overload begins.
3. **Admission check runs too late**: ac_pred runs at before_child_rpc, but most requests expire at before_poll (abort_slo) before reaching that point.
4. **Feedback loop oscillation**: ADJUST_RATE=0.5 with util oscillating around the target causes boom-bust cycles.
5. **Per-child-RPC admission is wrong for fanout**: Should check once per request, not per child.

---

## Iteration 1: Lower UTIL_TARGET to activate admission control (surge_2)

**Status:** Pending

### Change
- `UTIL_TARGET`: 0.92 → 0.80 in `libs/masa-policy/src/layer/admission/predictive.rs`
- `INITIAL_BUDGET_RATE`: 10_000_000.0 → 5_000_000.0 (halve initial budget to reduce warmup inflation)

### Hypothesis
The admission control is inactive because UTIL_TARGET=0.92 is above the actual CPU utilization of the bottleneck services (text-service P90=91.2%, compose-post P90=88.3%). With UTIL_TARGET=0.80, the token bucket will start reducing its refill rate when the bottleneck hits 80% utilization, which happens at ~1200-1400 RPS. This means by the time the system approaches saturation, the token bucket budget is depleted enough to start rejecting requests proactively.

Additionally, halving the initial budget rate reduces the "warmup inflation" problem: during 800-1200 RPS when utilization is low, the rate won't climb as high, making the transition to throttling faster.

### Expected outcomes if hypothesis is correct:
1. Non-zero client_shed / ac_pred rejections at 1400+ RPS (early returns with service name, not `None`)
2. Goodput plateau: sched_pred goodput at 1600-2000 RPS should be significantly higher than surge_1 (>1200 vs current ~1100-1200)
3. Possible goodput regression at 1200-1400 if threshold is too aggressive (over-throttling under moderate load)
4. Reduced oscillation at 1600 RPS (more stable second-by-second goodput)

### Experiment design
Same config as surge_1 (800-2000 RPS, 30s per step, 50ms SLO). This directly tests whether the admission control is the bottleneck by changing only the threshold parameters.
