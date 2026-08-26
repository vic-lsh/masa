# Estimator and ordering validation

The completed one-repetition pilot and its limitations are summarized in
[`RESULTS.md`](RESULTS.md). Regenerate the machine-readable tables with:

```bash
uv run python eval/estimator_ordering/analyze.py exp/synthbench/out
```

This evaluation closes the causal gap between Masa's path-conditioned continuation
estimator and its end-to-end goodput results. It measures the estimate returned before
each child RPC, the per-request continuation that is revealed later, the priority order
implied by both values, and the order realized at the contended service.

The primary reference is the **oracle-policy order**, not a claim of globally optimal
scheduling. For a request `i` with the propagated parent deadline `D_i`, learned
continuation `Rhat_i`, and pre-sampled reference continuation `R*_i`, the learned and
reference child subdeadlines are:

```text
Phat_i = D_i - Rhat_i
P*_i   = D_i - R*_i
```

Lower subdeadlines run first. Comparisons are made only between requests that contend in
the same burst or are simultaneously eligible at the same service. Predicted ties and
reference ties are reported separately.

The implementation currently uses `(root API, parent RPC, child RPC)` for the `full`
estimate that supplies the soft scheduling priority. Path/fanout lookup supplies the
`mean` and `floor` used for hard deadline tightening. The evaluation therefore reports
priority-key quality and path/fanout deadline-key quality separately; it must not label
the latter as the scheduling key. Tokio compares a static relative priority hint rather
than the absolute `D-R` value. Direct telemetry records that exact transmitted hint;
`D-R` is used only as the conceptual order and is equivalent for co-issued requests with
a common parent deadline.

## Common protocol

- Use Synthbench plans sampled before execution so `R*` is independent of the evaluated
  scheduling order.
- Log the learned estimate before the current outcome updates the estimator.
- Use paired random seeds and identical offered request plans across policy arms.
- Run 10 seconds of estimator warmup followed by at least 15 seconds of measurement.
- Use one pilot repetition for functional validation and five repetitions for reported
  confidence intervals.
- Report means and 95% t intervals across repetitions. Individual request pairs are not
  treated as independent repetitions.
- Keep admission control, early return, and load shedding disabled except in the shadow
  feasibility experiment.

## E1: Online estimator and key-quality audit

At each child-group issue, record the request ID, root API, parent and child methods,
observable path/group signature, effective fallback level, sample count, raw and decayed
learned estimates, deadline, and pre-sampled continuation. At parent completion, join the
query to the eventual realized continuation as a secondary, schedule-dependent label.

Replay the chronological observations through method-only, API+edge, and Masa's effective
keying schemes without looking ahead. Report normalized absolute error, residual
within-key variance, p5-p95 spread, full-key/fallback coverage, and sample counts.

## E2: Decision-level ordering audit

Reconstruct the candidate set at the shared service from child-send and first-start
timestamps. For every dispatch opportunity with at least two eligible requests, report:

- pairwise oracle-policy agreement;
- Kendall-style concordance with ties separated;
- top-choice agreement;
- learned tie and oracle tie rates; and
- whether first-start and first-finish order agree with the learned and reference orders.

Each dispatch receives equal aggregate weight so a single large queue does not dominate.

## E3: Within-key ambiguity and cross-key flips

For requests using the same effective key, report the fraction whose reference
continuations differ by more than 1%, 5%, 10%, and 25% of the SLO. For requests using
different keys, report the fraction for which the learned priority order is the reverse
of the oracle-policy order. Break results down by predicted-gap decile and lookup fallback
level.

## E4: Controlled observability sweep

Hold the work distribution, SLO, and bottleneck fixed while changing whether the
short/long suffix is runtime-visible before the shared child is issued. Evaluate 0%, 50%,
and 100% revealed-path mixes with deadline-only, learned, and exact-continuation arms.

Report key coverage, within-key ambiguity, oracle-policy agreement, shared-hop queueing,
per-class SLO attainment, and goodput. The gap between learned and exact scheduling
quantifies information that remains hidden at the decision point.

## E5: Does outcome track rank or point error?

Evaluate natural estimator variants on the same request plans: method-only, API+edge,
full/effective Masa, a short-history estimator, a stale estimator after a service-time
shift, and exact service continuation. Plot normalized goodput against both normalized
point error and decision-level oracle-policy agreement. Report rank and Pearson
correlations descriptively; causal conclusions rely on E6.

For the same-estimator magnitude ablation, build with `eval_estimator_audit` and set:

```json
{
  "pred": {
    "eval_learned_priority_scale": 2.0
  }
}
```

Sweep scales such as 0.5, 1.0, 2.0, and 4.0 on matched-deadline requests. The multiplier
is applied after estimate decay and only to the continuation consumed by the soft
scheduler. The unscaled learned full/floor values continue to determine hard deadlines,
admission, and abort behavior. Record the emitted `learned_priority_scale` and
`effective_priority_us` fields, and verify actual pairwise priority agreement rather than
assuming that a rescaling preserves order across requests with different deadlines.

## E6: Ordering-corruption dose response

Perturb the exact-reference priority in 0%, 10%, 25%, and 50% of matched-deadline bursts.
For selected bursts, complement the binary short/long continuation score around the
common SLO, reversing its soft priority order. Keep the exact hard-deadline values,
admission, abortion, request work, and marginal arrival process unchanged. This is a
causal test of whether degrading a known-good order changes outcomes; it is deliberately
not presented as a deployable estimator.

Report configured corruption and achieved start/finish order because an altered burst
may not create actual contention. Also report class queueing distributions, SLO
attainment, and goodput. A monotonic response connects measured ordering quality to
system outcomes. Fully reversed ordering is an optional endpoint, not the primary
baseline.

## E7: Shadow feasibility calibration

Scheduling consumes relative order, but RPC deadlines and overload control consume
numeric estimates. In shadow mode, record every point at which Masa would declare a
request infeasible, but neither abort the request nor feed the signal to gateway
admission. Let the request finish and report prediction precision, miss recall,
false-positive rate, and lead time to the SLO. Run at low load, the goodput knee, and
overload.

## Reviewer-facing output

The minimum result table is:

| Workload/load | NAE p50/p90 | Pairwise agreement | Top-choice agreement | Learned ties | Cross-key flips | Same-key material ambiguity | Goodput |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |

The claims are scoped as follows:

- E1-E3 measure whether the estimator and effective key produce a useful order.
- E4 measures what observed RPC history can and cannot reveal.
- E5-E6 test whether performance follows ordering more closely than point calibration.
- E7 separately validates the magnitude-sensitive feasibility path; the paper must not
  generalize the scheduling claim that "only order matters" to deadline or admission
  decisions.
