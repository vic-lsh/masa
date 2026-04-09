# Predictive Admission Control Tuning Plan

## Goal

Achieve stable goodput at 2000 RPS using the early-return-rate driven admission controller (`ac_pred`). Target: a flat goodput line near system capacity (~1800-1900 RPS), without the death-spiral caused by self-reinforcing rejection.

## Constraint

- **At most one new parameter** beyond the existing 4: `er_alpha`, `tau_fast`, `tau_slow`, `max_reject_floor`.
- 25 iterations max.
- Each iteration: hypothesis → expectation → result analysis (in MASA_IMPROVEMENTS.md style).

## Baseline Bug Fix (Iteration 0)

### Self-rejection feedback loop
Before tuning, the structural feedback loop must be cut. When `PredAdmissionRej` fires, the rejected request hit `finalize()` with `Err(DeadlineExceeded)`. The estimation layer marked it as a local early return, inflating `early_return_count`, which then fed back into `record_outcome` — causing rejection to inflate er_rate, which caused more rejection.

**Fix**: Added `self_rejected: AtomicBool` to `PredAdmissionLayer`. Set in `before_child_rpc` when Layer 2 rejects. Checked in `finalize()` to skip `record_outcome`.

**File**: `libs/masa-policy/src/layer/admission/predictive.rs`

## Iteration Phases

### Phase 1 — Saturation discovery (iter 1-3)
Use `sched_fifo` (no AC) to find the natural saturation point.

**Iter 1** — Broad sweep `[800, 1000, 1200, 1400, 1600, 1800, 2000, 2200]`, Search API only.
- **Hypothesis**: Saturation at ~1800-1900 (per user).
- **Expectation**: Goodput tracks RPS up to ~1800, then flattens or drops.
- **Analysis**: Identify the knee point and the collapse point.

**Iter 2** — If iter 1 saturation differs from expectation, narrow the sweep around the knee.

**Iter 3** — Confirm CPU bottleneck (read `cpu_summary.csv`) — saturation should be CPU-bound.

### Phase 2 — Cold-start sanity (iter 4-6)
Run with `sched_pred,ac_pred,abort_slack,est_mean_var` at default params on the same RPS sweep.

**Iter 4** — Default params, sweep through saturation.
- **Hypothesis**: With self-rejection bug fixed, AC reaches steady state without spiraling. At sub-saturation, `er_rate ≈ 0`. At super-saturation, `er_rate > 0` and rejection capped at `max_reject_floor=0.10`.
- **Expectation**: Goodput ≥ baseline (fifo) below saturation. Above saturation, goodput stays flat at ~capacity instead of collapsing.
- **Failure modes to watch**:
  - Goodput collapses anyway → feedback loop still present
  - Goodput drops below fifo at sub-saturation → AC over-rejecting
  - er_rate stays at 0 even above saturation → signal not reaching admission

**Iter 5** — If iter 4 succeeds, run at fixed 2000 RPS for 120s to test sustained behavior.

**Iter 6** — Burst test: 1500 RPS for 30s, 2200 RPS for 30s, 1500 RPS for 30s. Verify recovery.

### Phase 3 — Tune `er_alpha` (iter 7-10)
`er_alpha` controls how fast `er_rate` reacts to early returns. Default: 0.05.

**Iter 7** — `er_alpha = 0.01` (slow). Hypothesis: smoother but slower response to overload onset.
**Iter 8** — `er_alpha = 0.02`.
**Iter 9** — `er_alpha = 0.10` (fast). Hypothesis: quicker reaction but more oscillation.
**Iter 10** — `er_alpha = 0.20` (very fast).

For each: measure goodput stability at 2000 RPS, time to reach steady state, oscillation amplitude.

### Phase 4 — Tune `max_reject_floor` (iter 11-14)
This is the cap on rejection probability when goodput is healthy. Default: 0.10.

**Iter 11** — `max_reject_floor = 0.05` (tight).
**Iter 12** — `max_reject_floor = 0.20`.
**Iter 13** — `max_reject_floor = 0.30`.
**Iter 14** — `max_reject_floor = 0.50` (very loose).

