# Rajomon best params — trace S_14677443

## Source
- Trace: `trace-analysis/golden/S_14677443` (mssim)
- SLO: 100 ms
- Saturation RPS: 650 (0.8x–2.0x sweep: 500, 600, 800, 1000, 1300)
- Policy: `sched_fifo,ac_rajomon`
- Optimizer: 10-trial Optuna TPE, BO run 2026-04-15 on `vic/adc` branch at `b42e6656` (post-fixes: policy-param wiring, ac_rajomon loadgen feature flags, warmup dropped at loadgen).

## Best trial: 2, objective 2930.45

`best_params.json` holds the exact `{ "rajomon": {...} }` payload that the
RajomonOptimizer writes per trial. To replay: copy it into an experiment's
input dir as `policy_param.json` alongside `policies`, `gen_config.json`,
`mssim.json`, and `uv run -m exp_runner run mssim <exp>`.

## Full trial log
See `optimization_log.csv`. Trial 9 (2912.78) is a nearly-tied alternative
with distinct params — worth investigating as a second basin.
