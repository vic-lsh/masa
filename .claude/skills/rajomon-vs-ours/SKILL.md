---
name: rajomon-vs-ours
description: End-to-end head-to-head comparison of tuned Rajomon admission control baselines (sched_fifo,ac_rajomon and sched_tailclipper,ac_rajomon) against our policy (sched_pred,abort_slack,ac_pred,est_mean_var) on any Masa application (hotel, socialnet, synthetic, mssim). Trigger this skill whenever the user asks to compare Rajomon against our system, evaluate our admission control against Rajomon baselines, benchmark a new workload or trace against Rajomon, tune Rajomon for a specific workload and then head-to-head it, or anything phrased as "run the rajomon vs ours comparison" / "evaluate ours against rajomon on <app/trace>". Use even when the user doesn't name the skill explicitly — if they want a tuned-baseline-vs-our-policy head-to-head, this is it.
user_invocable: true
---

# Rajomon vs. Our Policy — comparison workflow

Runs a fair head-to-head between **freshly tuned** Rajomon baselines and our
policy (`sched_pred,abort_slack,ac_pred,est_mean_var`) on a user-supplied
workload. Works for any app under `apps/` (hotel, socialnet, synthetic,
mssim). The skill drives the model through four phases; each phase has a
long-running command, so **always pause and ask the user for permission
before kicking off experiments**.

## Inputs

Ask the user for:

- **App**: `hotel`, `socialnet`, `synthetic`, or `mssim`.
- **Base experiment config**: an existing config directory under
  `exp/<app>/in/<base>/` that defines the workload, SLO, RPS range, API
  selection, etc. The skill copies topology/workload from this base; it does
  NOT invent a workload from scratch. Example bases:
  `rajomon_hotel`, `s3204_acCmp`, `shared01`.
- **Comparison name** (optional): short handle used for the generated
  experiment dirs and tuned-params dir. Default: `<base>_acCmp`. Keep short
  — it becomes a directory name.

## Config rules that apply to every phase

- **Disable `MaxInFlight`** in every generated `gen_config.json`. A client
  in-flight cap hides the exact behavior we're measuring (admission control
  and scheduling under overload). If the base config has
  `"MaxInFlight": <N>`, set it to a very large value (e.g. `100000`) or
  remove the cap if the app supports omission. This applies to Phase 1
  (saturation), Phase 2 (optimizer's copy of the base), and Phase 3 (the
  comparison itself).
- **Always use `sched_fifo` / `sched_pred` / etc.**, not the legacy
  `fifo` / `prio_local` / `prio_oldest` names.

If the user gestures vaguely ("run it on the two-trace workload"), ask once
for the exact base config path — don't guess.

## Why this flow exists

Rajomon is extremely sensitive to its parameters, and tuned params do **not**
generalize across workloads. Running our policy against a generic or
previously-tuned Rajomon config would mostly be measuring Rajomon's
misconfiguration. The four-phase structure re-tunes Rajomon on the exact
workload it will be measured on, so the comparison is honest.

## Phase 1 — Saturation finding

Goal: find an RPS anchor for the optimizer sweep. Use the `saturation-finder`
skill if available — otherwise create a single-policy config and sweep.

**Config shape is app-specific.** Start by copying the user's base config
into `exp/<app>/in/<name>_sat/`, then:

- Overwrite `policies` with a single line: `sched_fifo` (no admission
  control, no early return — this measures the natural goodput curve
  uncontaminated by overload-control interventions).
- Keep the app-specific files as-is (`hotel.json`, `mssim.json`, etc.) —
  they carry the topology and SLO.
- In `gen_config.json`, set a broad `Rps` sweep that clearly spans from
  underload to collapse. Leave `DurationSecs`, `WarmupSecs`, and other
  workload knobs at the base's values unless the user wants them changed.

Ask the user: "Ready to run the saturation sweep? ~8–10 min." Then:

```bash
uv run -m exp_runner run <app> <name>_sat --plot
```

When it completes, read `exp/<app>/plots/<name>_sat/goodput_absolute_avg.csv`
(or the app's equivalent — hotel/socialnet produce
`goodput_ALL_aggregated.csv`, mssim produces `goodput_absolute_avg.csv`) and
pick **the RPS with the highest goodput**. Beyond that point goodput declines
under overload. Report the peak RPS and goodput to the user, and if the peak
is at the high end of the sweep, warn that the sweep may be truncated and
offer to re-sweep higher.