Hypothesis: tighter floor → more goodput at the edge but worse SLO compliance under sustained overload. Looser floor → better SLO compliance but lower goodput.

### Phase 5 — Tune `tau_fast` / `tau_slow` (iter 15-18)
These control how quickly the goodput floor lifts under genuine drops. Defaults: 0.5s / 5.0s.

**Iter 15** — `tau_fast=0.2, tau_slow=2.0` (more reactive).
**Iter 16** — `tau_fast=1.0, tau_slow=10.0` (more stable).
**Iter 17** — `tau_fast=0.5, tau_slow=20.0` (slow long-term reference).
**Iter 18** — Best combo from above × best er_alpha from Phase 3.

### Phase 6 — One new parameter (iter 19-22)
Based on Phase 1-5 results, identify the biggest remaining problem and add ONE parameter.

Candidate parameters (decide based on results):
- **`er_rate_target`**: instead of `reject_prob = er_rate`, use `reject_prob = max(0, er_rate - target)`. Tolerates a baseline rate of acceptable early returns without rejecting.
- **`min_admit_rate`**: hard floor on admission probability (e.g., 0.2) to prevent the controller from reaching ~0% admission even under severe overload.
- **`er_floor_threshold`**: only activate the goodput floor lift when `er_rate > threshold`, preventing premature floor-lifting.

**Iter 19** — Implement chosen parameter.
**Iter 20-22** — Sweep its value.

### Phase 7 — Final validation (iter 23-25)
**Iter 23** — Run final config side-by-side: `fifo`, `sched_slo,abort_slo`, `sched_pred,abort_slo,ac_pred,est_mean_var`. Compare goodput curves.
**Iter 24** — Stress test: 3000 RPS sustained for 120s. Verify graceful degradation.
**Iter 25** — Capture final tuned params in `policy_param.json` and document in this file.

## Result Format (per iteration)

Each iteration appends an entry below in this format:

```markdown
### Iteration N — <short title>
**Date**: YYYY-MM-DD
**Params changed**: `<param>=<value>`
**Hypothesis**: <what we expect to happen and why>
**Expectation**: <quantitative prediction — e.g., "goodput at 2000 RPS ≥ 1800/s, er_rate ≈ 0.1, no oscillation">
**Setup**: `exp/hotel/in/<config>`, RPS sweep `[...]`, duration N seconds
**Result**: <link to plot, key numbers from logs>
**Analysis**: <what actually happened, root cause if surprising>
**Conclusion**: <keep change / revert / next step>
```

## Iterations Log

### Iteration 0 — Self-rejection bug fix
**Date**: 2026-04-08
**Params changed**: none — code change only
**Hypothesis**: Counting our own admission rejections as early returns creates a self-reinforcing loop where rejection inflates `er_rate` and triggers more rejection.
**Expectation**: After fix, `er_rate` reflects only genuine downstream early returns, not the controller's own actions.
**Setup**: Code change to `libs/masa-policy/src/layer/admission/predictive.rs` — added `self_rejected: AtomicBool`, set on Layer 2 rejection in `before_child_rpc`, checked in `finalize` to skip `record_outcome`.
**Result**: Unit tests pass (`cargo test -p masa-policy --features sched_slo,ac_pred,est_mean_var -- predictive`).
**Analysis**: The fix is structurally sound — only requests admitted by AC contribute outcomes to `record_outcome`. Whether it actually breaks the spiral in practice depends on Phase 2 experiments.
**Conclusion**: Keep. Proceed to Phase 1.

### Iteration 1 — Saturation discovery (in progress)
**Date**: 2026-04-08
**Params changed**: none — using `sched_fifo`
**Hypothesis**: Search API saturates at ~1800-1900 RPS per user prior knowledge.
**Expectation**: Goodput tracks RPS up to ~1800, then flattens.
**Setup**: `exp/hotel/in/ac_tune_search` — Search only, RPS `[800, 1000, 1200, 1400, 1600, 1800, 2000, 2200]`, fifo only, 40s per step.
**Result**: TBD
**Analysis**: TBD
**Conclusion**: TBD
