---
name: mssim-graph-picker
description: Pick the next promising mssim call graph from `trace-analysis/graph_branch/` to benchmark, based on heuristics that predict whether our policy will beat Rajomon baselines. Inspects graphs in the directory, ranks them by method count, variant heterogeneity, top-USER-variant probability, and tail latency, and recommends 2-3 candidates with reasoning. Trigger this skill whenever the user asks "which graph should I run next?", "find me a graph that's good for our system", "pick a new call graph", "what's a good workload to test on", or anything phrased as graph/workload selection for mssim. Also trigger proactively when the user says they want to evaluate our policy on a new workload but hasn't named one yet — offer to pick instead of guessing.
user_invocable: true
---

# mssim graph picker

Recommends 2–3 mssim call graphs from `trace-analysis/graph_branch/` that
are likely to be useful benchmarks for our policy. The skill is short and
fast — it's just heuristic ranking, no experiments.

## When this skill is the right tool

- User wants to evaluate our policy on a new workload but doesn't have one
  picked.
- User has already burned through the obvious graphs (S_86516878,
  S_14677443, S_32048416, S_51794709) and wants the next interesting one.
- User asks for graphs with specific structural properties ("heterogeneous",
  "many parallel calls", "deep chain").

If the user already named a graph, use `mssim-graph-bench` directly — don't
re-pick.

## What predicts a "good for our policy" graph

Recorded from prior runs (see `docs/MSSIM_GRAPH_BENCH.md` and
`MEMORY.md` for the full evidence). The structural features that matter:

1. **USER-variant heterogeneity (most important)** — many distinct entry
   variants with low top-variant probability means the workload mixes
   requests with very different downstream costs. Our priority/admission
   policies exploit that; Rajomon's single-knob price can't. The strongest
   win we've recorded is on S_86516878 (150 USER variants, top variant only
   30.5% of traffic).
2. **Tight-but-feasible SLO regime** — the comparison only differentiates
   when queueing is the binding constraint. If intrinsic p99 ≪ SLO, all
   policies degenerate to "admit everything". The S_14677443 case
   illustrates: SLO=100ms ties everyone (~865 goodput); SLO=30ms gives ours
   +400 RPS. That's an SLO-tuning concern, not a graph-selection one, but
   note it when recommending.
3. **Branching cost asymmetry** — if different USER variants have visibly
   different downstream cost (≥3× spread in p99 sums), our estimator can
   pick that up branch-by-branch. Rajomon's price is global.
4. **Tractable scale** — `method_count` between ~50 and ~500. Below ~20 the
   workload is too trivial to differentiate; above ~500 docker startup gets
   slow and the system saturates at very low RPS.
5. **Bounded intrinsic p99** — top variant p99 should be roughly in
   `[10ms, 100ms]`. Above that, the leaf-spin cap (50ms total leaf latency
   in `apps/mssim/generic-service/src/core.rs`) won't save you from
   composing into >200ms per request, and saturation will hit at <50 RPS
   (the S_17454060 case — 220 methods, but variants p99 up to 448ms,
   unbenchmarkable).

What does **not** matter:
- Total trace count / row count in `summary.csv`.
- Number of services (the simulator runs all in one cluster).

## Procedure

### 1. Read the index

```bash
cat trace-analysis/graph_branch/summary.csv
```

Note `method_count` and `variant_count` for each graph. Compute V/M ratio
inline.

### 2. For top candidates, inspect properties

For each graph that passes the scale filter (`50 <= method_count <= 500`),
read:

- `trace-analysis/graph_branch/<graph>/latency_percentiles.json` — extract
  the top-5 variants by p99. Structure: `{method: {variant:
  {percentile_str: value_ms}}}`. Percentile keys are strings (`"50"`,
  `"99"`, `"100"`); values are in milliseconds.
- `trace-analysis/graph_branch/<graph>/interface_distribution.json` — counts
  of `{method: {variant: count}}`. Use this to identify the top-traffic
  variants and compute their share. (USER-facing methods aren't explicitly
  marked; treat the variants of the methods that appear most as roots in
  the trace as a proxy.)

If the user asks for fast turnaround, skip the per-graph inspection and
rank only on `summary.csv` features.

### 3. Score and rank

A simple ranking that has worked:

- **+3** if `method_count` in `[100, 400]` (sweet spot).
- **+2** if `variant_count / method_count >= 2.0` (variant richness).
- **+2** if top-5 variant p99 max is in `[10, 100]ms`.
- **−5** if top variant p99 > 200ms (intrinsic too heavy).
- **−3** if `method_count < 30` (too trivial).
- **−3** if `method_count > 600` (slow to spin up).

These weights are calibrated to past experience, not derived from anything
fundamental — adjust if the user pushes back.

### 4. Exclude graphs we've already characterized

Before reporting, check `docs/MSSIM_GRAPH_BENCH.md` (if it exists) for the
list of already-tested graphs. Don't re-recommend them unless the user
explicitly says "yes including the ones I've done".

Currently characterized:
- `S_86516878` — heterogeneous win
- `S_14677443` — needs tight SLO (30ms) for differentiation
- `S_32048416` — homogeneous, ties
- `S_51794709` — heterogeneous, ours wins big in deep overload

### 5. Report

Present 2–3 candidates as a short table with reasoning:

| Graph | Methods | Variants | V/M | Top-5 p99 | Why this one |
|---|---:|---:|---:|---|---|

For each candidate, give a one-sentence rationale that names the
structural feature being tested (e.g. "high V/M ratio — tests whether ours
generalizes beyond the giant-fanout S_86516878 case"). Mention any
concerns (e.g. "top variant p99 = 86ms — close to the 100ms cliff").

End with: "Want me to run `mssim-graph-bench` on the top pick?" — that's
the natural handoff.

## Things to avoid

- **Don't recommend a graph just because it's big.** Big means slow docker
  startup and high optimizer cost; it doesn't predict policy
  differentiation.
- **Don't tie-break by trace count.** Our experiments don't replay traces;
  they sample variants from the call graph. Trace count is a property of
  the original dataset, not the simulated workload.
- **Don't recommend an already-characterized graph** unless the user wants
  a fresh re-run (e.g. after a code change).
- **Don't pick more than 3.** Decision fatigue defeats the purpose.