**Legacy name warning:** use `sched_fifo`, not the deprecated `fifo`. Older
configs in `exp/` may still have `fifo` / `prio_local` / `prio_oldest` —
replace them with the new `sched_*` / `ac_*` / `est_*` flag names.

## Phase 2 — Tune Rajomon for each scheduler

Run two separate 10-trial Bayesian optimizations, both targeted at the Phase
1 saturation point. **Always clear Optuna state between runs** — the
optimizer writes to a shared `exp/<app>/out/_opt_rajomon/` dir and resumes
silently if trials already exist, which will no-op if the previous run hit
the trial cap.

Before each run:

```bash
rm -rf exp/<app>/out/_opt_rajomon exp/<app>/in/_opt_rajomon_* exp/<app>/out/_opt_rajomon_*
```

Ask the user before each: "Ready to run the 10-trial <policy> optimization?
~45 min."

**Fifo:**

```bash
uv run -m exp_runner optimize <app> <name>_acCmp \
  --policy sched_fifo,ac_rajomon \
  --saturation-rps <sat> \
  --iterations 10 \
  [--warm-start <path to previously-tuned fifo best_params.json, if any>] \
  --output exp/<app>/best_rajomon_params/<name>/sched_fifo/best_params.json
```

**Tailclipper:**

```bash
uv run -m exp_runner optimize <app> <name>_acCmp \
  --policy sched_tailclipper,ac_rajomon \
  --saturation-rps <sat> \
  --iterations 10 \
  [--warm-start <path to previously-tuned tailclipper best_params.json, if any>] \
  --output exp/<app>/best_rajomon_params/<name>/sched_tailclipper/best_params.json
```

Notes:
- The optimizer reads topology from `<name>_acCmp`. Either create a stub
  of that config before Phase 2 (copy the base + single-policy `policies`
  file) or create it in Phase 3 and re-order the phases as needed — the
  important thing is that `exp/<app>/in/<name>_acCmp/` exists and has a
  valid app-specific topology file.
- Warm-starts are optional but save trials. Look under
  `exp/<app>/best_rajomon_params/` for any prior winners on the same app;
  if the workload is substantially different, warm-starting from the wrong
  regime can still help Optuna explore but don't fixate on it.
- The optimizer's sweep is `[0.8, 1.0, 1.2, 1.5, 2.0]*sat` rounded to the
  nearest 50 via `exp_runner/runner/optimizer.py::generate_rps_sweep`. At
  very low saturation (sat ≤ 100) the sweep may collapse duplicate points
  — sanity-check the sweep printed at optimizer startup.
- After each run, show the user the top ~3 trials from
  `exp/<app>/out/_opt_rajomon/optimization_log.csv` and confirm the best
  params were saved to the `--output` path.
- Hygiene: archive the per-policy log
  (`mv exp/<app>/out/_opt_rajomon exp/<app>/out/_opt_rajomon_<policy>_sat<sat>`)
  before clearing for the next policy, so both trial logs are preserved.

## Phase 3 — Assemble the comparison experiment

Create (or overwrite) `exp/<app>/in/<name>_acCmp/` by copying the base
config and editing:

- Keep the app-specific topology file unchanged (`hotel.json`,
  `socialnet.json`, `mssim.json`, etc.) — same workload as the base.
- `gen_config.json`: Adjust `Rps` so the sweep **hits at least ~3x
  saturation at the top end** (e.g. sat=600 → max RPS ≥ 1800). Span from
  ~0.3x to ~3x saturation with ~10 points, denser around and above
  saturation. The overload regime is where the comparison matters most —
  if the top end doesn't reach 3x, the plot will truncate before our policy
  opens its biggest lead. Other fields inherit from the base unless the
  user asks. Confirm `MaxInFlight` is disabled (see config rules above).
- `policies`: exactly three lines (use any `abort_*` / `est_*` modifiers the
  app conventionally pairs with these — hotel and socialnet typically add
  `abort_slo`; mssim usually runs without it; inspect the base's existing
  `policies` file for the app convention):

  ```
  sched_fifo,ac_rajomon[,abort_slo]
  sched_tailclipper,ac_rajomon[,abort_slo]
  sched_pred,abort_slack,ac_pred,est_mean_var
  ```

