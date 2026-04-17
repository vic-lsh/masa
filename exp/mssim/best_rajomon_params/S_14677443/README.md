# Rajomon best params — trace S_14677443

## Workload
- Trace: `trace-analysis/golden/S_14677443`
- SLO: 100 ms
- Saturation RPS: 650 (optimizer sweep: 500, 600, 800, 1000, 1300)
- 30s per RPS level, 15s warmup dropped at loadgen, 100 000 MAX_IN_FLIGHT

## Results by scheduling policy

Both runs used 10 Optuna TPE trials with `ac_rajomon` admission control. Post
bug-fix branch state: policy-param wiring, loadgen feature flags, loadgen-side
warmup drop, per-second client_shed rate.

| Policy | Best obj | Winning trial | Notes |
|--------|---------:|:-------------:|-------|
| `sched_fifo,ac_rajomon` | 2930.45 | trial 2 | init_price=810, latency_threshold=22002, tokens_left_init=14 |
| `sched_tailclipper,ac_rajomon` | **3392.07** | trial 9 | init_price=758, latency_threshold=8558, tokens_left_init=217 |

TailClipper beats FIFO by 300–1000 objective points on every identical trial.
On three trials they picked the exact same Rajomon params and differed only in
scheduling: TailClipper's oldest-first ordering pushed the objective up
consistently. See the per-policy `optimization_log.csv` for the full trial
tables.

## Per-policy directories

- `sched_fifo/best_params.json` + `optimization_log.csv`
- `sched_tailclipper/best_params.json` + `optimization_log.csv`

To replay any winner: drop its `best_params.json` into an experiment's input
dir as `policy_param.json`, pair with the matching `policies` file and
`gen_config.json`, then `uv run -m exp_runner run mssim <exp>`.
