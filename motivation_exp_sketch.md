# S2.1 Experiment: Local SLOs Hide Goodput Opportunities

## Goal

Run a minimal synthetic experiment for Section 2.1 that shows a shared service can lose end-to-end goodput when it only sees a per-service SLO. The experiment should produce the plot currently marked as TODO in `sections/02_background.tex`: offered load on the x-axis and end-to-end goodput on the y-axis, with one line for `Local-SLO/FIFO` and one line for `Oracle margin scheduling`.

The experiment is not meant to evaluate Masa. It is a motivation experiment that isolates one mechanism: at a shared bottleneck, two RPCs can look identical under a local SLO while having different value to the user-facing end-to-end SLO.

## Claim To Validate

When two request classes share a bottleneck method with the same local SLO and same local service demand, a policy that only sees the local SLO cannot distinguish them. If the classes differ in upstream elapsed time or downstream remaining work, an oracle that schedules by true end-to-end margin can complete more user-facing requests within their SLO using the same arrivals, resources, and service times.

## Workload Model

Use a discrete-event simulator rather than the full runtime. This keeps the experiment small and makes the source of the goodput gap defensible.

Model one shared single-server bottleneck method `B` called by two user-facing request classes:

| Parameter | Class A: tight request | Class B: loose request |
| --- | ---: | ---: |
| End-to-end SLO | 100 ms | 100 ms |
| Upstream elapsed before `B` | 45 ms | 10 ms |
| Service time at `B` | 10 ms | 10 ms |
| Downstream work after `B` | 35 ms | 10 ms |
| Local SLO visible at `B` | 50 ms | 50 ms |

Both classes are intentionally identical from the bottleneck's local view: same method, same local SLO, same local service-time distribution. They differ only in hidden end-to-end context.

At arrival to `B`, the initial true margin is:

```text
margin = end_to_end_slo - upstream_elapsed - downstream_work - service_time_at_B
```

With the parameters above, Class A has 10 ms of initial margin and Class B has 70 ms. Queueing delay at `B` consumes this margin. A request contributes to goodput only if:

```text
upstream_elapsed + queue_wait_at_B + service_time_at_B + downstream_work <= end_to_end_slo
```

## Load Generation

Generate open-loop arrivals to the bottleneck.

- Use a 50/50 mix of Class A and Class B.
- Use Poisson arrivals unless there is a reason to use constant-rate arrivals for reproducibility.
- Sweep offered load from below saturation to deep overload, for example 40% to 180% of bottleneck capacity.
- Bottleneck capacity is `1 / service_time_at_B`; with 10 ms service time, capacity is 100 RPC/s.
- Suggested offered-load points: 40, 60, 80, 90, 100, 110, 120, 140, 160, 180 RPC/s.
- Run each load point for a fixed measurement window after warmup, for example 30 s warmup and 120 s measurement.
- Repeat each point with at least 5 random seeds and report mean plus standard deviation.

Use common random numbers: for a given load and seed, both policies must receive the same arrival sequence, class labels, and service times.

## Policies

### Local-SLO/FIFO

This policy represents the per-service SLO abstraction.

- The bottleneck only sees the method-local target.
- Since both classes have the same local SLO, all queued RPCs are equivalent.
- Serve queued RPCs in arrival order.
- Do not perform admission control, shedding, or early return.

### Oracle Margin Scheduling

This policy represents the upper bound on goodput available if the bottleneck had perfect end-to-end context.

- At every scheduling decision, compute each queued request's current true margin:

```text
current_margin =
  end_to_end_slo
  - elapsed_time_since_root_arrival
  - service_time_at_B
  - downstream_work_after_B
```

- Serve the queued request with the smallest current margin first.
- Do not admit, reject, or drop different requests from FIFO.
- Do not change service times.
- Break ties by arrival order.

The oracle is intentionally unrealizable. It should only reorder ready work. This is important because the Section 2.1 argument depends on attributing the goodput gap to local scheduling information hidden by per-service SLOs.

## Metrics

Primary metric:

- End-to-end goodput, measured as requests per second that complete within the 100 ms end-to-end SLO.

Secondary metrics to record for debugging and paper support:

- Offered load in RPC/s.
- Completion throughput in RPC/s, regardless of SLO.
- SLO miss rate.
- Mean and p95 queue wait at `B`, split by class.
- Goodput by class.
- Fraction of scheduling decisions where FIFO and oracle choose different classes.

## Expected Plot

Generate one plot for the paper:

- x-axis: offered load in RPC/s.
- y-axis: end-to-end goodput in requests/s.
- line 1: `Local-SLO/FIFO`.
- line 2: `Oracle margin scheduling`.
- error bars or shaded bands: standard deviation across seeds.

The expected shape is:

- Below saturation, both policies should match because queueing is small.
- Near and above saturation, oracle should sustain higher goodput by serving tight requests before loose requests.
- In deep overload, both policies may flatten or decline, but oracle should remain above FIFO unless the load is so high that even the oracle cannot keep tight requests within SLO.

## Acceptance Criteria

The experiment supports Section 2.1 if all of the following hold:

- Both policies use identical arrivals, resources, admitted requests, and service times for each seed.
- The bottleneck-local view is identical for the two request classes.
- Oracle margin scheduling achieves higher mean goodput than Local-SLO/FIFO for at least one overloaded load point.
- The paper plot makes the gap visually clear without depending on a single noisy seed.
- The implementation exports raw per-seed data so the result can be regenerated.

If the gap is weak, adjust only the hidden end-to-end context parameters, not the local bottleneck parameters. For example, increase Class A upstream elapsed time or downstream work, or increase Class B margin. Keep both classes identical in local SLO and service time at `B`.

## Suggested Artifacts

Place generated outputs under `figs/motivation/`:

- `s2_1_synthetic_goodput.csv`: one row per load, policy, and seed.
- `s2_1_synthetic_goodput_summary.csv`: mean and standard deviation by load and policy.
- `s2_1_synthetic_goodput.pdf`: camera-ready figure for the paper.
- `s2_1_synthetic_goodput.png`: quick preview figure.

After generating the figure, replace the placeholder in `sections/02_background.tex` with the PDF and update the caption only if the implemented parameters differ from this plan.

## Minimal Simulator Logic

For each load, seed, and policy:

1. Generate root arrivals and assign each request Class A or Class B.
2. Convert each root arrival into a bottleneck arrival by adding the class-specific upstream elapsed time.
3. Maintain a ready queue at `B`.
4. When `B` is idle, choose the next queued request using the policy.
5. Run the selected request for its bottleneck service time.
6. Mark it as goodput if root arrival to final downstream completion is at most the end-to-end SLO.
7. Repeat through warmup and measurement; report metrics only for completions whose root arrival occurred in the measurement window.

The simulator should not model Masa, slack estimation, gateway admission, per-service shedding, network jitter, or replica load balancing. Those mechanisms belong in the evaluation section, not this motivation experiment.
