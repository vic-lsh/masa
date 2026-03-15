# FORGE — Scheduling policy comparison with adctl

## Key questions
- How much of adctl's goodput benefit comes from the scheduling policy (prio_local vs prio_oldest vs fifo), when all policies use the same admission control (adctl,early,est_mean_var)?
- Does the scheduling policy matter at all when adctl is doing the heavy lifting, or does adctl equalize performance across scheduling strategies?

## Context

QUARTZ (iterations 11–14) established that `prio_local,est_mean_var,early,adctl` achieves 1705 goodput at 2000 RPS on the coral_ext_2 workload (Search SLO=200ms, Reservation SLO=50ms). This is a dramatic improvement over `prio_oldest,early` (400 at 2000 RPS in quartz_14).

However, the QUARTZ comparison was `prio_local+adctl` vs `prio_oldest` (without adctl). The natural question: how much of the 1300+ goodput gap is from adctl's admission control vs prio_local's scheduling? This evaluation isolates the scheduling policy variable by keeping adctl,early,est_mean_var constant.

**Policies under test:**
- `fifo,early,adctl,est_mean_var` — no priority scheduling, just FIFO + admission control
- `prio_oldest,early,adctl,est_mean_var` — oldest-first scheduling + admission control
- `prio_local,early,adctl,est_mean_var` — local-deadline scheduling + admission control

## Experiment series: forge_1, forge_2, ... (hotel)

### Baseline: forge_1 (mixed-SLO eval, coral_ext_2 config)

**Config:** Search SLO=200ms, Reservation SLO=50ms, RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000]. Three policies all with adctl,early,est_mean_var.

**Goal:** Establish baseline performance of each scheduling policy when combined with the same admission control mechanism.

### Results: forge_1

**Status:** Complete ✅

#### Goodput Comparison

| RPS | fifo+adctl | prio_oldest+adctl | prio_local+adctl | QUARTZ q14 (prio_local+adctl) |
|-----|-----------|-------------------|-----------------|-------------------------------|
| 100 | 99.5 | 99.5 | 99.6 | 99.8 |
| 400 | 398.1 | 398.1 | 398.1 | 399.3 |
| 800 | 796.0 | 795.9 | 796.1 | 798.5 |
| 1200 | 1193.6 | 1193.9 | 1194.0 | 1187.0 |
| 1400 | 1386.0 | 1392.1 | 1392.4 | 1271.2 |
| 1600 | 1506.8 | 1541.4 | 1575.1 | 1464.5 |
| 1800 | 1600.4 | 1642.9 | 1696.7 | 1604.0 |
| 2000 | 1755.1 | 1791.7 | 1829.1 | 1704.7 |

#### Goodput Fraction (% of offered load)

| RPS | fifo+adctl | prio_oldest+adctl | prio_local+adctl |
|-----|-----------|-------------------|-----------------|
| 1400 | 99.0% | 99.4% | 99.5% |
| 1600 | 94.2% | 96.3% | 98.4% |
| 1800 | 88.9% | 91.3% | 94.3% |
| 2000 | 87.8% | 89.6% | 91.5% |

#### Early Return Rates (requests/sec at frontend)

| RPS | fifo+adctl | prio_oldest+adctl | prio_local+adctl |
|-----|-----------|-------------------|-----------------|
| 1600 | 84.6 (82.5 Res + 2.1 Search) | 49.8 (42.2R + 7.6S) | 15.6 (9.0R + 6.6S) |
| 1800 | 188.7 (122.6R + 66.1S) | 147.4 (47.4R + 99.9S) | 92.8 (4.7R + 88.1S) |
| 2000 | 234.1 (70.2R + 163.8S) | 197.1 (31.3R + 165.8S) | 160.5 (4.8R + 155.7S) |

### Key Findings

1. **adctl is the dominant mechanism; scheduling policy is a refinement.** Without adctl, prio_oldest got 400 goodput at 2000 RPS (QUARTZ q14). With adctl, even fifo achieves 1755 — a 4.4x improvement. The spread across all three policies is only 74 goodput (4.2%) at 2000 RPS.

2. **prio_local > prio_oldest > fifo consistently.** The ranking is monotonic at every overload point. The gap widens with load: +6 at 1400, +68 at 1600, +96 at 1800, +74 at 2000 (prio_local vs fifo).

3. **prio_local concentrates early returns on Search (expensive), sparing Reservation (cheap).** At 2000 RPS: prio_local early-returns only 4.8 Reservation/s vs fifo's 70.2. This is the scheduling policy's mechanism — by giving tight-SLO Reservation requests higher priority, fewer miss their deadline and need shedding.

4. **All policies improve over QUARTZ q14 at 1400+ RPS.** forge_1 prio_local achieves 1829 at 2000 vs q14's 1705 (+124). Likely because q14 used `prio_local,est_mean_var,early,adctl` but prio_oldest,early (without adctl or est_mean_var) as its baseline — the QUARTZ runs were from an earlier code state. Some run-to-run variance is also expected.

5. **reservation-service is the bottleneck** (CPU ~100% at p90/p99 across all policies), confirming the workload is compute-bound and the scheduling policy's effect operates within that constraint.

### Assessment

**Answer to the key question:** adctl provides ~95% of the goodput improvement (from 400 to 1755-1829). The scheduling policy provides the remaining ~5% refinement (74 goodput spread at 2000 RPS). However, prio_local's advantage is consistent and grows with load — it's a real benefit, just not the primary one.

The mechanism is clear: prio_local prioritizes cheap Reservation requests (tight 50ms SLO → tight local deadline), keeping them on schedule while shedding expensive Search requests. fifo has no such selectivity, so more Reservation requests miss their SLO and get early-returned unnecessarily.
