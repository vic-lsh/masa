# Matched-deadline continuation-ordering experiment

This controlled experiment asks whether Masa changes the ordering of requests that have
the same end-to-end deadline but very different work remaining, and whether that change
improves goodput.

Requests arrive in bursts of eight. Every request in a burst has the same gateway-entry
timestamp and 50 ms deadline, while the API is sampled independently with a 50/50 mix.
Over 99% of measured bursts contain both request classes. The paths are:

- `a`: 3 ms of CPU at the single-replica `shared` service, then a 2 ms nonblocking wait.
- `b`: the same 3 ms at `shared`, then a 25 ms nonblocking wait.

Durations use small three-point distributions around those means. This models requests
from one user-facing class taking different internal branches. Running the long
continuation first lets its wait overlap more of the shared service's CPU work. The burst
is an intentional mechanism probe: synchronized arrivals occur with fan-out, batching,
and periodic releases, but this is not presented as a representative macrobenchmark.

The configured 100, 220, and 280 RPS values remain total request rates; bursts arrive at
one eighth those rates. A 32-request in-flight cap prevents an unstable client backlog,
but did not drop requests in the measured runs. No policy enables admission control,
early return, or load shedding.

The three policies are:

- `sched_slo`: deadline-only scheduling. Same-deadline requests are effectively unordered.
- `sched_pred,est_mean_var`: Masa's learned continuation estimate.
- `eval_oracle_continuation`: pre-sampled continuation service demand injected into the
  same `sched_pred` priority and reprioritization path.

### Ordering-corruption dose response

The exact-continuation arm supports an evaluation-only corruption probability in
`policy_param.json`:

```json
{
  "pred": {
    "eval_oracle_order_corruption_probability": 0.25
  }
}
```

Use separate experiment directories for probabilities 0, 0.10, 0.25, and 0.50. The
decision is deterministic from the gateway-entry timestamp, so every request in one
matched-deadline burst receives the same treatment. Selected bursts complement the
exact scheduling estimate around the common SLO, reversing the short/long priority
order. Only the soft scheduling estimate changes; the exact mean/floor used to propagate
hard deadlines remains unchanged. Consequently the intervention does not change request
work, admission, abortion, or the marginal arrival process. Report the realized
long-before-short ordering rate in addition to the configured probability.

Run the experiment and reproduce the pairwise ordering measurement with:

```bash
uv run -m exp_runner run synthbench oracle_continuation --plot
uv run python exp/synthbench/in/oracle_continuation/measure_ordering.py
```

For each mixed burst, the ordering metric compares every `b`/`a` pair at the shared hop.
A pair agrees with the intended ordering when the long-continuation `b` request starts or
finishes that hop before `a`. This definition is workload-grounded and does not call an
estimator numerically "accurate."

## Results (2026-08-25)

After exploratory workload and SLO pilots, the configuration above was frozen and run
for five fresh repetitions. Values below are means and 95% t intervals across runs.

### End-to-end goodput

| Offered RPS | Deadline only | Learned continuation | Exact service continuation |
| ---: | ---: | ---: | ---: |
| 100 | 72.29 ± 1.60 | 73.28 ± 0.65 | 71.58 ± 1.32 |
| 220 | 164.52 ± 2.94 | 168.56 ± 1.48 | 165.24 ± 2.93 |
| 280 | 217.55 ± 2.34 | 220.11 ± 1.92 | 217.31 ± 3.70 |

The learned-minus-deadline paired differences are 0.99 ± 1.72, **4.04 ± 3.09**,
and 2.57 ± 3.61 requests/s. Only the 220 RPS interval excludes zero. The exact arm's
paired differences are -0.71 ± 1.94, 0.72 ± 4.13, and -0.23 ± 5.46 requests/s; none
exclude zero.

### Direct ordering effect

The percentage of long/short pairs for which `b` finishes the shared hop first is:

| Offered RPS | Deadline only | Learned continuation | Exact service continuation |
| ---: | ---: | ---: | ---: |
| 100 | 50.82 ± 2.73% | 59.05 ± 0.76% | 58.09 ± 1.58% |
| 220 | 50.09 ± 1.32% | 59.55 ± 1.03% | 58.72 ± 1.33% |
| 280 | 50.29 ± 0.30% | 59.18 ± 1.36% | 56.98 ± 1.13% |

The learned policy improves this pairwise rate over deadline-only scheduling by
8.23 ± 3.16, 9.47 ± 1.52, and 8.90 ± 1.56 percentage points. The corresponding start-order
rates are about 50% for deadline-only, 66-68% for learned continuation, and 63-66% for
the exact arm. Thus the scheduling mechanism changes ordering reliably even though not
every request is simultaneously runnable.

### Where the latency distribution moves

At 220 RPS, learned continuation shifts shared-hop queueing in the expected directions:

- `a` (short continuation): p50 queueing increases by 0.22 ± 0.10 ms.
- `b` (long continuation): p50 queueing decreases by 0.42 ± 0.09 ms and p90 decreases
  by 0.83 ± 0.08 ms.
- End-to-end p50 moves up 0.27 ± 0.20 ms for `a` and down 0.23 ± 0.16 ms for `b`.
- All `a` requests still meet the SLO, while `b` SLO attainment rises by
  3.09 ± 1.97 percentage points.

The shared-hop queueing redistribution repeats at all three loads: learned scheduling
raises `a`'s median by 0.18-0.24 ms and lowers `b`'s median by 0.35-0.42 ms. The end-to-end
p90/p99 changes are generally below 0.3 ms and usually indistinguishable from zero,
because the 20-30 ms downstream wait dominates those quantiles and runnable tasks can
still interleave after their first poll. Since the 50 ms SLO cuts through the middle of
`b`'s distribution, the smaller median shift can change deadline attainment without a
visually large tail-latency shift.

## Interpretation

This experiment validates the narrow mechanism claim: continuation estimates cause Masa
to favor the request whose longer remaining wait should start first, and the class-level
queueing distributions move accordingly. It gives limited evidence for an end-to-end
benefit—one load has a repeatable learned-estimator goodput gain—but it does not establish
a broad or large scheduling-only goodput improvement.

It also does not yet prove that relative rank alone is sufficient. The learned and exact
arms induce similar pairwise ordering changes, but the exact arm has no repeatable
goodput gain. The exact value is service demand from the pre-sampled execution plan, not
realized wall-clock continuation including transport, runtime overhead, and
schedule-dependent delay. Numeric scale can also affect which differently aged runnable
tasks overlap. A cleaner rank-versus-magnitude follow-up would apply monotone rescalings
to the *same* learned estimate under matched deadlines and verify that pairwise decisions
are preserved before comparing outcomes.
