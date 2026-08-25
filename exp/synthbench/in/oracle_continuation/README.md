# Exact-continuation scheduling experiment

This experiment measures how much of the goodput benefit from exact continuation-aware
scheduling Masa's online estimator captures.

APIs `a` and `b` have the same 40 ms end-to-end SLO and contend at the single-replica
`shared::run` method. After that method returns, `a` has 2 ms of continuation while `b`
has 14 ms. Tail services are provisioned to keep `shared` as the only intended bottleneck.

The three policies isolate the scheduling signal:

- `sched_slo`: deadline-only scheduling.
- `sched_pred,est_mean_var`: Masa's learned continuation estimate.
- `eval_oracle_continuation`: exact continuation service demand from the pre-sampled
  synthbench execution plan, injected into the same `sched_pred` priority and
  reprioritization path.

No policy enables load shedding or admission control, so estimates affect only dispatch
order. This avoids comparing Masa against the separate absolute-priority `sched_oracle`
scheduler, whose runtime behavior differs. The exact variant still runs the learned
estimator's bookkeeping so the intervention changes only the value used for priority.

Run with:

```bash
uv run -m exp_runner run synthbench oracle_continuation --plot
```

## Results (2026-08-25)

Five repetitions produced the following aggregate goodput (mean and 95% t interval):

| Offered RPS | Deadline only | Learned continuation | Exact service continuation |
| ---: | ---: | ---: | ---: |
| 420 | 367.4 ± 20.5 | 368.4 ± 14.6 | 363.6 ± 14.4 |
| 440 | 338.9 ± 37.1 | 339.1 ± 25.0 | 338.4 ± 24.4 |
| 460 | 267.3 ± 20.5 | 273.8 ± 51.2 | 271.6 ± 49.8 |

The learned-minus-exact paired differences were 4.8 ± 21.9, 0.8 ± 39.5, and
2.2 ± 90.9 RPS. Thus, perfect pre-sampled service demand did not improve over Masa's
learned estimate in this workload. However, learned scheduling also did not improve
significantly over deadline-only scheduling, so this result does **not** establish that
ordering quality causes Masa's gains.

This is also a service-demand oracle, not a causal wall-clock oracle: Masa's learned
target includes queueing and transport after a child returns, which cannot be known in
advance. Since priority is `deadline - estimate`, numeric scale can change comparisons
between differently aged requests even if API-level estimator ordering is unchanged.
The strong claim that only rank matters should therefore be narrowed or tested with
request-level priority-order agreement against realized continuations.
