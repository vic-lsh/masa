# tracebench graph_branch experiments

Index of graphs benchmarked in this session, with the SLO and RPS sweep used.
Policies are fixed across runs (`sched_fifo,ac_rajomon`,
`sched_tailclipper,ac_rajomon`, `sched_pred,abort_slack,ac_pred,est_mean_var`).

Graphs live under `trace-analysis/graph_branch/<graph>/`. Configs under
`exp/tracebench/in/<name>/`, results under `exp/tracebench/plots/<name>/`.

| Graph | Experiment | SLO (ms) | RPS sweep | Notes |
|---|---|---:|---|---|
| S_86516878 | `cg8651_acCmp` | 52 | 300, 500, 800, 1000, 1200, 1500, 2000, 2500, 3000 | Heterogeneous workload (150 USER variants, top variant 30%). Big win for ours: ~1450 vs ~1085 TC, ~890 FIFO at high RPS. |
| S_14677443 | `s1467_acCmp_branch` | 100 | 200, 400, 800, 1000, 1200, 1400, 1500, 1600, 1800, 2000 | Loose SLO; all policies plateau ~865 — no differentiation. |
| S_14677443 | `s1467_acCmp_branch_slo30` | 30 | same | Tight-but-feasible SLO. Big win: ours ~770 plateau, Rajomon collapse to 270–500. |
| S_32048416 | `s3204_acCmp_branch` | 100 | same | Originally exposed a metastable lockout; after BCF + decay fix, ours plateaus ~395. CPU-saturated workload (98% of requests share one path). |
| S_51794709 | `s5179_acCmp_branch` | 100 | 800, 1500, 2100, 2400, 2700, 3500, 4500, 6000 | Saturation ~2300 RPS. Big win in deep overload: ours plateaus ~2700–3000 across 2.5× sat, tuned rajomon (fifo or tc) collapses to ~1340 at 6000 RPS. Cause: rajomon's price feedback over-sheds client-side (121k of 180k offered at 6000 RPS) while server has headroom (p99 88ms vs SLO 100ms). |
| S_119456620 | `s1194_acCmp_branch` | 100 | 600, 1200, 1500, 1700, 2000, 2500, 3000, 3750 | Saturation ~1500 RPS (p99 sat ≈ 66ms under SLO=100ms). Tie at the knee (1500–1700: all within ±25 RPS). Big win in overload: ours plateaus and **climbs** with offered load — 2500: 1688 vs 1435 fifo / 1340 tc; 3000: 1794 vs 1386 / 1196; 3750: 1912 vs 1285 / 1006 (+627 vs fifo, +906 vs tc at 2.5× sat). Heterogeneous entry: top method MS_42200 has 16 variants, top variant only 21.3%. 600 RPS row is cold-start (all policies <90% goodput at well-below-saturation). Rajomon tuning (10 trials each for fifo and tc, warm-started) converged to the **same** params (s5179 seed: lat_thr=21673us, init_price=174, ts_init=7888) — the un-tuned 3-way run is already the tuned comparison. |
| S_58367703 | `s5836_acCmp_branch` | 100 | 20, 40, 60, 80, 100, 130, 160 | CPU-bound graph: saturates at only ~50 RPS because every non-leaf method busy-spins for `total_latency - fanout_elapsed` (`apps/tracebench/generic-service/src/core.rs:132`); leaf 50ms cap doesn't help. Mixed result with **untuned** rajomon (used s1467 template params): tc+rajomon wins the knee (80 RPS: 44.2 vs ours 36.0); ours wins deep overload (130 RPS: 42.4 vs 31.2 tc, 38.1 fifo). Intrinsic per-request p99 sits right at SLO=100ms (87% goodput at 10 RPS), so SLO competition is tight by construction. |

## Graph properties (relevant for policy comparison)

| Graph | Methods | Variants | USER variants | Top-USER prob | Frontend p99/p50 | Frontend variant cost spread |
|---|---:|---:|---:|---:|---:|---:|
| S_86516878 | 862 | 9412 | 150 | 30.5% | n/a (cheap) | 1.3× |
| S_14677443 | 16 | 34 | 12 | 50.9% | 2× | 5.0× |
| S_32048416 | 20 | 85 | 5 | 98.1% | 3.8× | 1.2× |
| S_51794709 | 295 | 513 | n/a | n/a | n/a | n/a |
| S_58367703 | 92 | 696 (90 in latency json) | n/a | n/a | top variant p99 230ms | 13 variants p99>50ms |
| S_119456620 | 170 | 398 | 16 (top method MS_42200) | 21.3% | top variant p99 26ms | top-5 p99 11.5–26ms |

## Lessons

- **Heterogeneous entry workload** (many USER variants, low top-variant probability) is the strongest predictor of our policy beating Rajomon. S_86516878 wins big; S_32048416 ties.
- **Tight-but-feasible SLO** is where priority scheduling shines. S_14677443 at SLO=100ms (intrinsic ≪ SLO) showed ~tie; same workload at SLO=30ms (queueing is the constraint) showed +400 RPS over baselines.
- **Loose SLO** (intrinsic ≪ SLO and no overload) → all policies degenerate to "admit everything"; goodput limited only by raw throughput. No differentiation possible.
- **Deep saturation** (e.g. S_32048416 hitting CPU ceiling at ~395 across all policies) limits the absolute headroom for any policy advantage.
- **Aggregate per-request CPU** is the saturation driver, not just leaf latency. Non-leaf methods busy-spin for the remaining `total_latency - fanout_elapsed` after children return (`apps/tracebench/generic-service/src/core.rs:132`), uncapped. A graph with many methods (S_58367703, 92 methods, mean variant p50 12ms) hits CPU saturation at very low RPS (~50) even when no single variant looks heavy. The 50ms cap only applies to leaves.

## Workload selection guide

To stress-test our policy's structural advantages, prefer graphs with:
1. Many USER variants (>50), top variant <50% probability.
2. Large downstream cost spread across USER variants (≥3× p99 sum).
3. Set SLO to roughly **3–10×** the dominant variants' intrinsic p99.
4. Sweep RPS from low (100) to ≥2× the saturation point.
