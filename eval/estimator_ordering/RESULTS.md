# Estimator and ordering pilot results

Date: 2026-08-26. These are one-repetition mechanism pilots, not paper-ready
confidence-interval results. All 11 experiment roots completed successfully. The raw
outputs are under `exp/synthbench/out/estimator_ordering_*`; the generated aggregate
tables are in `results/`.

## Setup and reference

Requests arrive at 220 RPS in matched-deadline bursts of eight with a 50 ms SLO. Every
request first executes about 3 ms of CPU at a single-replica `shared` service, followed by
either about 2 ms (short) or about 25 ms (long) of nonblocking continuation work. The
aggregate mix is 50/50 in every observability arm:

| Arm | What the scheduling key reveals before `shared` |
| --- | --- |
| Hidden | One API key contains both short and long continuations. |
| Partial | One quarter of all requests are a known-long API; the remaining API still mixes short and long. |
| Visible | Separate short and long API keys. |

The primary label is the continuation service demand sampled before the request runs.
It is schedule-independent, unlike realized wall-clock continuation. Comparisons are
restricted to requests sharing a matched deadline. The audit joined all 3,304 measured
requests at each observability point to a pre-decision estimate and reference label.

The code audit found an important claim boundary: the soft scheduling score currently
uses `(root API, parent RPC, child RPC)`. Path/fanout-conditioned estimates feed hard
deadline tightening, not the `full` estimate used as scheduling priority. The results
below therefore describe the actual scheduling key, not the paper's richer conceptual
key.

## E1-E3: Numeric error, ordering, and within-key ambiguity

`NAE` is absolute error divided by the 50 ms SLO. Pair agreement excludes learned ties;
the tie rate is reported separately. Top-choice agreement gives fractional credit when
the learned score ties multiple candidates. Same-key ambiguity is the fraction of
same-key pairs whose true continuations differ by more than 5% of the SLO.

| Visibility | Keys | NAE p50/p90 | Learned ties | Pair agreement | Top choice | Cross-key flips | Same-key ambiguity |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Hidden | 1 | 19.0% / 41.7% | 100.0% | n/a | 17.4% | n/a | 64.1% |
| Partial | 2 | 6.9% / 40.7% | 60.2% | 86.1% | 33.9% | 13.9% | 51.7% |
| Visible | 2 | 2.7% / 10.1% | 40.8% | 100.0% | 38.4% | 0.0% | 27.6% |

This is direct evidence for both sides of the reviewer question. The key gives a useful
cross-class order when the branch is observable: the fully visible arm has no cross-key
flips and perfect agreement among non-tied pairs. It does not resolve per-request
variation inside a key: even the visible arm has 27.6% material same-key ambiguity and a
low top-choice rate. Hidden branch variation collapses the estimator to a tie.

## E2-E4: Does the estimated order reach the shared service?

The table uses the non-audit policy runs for system outcomes. `Start agreement` is the
fraction of reference-distinct pairs whose longer continuation starts the shared hop
first. The exact arm substitutes pre-sampled continuation into the same scheduler.

| Visibility | Policy | Start agreement | Finish agreement | Goodput RPS |
| --- | --- | ---: | ---: | ---: |
| Hidden | Deadline only | 49.8% | 50.4% | 149.17 |
| Hidden | Learned | 49.1% | 49.9% | 146.13 |
| Hidden | Exact reference | 65.7% | 57.3% | 152.56 |
| Partial | Deadline only | 49.5% | 51.2% | 151.31 |
| Partial | Learned | 54.6% | 51.4% | 150.27 |
| Partial | Exact reference | 65.2% | 59.6% | 155.46 |
| Visible | Deadline only | 49.4% | 49.2% | 146.26 |
| Visible | Learned | 60.6% | 56.1% | 149.57 |
| Visible | Exact reference | 63.7% | 58.2% | 151.00 |

As visibility increases, learned scheduling moves from chance to 60.6% start agreement;
the exact arm shows that per-request information can move it further. Goodput does not
vary monotonically across the independent one-run visibility pilots, so this table is
evidence for the ordering mechanism, not a precise causal goodput estimate.

## E5: Rank versus numeric magnitude

### Same learned estimator, different magnitudes

The multiplier changes only the soft-priority continuation. Hard deadlines, request
work, admission, and abortion are unchanged. `Policy NAE` measures the scaled value
actually consumed by scheduling. All positive scales preserve the intended order under
the matched deadlines.

