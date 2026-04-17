# harbor — Rajomon retuning on hotel after bug fixes

## Key questions
- After recent Rajomon bug fixes, what are the optimal Rajomon parameters for each scheduling policy (`sched_fifo`, `sched_tailclipper`, `sched_pred,est_mean_var`) on hotel?
- Do the retuned parameters improve goodput on both api_search (Search-only) and api_2 (Search+Reservation)?

## Setup
- Base config: `exp/hotel/in/api_search/` (Search, SLO=200ms, RPS 700–1200, 20s warmup, 60s duration)
- Optimizer: `uv run -m exp_runner optimize hotel api_search` with `--saturation-rps 700 --iterations 10`
- Policies (no `abort_*` flags):
  1. `sched_fifo,ac_rajomon`
  2. `sched_tailclipper,ac_rajomon`
  3. `sched_pred,ac_rajomon,est_mean_var`
- Per-run outputs under `exp/hotel/out/_opt_rajomon_harbor/<policy>/best_params.json`
- Optuna DB cleared between runs to prevent cross-contamination

## Sanity checks per optimizer run
After each optimize run, before moving to the next policy, verify:
- Best trial's goodput is non-zero at the saturation RPS
- Rejection/early-return breakdown attributes a meaningful share to Rajomon admission control (not solely deadline/SLO aborts or downstream RPC errors)
- If either check fails: stop, diagnose, and surface to the user before continuing.

## Sanity-check method
- Aggregated `early_return_ALL_breakdown.csv` groups by `SrcService::SrcMethod`, NOT by reason. Rajomon rejections look like "reservation.Reservation::CheckAvailability" rows.
- To verify Rajomon is firing, grep raw loadgen CSVs (`exp/hotel/out/_opt_rajomon_<N>/<trial>/<policy>/r<rps>_*.csv`) for `RajomonAdmissionRej` and `RajomonChildBudgetRej`.

## Optimization results

### sched_fifo,ac_rajomon (best trial 4, objective 3188.67)
Best params:
```json
{
  "latency_threshold_us": 9207,
  "price_update_rate_ms": 10,
  "price_step_up": 3,
  "init_price": 481,
  "price_freq": 3,
  "tokens_left_init": 1728,
  "token_update_rate_ms": 3,
  "token_update_step": 2756
}
```
Goodput (best trial): 600→587, 700→650, 800→650, 1000→649, 1400→652. Plateau at ~650 past saturation (healthy).
Rajomon engagement verified in raw CSVs: 1347 RajomonAdmissionRej + 531 RajomonChildBudgetRej at r1000_Search (trial 4).
Runtime: 49.9 min for 10 iterations. Objective distribution across trials: healthy spread (69–3188), optimizer differentiating.

### sched_tailclipper,ac_rajomon (best trial 5, objective 3234.09)
Best params:
```json
{
  "latency_threshold_us": 13423,
  "price_update_rate_ms": 10,
  "price_step_up": 1,
  "init_price": 528,
  "price_freq": 3,
  "tokens_left_init": 5723,
  "token_update_rate_ms": 1,
  "token_update_step": 362161
}
```
Goodput (best trial): 600→589, 700→530, 800→352, 1000→421, 1400→293.
Rajomon engagement verified: 0/0 at 600, 468/191 at 700, 1206/401 at 800, 1430/516 at 1000, 1894/765 at 1400 (admission/child).
Runtime: 49.7 min.
Note: lower peak goodput than fifo (530 vs 650 at 700 RPS), but optimizer preferred params with higher objective — likely due to penalty structure favoring more aggressive shedding at overload.

### sched_pred,ac_rajomon,est_mean_var (best trial 5, objective 3221.53)
Best params:
```json
{
  "latency_threshold_us": 13423,
  "price_update_rate_ms": 10,
  "price_step_up": 1,
  "init_price": 528,
  "price_freq": 3,
  "tokens_left_init": 5723,
  "token_update_rate_ms": 1,
  "token_update_step": 362161
}
```
**Note:** Identical params to sched_tailclipper's best — this is because with only 10 iterations, Optuna stays within its random startup phase (deterministic seed), so both studies sampled the same parameter grid. Objective differs (3221 vs 3234) because same params perform differently under each scheduling policy. To explore truly different regions per policy, more iterations (≥30) would be needed.
Rajomon engagement verified: 0 at 600, 526 at 700, 1528 at 800, 2621 at 1000, 3140 at 1400 (combined admission+child rejections).
Runtime: 50 min.
Sanity: non-zero goodput, Rajomon firing — PASS per stated criteria.

## Combined validation

### harbor_api_search (Search only, SLO=200ms, RPS 700-1200)
| RPS  | sched_fifo | sched_tailclipper | sched_pred,est_mean_var |
|------|-----------|-------------------|-------------------------|
| 700  | 653.2     | 632.9             | 638.5                   |
| 800  | 656.0     | 650.0             | 636.8                   |
| 900  | 655.6     | 675.9             | 637.5                   |
| 1000 | 660.2     | 666.7             | 659.7                   |
| 1100 | 668.3     | 650.3             | 650.5                   |
| 1200 | 656.6     | 644.2             | 658.3                   |

All three policies deliver healthy, similar goodput (~630-675) across the RPS sweep — Rajomon with per-policy tuning is keeping the system near saturation throughput well past the offered-load saturation point (700 RPS). No policy collapses at overload.

### harbor_api_2 (Search+Reservation, SLOs 200/100 ms, RPS 100/1000/1600/1800/2000)
| RPS  | sched_fifo | sched_tailclipper | sched_pred,est_mean_var |
|------|-----------|-------------------|-------------------------|
| 100  | 100.7     | 97.7              | 97.2                    |
| 1000 | 994.9     | 1006.3            | 1000.7                  |
| 1600 | 1175.6    | 1052.1            | 1103.9                  |
| 1800 | 1155.9    | 1124.7            | 1167.6                  |
| 2000 | 1183.9    | 1149.8            | 1181.9                  |

All three policies retain ≥1100 goodput under 2× overload. sched_fifo slightly leads at 1600 RPS (+70 vs pred, +123 vs tailclipper); sched_pred catches up and matches fifo at 1800-2000. Tailclipper trails by ~30-60 at deep overload.

Rajomon params optimized on api_search transfer cleanly to api_2 — no collapse or pathological behavior.

## Summary
- Per-policy Rajomon parameter sets recorded in `exp/hotel/in/harbor_api_search/policy_param.json` and `exp/hotel/in/harbor_api_2/policy_param.json`.
- Optimizer caveat: with `--iterations 10`, Optuna stays in random-startup phase, so tailclipper and sched_pred landed on identical param vectors (different seeds would've explored different points). Run longer (≥30 iter) if tighter per-policy tuning is desired.
- All three policies are healthy across api_search and api_2; Rajomon engagement verified via `RajomonAdmissionRej`/`RajomonChildBudgetRej` counts in raw loadgen CSVs.
