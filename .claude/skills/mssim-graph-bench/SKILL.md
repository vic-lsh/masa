---
name: mssim-graph-bench
description: End-to-end benchmark of a mssim call graph from `trace-analysis/graph_branch/<graph>/` against tuned Rajomon baselines. Probes saturation with sched_fifo, picks an SLO at the latency knee, sets a sweep that spans underload to ~2.5x overload, tunes Rajomon for both sched_fifo and sched_tailclipper, and runs the three-way head-to-head (`sched_fifo,ac_rajomon` | `sched_tailclipper,ac_rajomon` | `sched_pred,abort_slack,ac_pred,est_mean_var`). Trigger this skill whenever the user asks to evaluate a mssim graph, benchmark a new call graph, "try graph X", "test graph X end-to-end", "find a graph that works for our system", or anything that asks to take a graph from `trace-analysis/graph_branch/` from inspection through final comparison. Use proactively even when the user only names a graph and says "run it" — they almost always want the full e2e flow, not just one ad-hoc run.
user_invocable: true
---

# mssim graph end-to-end benchmark

Given a graph in `trace-analysis/graph_branch/<graph>/`, drive the full
benchmark pipeline: inspect → probe saturation → pick SLO → validate ours →
tune Rajomon (fifo + tailclipper) → 3-way head-to-head → report. The skill
codifies the loop the user has run by hand many times, including the cold-
start traps and the shared-Optuna-dir bug.

Each phase has a long-running command — **always confirm with the user before
launching anything that takes more than a couple minutes** (saturation probe,
optimizer runs, final comparison). Cheap commands (graph inspection, config
edits) don't need confirmation.

## Inputs

- **Graph name** (required): a directory under `trace-analysis/graph_branch/`,
  e.g. `S_51794709`. If the user only says "try a new graph" without naming
  one, hand off to the `mssim-graph-picker` skill first to pick a
  candidate, then resume here.
- **Experiment handle** (optional): short slug for the config dir. Default:
  first 4 digits of the graph ID prefixed with `s`, suffixed with
  `_acCmp_branch`. Example: `S_51794709` → `s5179_acCmp_branch`.

## Phase 0 — Graph inspection (cheap, no confirm needed)

Before spinning anything up, check that the graph is plausibly benchmarkable.
Heavy graphs waste hours; the goal here is to bail early when the graph is
unsuitable.

Read three things:

1. **`trace-analysis/graph_branch/summary.csv`** — find the row for this graph.
   Note `method_count` and `variant_count`.
2. **`trace-analysis/graph_branch/<graph>/latency_percentiles.json`** — extract
   the top-10 variants by p99. Structure: `{method: {variant: {percentile_str:
   value_ms, ...}}}`. Percentile keys are strings like `"50"`, `"99"`, `"100"`
   and values are in milliseconds.
3. **`trace-analysis/graph_branch/<graph>/interface_distribution.json`** —
   per-method variant counts.

Use these heuristics to decide whether to proceed:

- **Too heavy** — if the worst variant p99 exceeds ~400ms or the chain implies
  natural per-request p99 above ~200ms (multiple long methods in a deep
  chain), the leaf-spin cap (50ms total leaf latency, set in
  `apps/mssim/generic-service/src/core.rs`) won't save you. Saturation will
  hit at <50 RPS and goodput will collapse before the comparison can be
  meaningful. Tell the user, recommend a different graph.
- **Too small / trivial** — if `method_count <= ~20` and intrinsic p99 < 20ms
  across the board, saturation will be very high (>3000 RPS) and the
  comparison will mostly measure raw throughput. Still runnable, but warn the
  user.
- **Sweet spot** — `100 <= method_count <= 400`, top variant p99 in the
  10–100ms range, reasonable variant heterogeneity. This is where the
  comparison is interesting.

Report the inspection findings to the user in a short table (methods,
variants, top-3 p99 variants) before asking to proceed.

## Phase 1 — Create base experiment dir + saturation probe

Create `exp/mssim/in/<handle>/` with four files. Copy `policy_param.json`
from any existing mssim config (`s1467_acCmp_branch` is a fine template).

```
exp/mssim/in/<handle>/
├── mssim.json            # callgraph_dirs + slo_ms
├── gen_config.json       # broad initial Rps sweep
├── policies              # SINGLE LINE: sched_fifo
└── policy_param.json     # copy from s1467_acCmp_branch
```

**`mssim.json`:**

```json
{
    "callgraph_dirs": ["trace-analysis/graph_branch/<graph>"],
    "slo_ms": 100,
    "orchestrator": "localhost:50051",
    "replay_path": null,
    "stats_interval_sec": 2,
    "extra_env": {}
}
```

Pick the initial `slo_ms` based on Phase 0 inspection: 100ms is a fine
default; raise to 500ms if intrinsic p99 looks high. The SLO is finalized
after Phase 2.

**`gen_config.json`** for the broad probe:

```json
{
    "Repeats": 1,
    "Rps": [100, 300, 600, 1000, 1500, 2000],
    "DurationSecs": 30,
    "WarmupSecs": 0,
    "MaxInFlight": 0
}
```