| Scale | Policy NAE p50/p90 | Pair agreement | Cross-key flips | Goodput RPS |
| ---: | ---: | ---: | ---: | ---: |
| 0.5x | 2.0% / 31.7% | 100.0% | 0.0% | 148.99 |
| 1x | 2.7% / 10.1% | 100.0% | 0.0% | 150.22 |
| 2x | 40.0% / 61.1% | 100.0% | 0.0% | 149.00 |
| 4x | 23.5% / 167.9% | 99.97% | 0.0% | 149.36 |

Despite a large numeric-error range, goodput stays between 148.99 and 150.22 RPS (under
0.9% peak-to-peak) while pair agreement stays essentially fixed. This is the cleanest
pilot evidence for the narrow claim that scheduling is less sensitive to magnitude than
to order. Repetitions are required before claiming equivalence.

### Natural estimator variants

The raw estimator and the policy-effective score must be distinguished. RMS and
histogram update in batches; between updates, time decay drove their effective priority
toward a tie even though their raw point error was small.

| Estimator | Raw NAE p50/p90 | Policy NAE p50/p90 | Learned ties | Start agreement | Goodput RPS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mean/variance | 2.7% / 10.1% | 2.7% / 10.1% | 40.8% | 58.3% | 150.22 |
| RMS | 4.0% / 15.1% | 39.0% / 51.0% | 100.0% | 50.1% | 148.49 |
| Histogram | 4.0% / 7.2% | 39.0% / 59.0% | 100.0% | 49.5% | 145.42 |

In particular, the histogram has a better raw p90 point error than mean/variance but
loses all scheduling discrimination after policy transformations. This supports
measuring the effective order handed to the runtime rather than only an offline error
statistic.

## E6: Ordering-corruption dose response

The intervention reverses exact short/long soft priorities for a sampled fraction of
matched-deadline bursts. The achieved selected-burst rates confirm that the knob fired.

| Configured corruption | Selected bursts | Start agreement | Finish agreement | Long queue p50 | Short queue p50 | Goodput RPS |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0% | 0.0% | 65.0% | 58.4% | 0.632 ms | 1.257 ms | 149.14 |
| 10% | 9.2% | 63.1% | 58.3% | 0.663 ms | 1.191 ms | 152.63 |
| 25% | 23.5% | 58.4% | 59.0% | 0.671 ms | 1.030 ms | 151.59 |
| 50% | 46.7% | 51.4% | 58.1% | 0.803 ms | 0.858 ms | 150.46 |

Corruption monotonically reduces start-order agreement and shifts queueing away from
short requests and onto long requests. It does not produce a monotonic finish-order or
goodput response in this pilot. Thus it causally validates queueing redistribution, but
does not yet validate a goodput-sensitivity claim. This is consistent with blocking
continuations interleaving after their first poll and with work conservation limiting
aggregate-capacity gains.

## E7: Shadow feasibility calibration

The shadow check asks whether the shared hop finishes after the learned tightened child
deadline; it records the signal but does not abort or feed admission control. The final
end-to-end SLO result supplies the label. The reference check replaces the learned
continuation with pre-sampled continuation work.

| Offered RPS | Learned precision | Learned recall | Learned FPR | Reference precision | Reference recall | Reference FPR |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 88.9% | 11.8% | 0.83% | 100.0% | 51.0% | 0.0% |
| 220 | 92.3% | 6.9% | 0.27% | 100.0% | 44.1% | 0.0% |
| 280 | 93.2% | 5.1% | 0.18% | 100.0% | 41.3% | 0.0% |

The current learned hard-deadline estimate is conservative: its signals are usually
correct but miss most eventual deadline misses. The reference still misses roughly half
because service demand alone omits later queueing and runtime delay. This result argues
against extending the scheduling claim that “only order matters” to deadline tightening,
abortion, or admission control; those paths require separate numeric calibration.

## What is supported now

- The actual scheduling key provides a strong short-versus-long order when the branch is
  observable, and direct data quantifies its ties, flips, and residual within-key
  ambiguity.
- Positive rescalings can make numeric error dramatically worse without changing the
  measured order or materially moving goodput in this matched-deadline pilot.
- Deliberately corrupting order predictably moves the class queueing distributions.

What is not yet supported is a broad claim that ordering accuracy alone creates a large
goodput gain. Every new point has one repetition, corruption did not move goodput
monotonically, and ready-set reconstruction still uses matched bursts plus observed
shared-hop start/finish rather than scheduler-internal candidate-set telemetry. The next
paper-facing step is to freeze these configurations, run at least five paired repetitions,
and lead with the direct accuracy/ambiguity and scale-invariance tables rather than a
large scheduling-only goodput claim.
