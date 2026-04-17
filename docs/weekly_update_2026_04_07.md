# Weekly Update — 2026-04-07

## What we shipped

- **Predictive AC v2 is tuned and stable** — 3 experiment tracks (anchor/opal/volt), ~150 iterations total. Final params: `probe_min=0.15`, `rejection_alpha=0.05`, `tau=2.0`. Best result: +118 goodput at 1400 RPS with halved CoV vs baseline.
- **Major refactor of masa-policy** — estimation and admission are now separate composable layers with type-safe keys. Much easier to reason about and extend.
- **New `abort_slack` feature** — predictive abort based on estimated remaining compute, not just deadline.
- **Rajomon experiments** added to hotel app for cross-comparison.

## What we learned

- Early feasibility checks in `before_poll` cause feedback loops — removing them gave +67 to +232 goodput.
- Vegas-style concurrency limiting and ER-gated admission both failed to match the goodput-tracking token bucket.
- The "slow climb" at sub-saturation is a fundamental limitation of the current approach — 14 opal iterations confirmed it's irreducible with pure goodput tracking.

## What needs attention next

1. **`probe_max` is dead code** — explore mode falls back to a fixed `initial_budget_rate` instead of `goodput_rate * (1 + probe_max)`. This is likely the root cause of the slow-climb problem that the opal track couldn't solve. One-line fix, but needs a volt-style experiment to validate.
2. **No hysteresis on explore/exploit switching** — the controller oscillates near the rejection threshold. Should add a second threshold param.
3. **Socialnet evaluation** — most tuning was on hotel. Need to confirm the parameters generalize.

## Decision needed

Should we prioritize fixing the `probe_max` bug and re-running experiments? It could resolve the sub-saturation slow-climb issue that we spent ~40 opal iterations trying to fix through other means — the real problem may have been that explore mode was never actually tracking goodput.
