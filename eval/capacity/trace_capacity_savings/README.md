# Trace capacity savings

This matched-goodput experiment measures provisioned capacity for the combined
Alibaba A3 (`S_14677443`) and A4 (`S_32048416`) workload. Each point is an
ordinary runner configuration under
[`exp/tracebench/in/trace_capacity_savings`](../../../exp/tracebench/in/trace_capacity_savings/),
nested by attainment target, policy, and target goodput.

The offered rate is `target_goodput / attainment`, and both entry paths use a
100 ms end-to-end SLO. The merged workload has 20 backend services plus its
frontend. All services remain at one replica except `MS_37691`, the hot
downstream service common to A3 and A4. The counts below are replicas of that
bottleneck, matching the capacity metric in the paper.

| Attainment | Policy | Bottleneck replicas at 1k / 2k / 3k / 4k goodput |
| --- | --- | --- |
| 90% | Rajomon-FIFO | 23 / 38 / 51 / 64 |
| 90% | Rajomon-TailClipper | 23 / 30 / 38 / 45 |
| 90% | Masa | 21 / 25 / 28 / 31 |
| 99% | Rajomon-FIFO | 30 / 50 / 61 / 71 |
| 99% | Rajomon-TailClipper | 24 / 35 / 46 / 54 |
| 99% | Masa | 23 / 27 / 31 / 35 |

Run every point on the evaluation Kubernetes environment:

```bash
./eval/capacity/trace_capacity_savings/run.sh --k8s
```

The runner writes each point independently under the matching nested path in
`exp/tracebench/out/trace_capacity_savings/`. Check that achieved goodput
reaches the path's target at the committed replica count. Kind can validate an
individual point, for example:

```bash
uv run -m exp_runner run tracebench trace_capacity_savings/p90/masa/1k --kind
```

Kind is suitable for functional validation, not comparison with the paper's
capacity numbers. The largest point provisions 71 bottleneck replicas, so the
full group belongs on the evaluation cluster.
