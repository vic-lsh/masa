# mssim graph_branch experiments

Index of graphs benchmarked in this session, with the SLO and RPS sweep used.
Policies are fixed across runs (`sched_fifo,ac_rajomon`,
`sched_tailclipper,ac_rajomon`, `sched_pred,abort_slack,ac_pred,est_mean_var`).

Graphs live under `trace-analysis/graph_branch/<graph>/`. Configs under
`exp/mssim/in/<name>/`, results under `exp/mssim/plots/<name>/`.

| Graph | Experiment | SLO (ms) | RPS sweep | Notes |
|---|---|---:|---|---|
| S_86516878 | `cg8651_acCmp` | 52 | 300, 500, 800, 1000, 1200, 1500, 2000, 2500, 3000 | Heterogeneous workload (150 USER variants, top variant 30%). Big win for ours: ~1450 vs ~1085 TC, ~890 FIFO at high RPS. |
| S_14677443 | `s1467_acCmp_branch` | 100 | 200, 400, 800, 1000, 1200, 1400, 1500, 1600, 1800, 2000 | Loose SLO; all policies plateau ~865 — no differentiation. |
| S_14677443 | `s1467_acCmp_branch_slo30` | 30 | same | Tight-but-feasible SLO. Big win: ours ~770 plateau, Rajomon collapse to 270–500. |
| S_32048416 | `s3204_acCmp_branch` | 100 | same | Originally exposed a metastable lockout; after BCF + decay fix, ours plateaus ~395. CPU-saturated workload (98% of requests share one path). |
| S_51794709 | `s5179_acCmp_branch` | 100 | 800, 1500, 2100, 2400, 2700, 3500, 4500, 6000 | Saturation ~2300 RPS. Big win in deep overload: ours plateaus ~2700–3000 across 2.5× sat, tuned rajomon (fifo or tc) collapses to ~1340 at 6000 RPS. Cause: rajomon's price feedback over-sheds client-side (121k of 180k offered at 6000 RPS) while server has headroom (p99 88ms vs SLO 100ms). |

## Graph properties (relevant for policy comparison)

| Graph | Methods | Variants | USER variants | Top-USER prob | Frontend p99/p50 | Frontend variant cost spread |
|---|---:|---:|---:|---:|---:|---:|
| S_86516878 | 862 | 9412 | 150 | 30.5% | n/a (cheap) | 1.3× |
| S_14677443 | 16 | 34 | 12 | 50.9% | 2× | 5.0× |
| S_32048416 | 20 | 85 | 5 | 98.1% | 3.8× | 1.2× |
| S_51794709 | 295 | 513 | n/a | n/a | n/a | n/a |

## Lessons

- **Heterogeneous entry workload** (many USER variants, low top-variant probability) is the strongest predictor of our policy beating Rajomon. S_86516878 wins big; S_32048416 ties.
- **Tight-but-feasible SLO** is where priority scheduling shines. S_14677443 at SLO=100ms (intrinsic ≪ SLO) showed ~tie; same workload at SLO=30ms (queueing is the constraint) showed +400 RPS over baselines.
- **Loose SLO** (intrinsic ≪ SLO and no overload) → all policies degenerate to "admit everything"; goodput limited only by raw throughput. No differentiation possible.
- **Deep saturation** (e.g. S_32048416 hitting CPU ceiling at ~395 across all policies) limits the absolute headroom for any policy advantage.

## Workload selection guide

To stress-test our policy's structural advantages, prefer graphs with:
1. Many USER variants (>50), top variant <50% probability.
2. Large downstream cost spread across USER variants (≥3× p99 sum).
3. Set SLO to roughly **3–10×** the dominant variants' intrinsic p99.
4. Sweep RPS from low (100) to ≥2× the saturation point.