- `policy_param.json`: nested per-scheduling-policy structure, resolved by
  `exp_runner/runner/apps/utils.py::resolve_policy_params`:

  ```json
  {
    "rajomon": {
      "sched_fifo":         { <contents of best_rajomon_params/<name>/sched_fifo/best_params.json> },
      "sched_tailclipper":  { <contents of best_rajomon_params/<name>/sched_tailclipper/best_params.json> }
    },
    "pred": {
      "tau_er": 2.0,
      "estimator_k": 0.0
    }
  }
  ```

  `pred` defaults (`tau_er=2.0, estimator_k=0.0`) match the post-AIMD-removal
  defaults in `libs/masa-policy/src/policy_params.rs`. Keep them unless the
  user explicitly wants to sweep pred params.

Show the user the final `gen_config.json`, `policies`, and `policy_param.json`
and confirm before running.

## Phase 4 — Run the e2e comparison

Ask: "Ready to run the e2e comparison? ~20 min with cached docker images."

```bash
uv run -m exp_runner run <app> <name>_acCmp --plot
```

When done, read the aggregated goodput CSV and report a table with:

- Absolute goodput per policy per RPS.
- Percentage gain of our policy over each baseline: `(ours/baseline - 1)*100`.

The CSV path varies by app:
- `exp/<app>/plots/<name>_acCmp/goodput_absolute_avg.csv` (mssim)
- `exp/<app>/plots/<name>_acCmp/goodput_ALL_aggregated.csv` (hotel, socialnet)

Call out qualitatively where our policy dominates (usually the overloaded
regime, RPS > saturation). If tailclipper+rajomon regresses vs fifo+rajomon
at any RPS, flag it — that usually means the tuned baseline params don't
generalize above the sweep range used for tuning.

## Things that have bitten past runs

- **Optuna state leakage** — the optimizer resumes from whatever is in
  `exp/<app>/out/_opt_rajomon/`. Forgetting to `rm -rf` between policies
  leaves the previous policy's trials as the "history" and the run silently
  completes in zero seconds. Always clear.
- **Stale docker containers** — if a previous experiment crashed, stale
  containers with `<app>-*` / `mssim-*` prefixes may block new runs. If you
  see weird container creation errors: `docker ps --format '{{.Names}}' |
  grep <app> | xargs -r docker rm -f`.
- **Goodput denominator bug (mssim only)** — pre-commit `66658a85`,
  `_compute_goodput` in `exp_runner/runner/plotting/mssim.py` subtracted
  warmup from the denominator, inflating reported goodput 2x. On a branch
  that predates that fix, the numbers from `goodput_absolute_avg.csv` are
  not trustworthy; `goodput_timeline.csv` values are correct. Hotel and
  socialnet don't share this bug.
- **Low saturation + nearest-50 rounding** — at sat ≤ 100 the 0.8x/1.0x/1.2x
  sweep points can collapse. If the optimizer startup log shows duplicate
  RPS values, bump the saturation guess or note that the low end is
  effectively a repeated measurement.
- **Tuning that regresses above its own sweep range** — the optimizer's
  objective sums over `[0.8x..2.0x]*sat`. Params that win on that sweep may
  regress above `2.0x` in the e2e run (which is why the e2e sweep extends
  to `3x`). That's a feature (it surfaces where the baseline was overfit),
  but flag it to the user explicitly.
- **App-specific config divergence** — older hotel/socialnet configs use
  `Apis`/`Slos`/`Addr` in `gen_config.json`; mssim uses `MaxInFlight` and
  the topology lives in `<app>.json`/`mssim.json`. Always copy the base's
  config shape rather than assembling one from scratch.

## Summary of files touched

- `exp/<app>/in/<name>_sat/` — saturation-finding config (Phase 1).
- `exp/<app>/best_rajomon_params/<name>/sched_fifo/best_params.json` —
  tuned fifo params (Phase 2).
- `exp/<app>/best_rajomon_params/<name>/sched_tailclipper/best_params.json`
  — tuned tailclipper params (Phase 2).
- `exp/<app>/in/<name>_acCmp/` — comparison experiment config (Phase 3).
- `exp/<app>/plots/<name>_acCmp/goodput_*.csv` — final results (Phase 4).