Always `MaxInFlight: 0` — the cap hides admission/scheduling behavior.

Confirm with user, then run:

```bash
uv run -m exp_runner run mssim <handle> --plot
```

Read `exp/mssim/plots/<handle>/goodput_fraction_avg.csv` and the per-RPS
latency distributions from
`exp/mssim/out/<handle>/0/sched_fifo/r<RPS>_<graph>.csv`. The CSV has columns
`api,request_id,slo,start_at,deadline,latency,error,q_lat_init,q_lat_resume`.
**Compute p99 only over rows where `error == "/None"`** — the literal string
`/None` marks a successful response. Anything else is an error/early-return.

### Reading the probe

You're looking for the **knee**: the first RPS where p99 jumps sharply (e.g.
20ms → 200ms across one step). That's saturation.

Common patterns and how to react:

- **Everything healthy** (e.g. p99 < 30ms at top RPS, goodput ≈ 100%) — sweep
  was too low. Push higher: `[3000, 5000, 7000, 9000, 11000, 14000, 17000,
  20000]` and rerun.
- **Everything collapsed** (e.g. p99 > 1s at lowest RPS) — sweep was too high
  OR the graph is intrinsically too heavy. First lower the sweep to
  `[20, 50, 100, 200, 400]`. If even `20 RPS` looks bad, the graph is too
  heavy — bail and tell the user.
- **Sharp transition** (e.g. p99 = 25ms at 2000 RPS, 250ms at 2500 RPS,
  multi-second at 3000 RPS) — saturation is between the healthy and
  collapsed point. Tighten with denser steps around it
  (`[2000, 2200, 2400, 2600, 2800]`) and rerun. Aim for ≤ 200 RPS resolution
  on the knee.

### Cold-start artifact (very common)

The very first RPS level after a fresh docker startup often shows pathological
p99 even at low load — typically because the JIT/warmup eats most of the
window and the loadgen flush-bursts at the end. Symptoms:

- p99 in the hundreds of milliseconds at the first level only, with normal
  values from the second level onward.
- Or: `n` (sample count) at level 1 is much larger than `RPS * DurationSecs`,
  meaning the loadgen flushed pent-up requests after warmup.

When this happens, **ignore the first level** when assessing saturation.
Don't conclude saturation lies below the first level just because the
first-level p99 looked bad — re-probe with a higher floor or `WarmupSecs > 0`
if the user wants confirmation.

## Phase 2 — Pick SLO and final RPS sweep

Once you've located saturation:

- **SLO**: round the natural p99 at saturation up to a clean number, then
  pad. Example: p99 ≈ 25ms at saturation → set `slo_ms: 100`. The pad gives
  the priority/admission-control policies room to operate without all three
  hitting the wall together.
- **Final Rps sweep**: 6–8 points from ~0.4× saturation to ~2.5–3× saturation,
  denser around saturation, sparser in deep overload. Example for
  saturation = 2300:

  `[800, 1500, 2100, 2400, 2700, 3500, 4500, 6000]`

  The high end matters — that's where ours opens its biggest lead. If the
  sweep tops out at < 2× saturation, ours and rajomon may look comparable
  even when ours dominates above 2.5×.

Update `mssim.json` with the chosen SLO, and `gen_config.json` with the final
sweep.

## Phase 3 — Validate ours plateaus

Before tuning Rajomon, sanity-check that ours behaves on this workload —
otherwise no point comparing. Set policies to ours-only:

```
sched_pred,abort_slack,ac_pred,est_mean_var
```

Run:

```bash
uv run -m exp_runner run mssim <handle> --plot
```

Read the goodput CSV. **What "good" looks like:**

- Linear scaling up to saturation (goodput ≈ RPS).
- Plateau in overload (goodput stays within ~10% of peak as RPS rises 2–3×).
- p99 of admitted-OK requests stays under SLO across the sweep.
- Shed fraction (rows with `error` containing `EarlyReturn`) rises smoothly
  with overload — no sudden cliff.

**What's bad:**

- Goodput collapses past saturation instead of plateauing → policy isn't
  shedding fast enough, or the workload is unstable. Maybe re-check Phase 1
  conclusions.
- p99 climbs into hundreds of ms across the board → SLO too tight, or the
  graph has a long-tail variant our estimator can't catch. Consider raising
  SLO.

If ours plateaus well, proceed. If not, report to the user and stop — the
comparison won't be honest if our policy is misbehaving.

## Phase 4 — Tune Rajomon for both schedulers

**Critical: clear the Optuna state between policies.** The optimizer writes
to `exp/mssim/out/_opt_rajomon/` and silently resumes if trials already
exist. If you tune fifo and then run the tailclipper command without
clearing, you get fifo's params back instantly.

Run sequentially. Confirm with user before each (each is ~30–45 min for 10
trials).

**Fifo first:**

```bash
uv run -m exp_runner optimize mssim <handle> \
  --policy "sched_fifo,ac_rajomon" \
  --saturation-rps <sat> \
  --iterations 10
```

When done, archive the result and the best params:

```bash
mv exp/mssim/out/_opt_rajomon exp/mssim/out/_opt_rajomon_fifo
```

**Tailclipper, warm-started from fifo's winner:**

```bash
uv run -m exp_runner optimize mssim <handle> \
  --policy "sched_tailclipper,ac_rajomon" \
  --saturation-rps <sat> \
  --iterations 10 \
  --warm-start exp/mssim/out/_opt_rajomon_fifo/best_params.json
```

```bash
mv exp/mssim/out/_opt_rajomon exp/mssim/out/_opt_rajomon_tc
```

After each run, read `optimization_log.csv` and report the top trial's
objective. The objective is summed goodput across the optimizer's internal
sweep (`[0.8, 1.0, 1.2, 1.5, 2.0] * sat`). A good fifo objective on a
non-trivial workload is typically 4–10× saturation.

If tailclipper's best is the warm-start trial unchanged, that's fine — it
just means fifo's params transfer cleanly.

## Phase 5 — 3-way head-to-head

Update three files:

**`policies`:**

```
sched_fifo,ac_rajomon
sched_tailclipper,ac_rajomon
sched_pred,abort_slack,ac_pred,est_mean_var
```

**`policy_param.json`** — nest both tuned configs:

```json
{
  "rajomon": {
    "sched_fifo":        { ...contents of _opt_rajomon_fifo/best_params.json... },
    "sched_tailclipper": { ...contents of _opt_rajomon_tc/best_params.json... }
  },
  "pred": {
    "tau_er": 2.0,
    "estimator_k": 0.0
  }
}
```

`gen_config.json` already has the final sweep from Phase 2 — no changes.

Confirm with the user, then run:

```bash
uv run -m exp_runner run mssim <handle> --plot
```

## Phase 6 — Report

Read `exp/mssim/plots/<handle>/goodput_absolute_avg.csv` and produce a table:

| RPS | fifo+rajomon | tc+rajomon | ours | Δ vs fifo | Δ vs tc |

Highlight where ours wins, separately for the saturation knee, light
overload, and deep overload regimes. Note any cold-start row (first RPS) as
suspect.

If the comparison is interesting, offer to update
`docs/MSSIM_GRAPH_BENCH.md` with a row for this graph (Graph, Experiment,
SLO, RPS sweep, headline result).

Also point the user at the generated plots:

- `exp/mssim/plots/<handle>/goodput_absolute_avg.png`
- `exp/mssim/plots/<handle>/goodput_fraction_avg.png`
- `exp/mssim/plots/<handle>/latency_percentiles_avg.png`
- `exp/mssim/plots/<handle>/latency_cdf_<rps>rps_avg.png` (per-RPS CDFs)

## Things that have bitten past runs

- **Shared `_opt_rajomon/` dir** — see Phase 4. Always `mv` between policies.
  Symptom: tailclipper "completes" in seconds with the same best params as
  fifo.
- **Cold-start at first RPS** — see Phase 1. First level after fresh docker
  often shows phantom saturation. Drop it from the analysis.
- **`error` field semantics** — empty string is NOT an OK marker. `/None`
  is. Anything else (including `EarlyReturn`, `PredAdmissionRej`, gRPC
  errors) is a non-OK row that should not contribute to OK-only latency
  percentiles.
- **`sent` count drops at high overload** — at very high target RPS, the
  loadgen may not actually achieve the target because admission rejects so
  early that it backs off. Goodput numbers are still valid; just don't
  expect `n ≈ RPS * DurationSecs` past 2× saturation.
- **Stale docker containers** — if a previous run was killed, `docker ps |
  grep <handle>` may show ghosts. Clean with
  `docker compose -p <project_name> down`. Project names look like
  `mssim-<handle short>--<hash>`.
- **Heavy graph that "almost works"** — sometimes a graph saturates at low
  RPS but goodput is still nonzero. Tempting to push through, but if
  natural p99 exceeds the SLO at the lowest RPS, no policy can help.
  Recognize and bail.
- **Loose SLO yields no differentiation** — if intrinsic p99 ≪ SLO, all
  three policies degenerate to "admit everything" and tie. Tighten the SLO
  to expose priority-scheduling advantage. The S_14677443 case: SLO=100ms
  ties everyone at ~865 goodput; same workload at SLO=30ms gives ours +400
  RPS over baselines.
- **Homogeneous workload caps the win** — if one USER variant accounts for
  >95% of traffic (e.g. S_32048416), all admission policies have the same
  job, and the priority-scheduling advantage shrinks. Heterogeneous
  workloads (many USER variants, low top-variant probability) are where
  ours wins big.

## Summary of files touched

- `exp/mssim/in/<handle>/` — all configs.
- `exp/mssim/out/<handle>/` — raw results per policy.
- `exp/mssim/out/_opt_rajomon_fifo/`, `_opt_rajomon_tc/` — per-policy
  optimizer logs.
- `exp/mssim/plots/<handle>/` — final plots and aggregated CSVs.
- (Optional) `docs/MSSIM_GRAPH_BENCH.md` — append a row.
