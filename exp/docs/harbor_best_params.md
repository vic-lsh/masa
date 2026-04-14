# harbor — best Rajomon parameters (backup reference)

Tuned on `exp/hotel/in/api_search/` (Search API, SLO=200ms, RPS 600/700/800/1000/1400) with `uv run -m exp_runner optimize hotel api_search --saturation-rps 700 --iterations 10`.

Raw optimizer outputs: `exp/hotel/out/_opt_rajomon_harbor/<policy>/best_params.json`.
Combined nested form: `exp/hotel/in/harbor_api_search/policy_param.json` and `exp/hotel/in/harbor_api_2/policy_param.json`.

## sched_fifo,ac_rajomon
Objective: 3188.67 (trial 4)
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

## sched_tailclipper,ac_rajomon
Objective: 3234.09 (trial 5)
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

## sched_pred,ac_rajomon,est_mean_var
Objective: 3221.53 (trial 5)
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

**Note:** tailclipper and sched_pred converged on identical parameter vectors because 10 iterations keeps Optuna in its deterministic random-startup phase. For truly independent per-policy tuning, re-run with `--iterations 30` or higher.

## Nested form (for multi-policy experiments)
Copy `exp/hotel/in/harbor_api_search/policy_param.json` into any new experiment directory that mixes these policies. The runner keys Rajomon params by scheduling-policy prefix (`sched_fifo`, `sched_tailclipper`, `sched_pred`).
