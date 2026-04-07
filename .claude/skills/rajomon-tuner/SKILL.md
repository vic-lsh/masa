---
name: rajomon-tuner
description: End-to-end Rajomon admission control parameter optimization workflow
user_invocable: true
---

# Rajomon Parameter Tuner

Orchestrates Bayesian optimization of Rajomon admission control parameters for Masa applications.

## Prerequisites
- Docker must be running
- The target app's base experiment must exist (e.g., `rajomon_hotel` in `exp/hotel/in/`)

## Workflow

### 1. Determine Saturation RPS
If the user doesn't provide a saturation RPS value:
- Check if a saturation-finder experiment has been run for the app
- Or use known values from existing experiments (e.g., hotel's `rajomon_hotel` uses RPS up to 2000)
- The saturation RPS is the max sustainable RPS before significant SLO violations

### 2. Run the Optimizer
```bash
uv run -m exp_runner optimize <app> <experiment_base> \
    --policy <policy> \
    --saturation-rps <rps> \
    --iterations <n>
```

Example for hotel:
```bash
uv run -m exp_runner optimize hotel rajomon_hotel \
    --policy "sched_slo,ac_rajomon,abort_slo" \
    --saturation-rps 1800 \
    --iterations 30
```

Monitor the log output for progress. Each iteration takes ~1-3 minutes depending on the app.

### 3. Validate Results
After optimization completes:
1. Check `exp/<app>/out/_opt_rajomon/best_params.json` for the optimized parameters
2. Check `exp/<app>/out/_opt_rajomon/optimization_log.csv` for the trial history
3. Run a full-length experiment with the best params to validate:
```bash
cp exp/<app>/out/_opt_rajomon/best_params.json exp/<app>/in/<experiment>/policy_param.json
uv run -m exp_runner run <app> <experiment> --plot
```

### 4. Cross-App Transfer (Warm-Starting)
The Rajomon paper (Figure 10a) shows low parameter variability across apps. After optimizing one app, warm-start the next:

Recommended order: hotel -> socialnet -> mssim -> synthetic

```bash
uv run -m exp_runner optimize socialnet <socialnet_experiment> \
    --policy "sched_slo,ac_rajomon,abort_slo" \
    --saturation-rps <rps> \
    --warm-start exp/hotel/out/_opt_rajomon/best_params.json \
    --iterations 15
```

### 5. Produce Final policy_param.json
The optimizer outputs flat params (single scheduling policy). To produce the nested format expected by multi-policy experiments:

```json
{
  "rajomon": {
    "sched_slo": { ... optimized params ... },
    "sched_fifo": { ... optimized params for fifo ... },
    "sched_pred": { ... optimized params for pred ... }
  }
}
```

Run the optimizer separately for each scheduling policy variant, then combine the results.

### 6. Resumability
The optimizer uses SQLite storage (`exp/<app>/out/_opt_rajomon/optuna.db`). If interrupted, re-running the same command resumes from where it left off. To start fresh, delete the `_opt_rajomon` output directory.

## Key Parameters
- `--iterations 30`: Default, takes ~60-90 min. Use 5-10 for quick tests.
- `--penalty-weight 10.0`: Controls goodput vs latency trade-off. Higher values penalize SLO violations more.
- `--saturation-rps`: Critical parameter. Too low = optimizer won't see overload behavior. Too high = everything fails.
