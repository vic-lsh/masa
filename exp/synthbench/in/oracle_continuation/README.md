# Exact-continuation scheduling experiment

This experiment asks whether Masa's scheduling benefit comes from ordering requests by
remaining work, rather than from numerically precise remaining-time estimates.

Request classes `a` and `b` model two execution paths under the same 75 ms user-facing
SLO. Each path visits a single-replica shared CPU service twice, with asynchronous I/O
between visits and branch-specific work afterward:

- `a`: 3 ms shared CPU, 2 ms I/O, 3 ms shared CPU, and 2 ms tail work.
- `b`: 3 ms shared CPU, 8 ms I/O, 3 ms shared CPU, and 10 ms tail work.

Each duration is a small three-point distribution around the stated mean. The alternating
CPU/I/O structure creates repeated, realistic scheduling opportunities without adding a
second bottleneck. The shared service has about 166 requests/s of nominal capacity. We
offer 130, 140, and 145 requests/s with a 32-request in-flight cap, keeping the shared
service busy while avoiding the clearly collapsed region observed above 145 requests/s.

The 75 ms SLO is about 1.5x the longer path's pre-saturation p95, following the paper's
SLO-setting methodology. Giving both paths the same SLO is intentional: it models hidden
path variation within one user-facing request class, for which deadline-only scheduling
cannot distinguish work remaining.

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

After exploratory SLO/load pilots, the settings above were frozen. Five fresh repetitions
produced the following aggregate goodput (mean and 95% t interval):

| Offered RPS | Deadline only | Learned continuation | Exact service continuation |
| ---: | ---: | ---: | ---: |
| 130 | 116.9 ± 5.1 | 117.8 ± 1.3 | 113.5 ± 5.0 |
| 140 | 110.2 ± 3.9 | 106.9 ± 5.9 | 105.1 ± 16.4 |
| 145 | 103.1 ± 8.9 | 99.1 ± 11.6 | 105.3 ± 14.7 |

The learned-minus-deadline paired differences were 0.9 ± 5.3, -3.3 ± 6.0, and
-4.0 ± 15.0 requests/s. The learned-minus-exact differences were 4.3 ± 6.0,
1.8 ± 19.1, and -6.1 ± 22.1 requests/s. Every interval includes zero. Per-class
goodput also shows no consistent starvation-hidden aggregate gain. The shared service
averaged 81-82% CPU across policies, confirming that the experiment exercised contention.

This tuned scheduling-only experiment therefore does **not** validate the hypothesis that
continuation ordering causes Masa's gains. The positive 140 requests/s exploratory pilot
did not survive repetition. Above this region, all policies become highly variable because
there is no admission control or early return; that behavior is consistent with the
paper's overload-control ablation, but it cannot be counted as a scheduling result.

The exact arm is a service-demand oracle, not a causal wall-clock oracle: Masa's learned
target includes queueing, transport, and runtime overhead after a child returns, all of
which depend partly on the schedule being evaluated. Since priority is
`deadline - estimate`, numeric scale can change comparisons between differently aged
requests even when path-level estimator ranks agree. A stronger follow-up would record
each runnable request's realized continuation under a reference execution and measure
pairwise priority-order agreement at actual scheduling decisions. Until then, the paper
should avoid claiming that rank alone is sufficient.
