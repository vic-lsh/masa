# Hotel capacity savings

This matched-goodput experiment measures how much provisioned capacity each
policy needs for the Hotel Search and Reservation mix. Each point is an
ordinary runner configuration under
[`exp/hotel/in/hotel_capacity_savings`](../../../exp/hotel/in/hotel_capacity_savings/),
nested by attainment target, policy, and target goodput.

The offered rate is `target_goodput / attainment`. Search and Reservation are
equally weighted and retain their 200 ms and 100 ms SLOs. The eight backend
services remain at one replica each; the committed configurations vary the
shared frontend bottleneck. The counts below are replicas of that bottleneck,
matching the capacity metric in the paper.

| Attainment | Policy | Bottleneck replicas at 3k / 5k / 7k / 9k goodput |
| --- | --- | --- |
| 90% | Rajomon-FIFO | 12 / 15 / 40 / 61 |
| 90% | Rajomon-TailClipper | 12 / 14 / 30 / 48 |
| 90% | Masa | 12 / 12 / 18 / 30 |
| 99% | Rajomon-FIFO | 15 / 17 / 60 / 120 |
| 99% | Rajomon-TailClipper | 15 / 17 / 36 / 75 |
| 99% | Masa | 14 / 17 / 30 / 58 |

Run every point on the evaluation Kubernetes environment:

```bash
./eval/capacity/hotel_capacity_savings/run.sh --k8s
```

The runner writes each point independently under the matching nested path in
`exp/hotel/out/hotel_capacity_savings/`. Check that achieved goodput reaches the path's
target at the committed replica count. Kind can validate an individual point,
for example:

```bash
uv run -m exp_runner run hotel hotel_capacity_savings/p90/masa/3k --kind
```

Kind is suitable for functional validation, not comparison with the paper's
capacity numbers. The largest point provisions 120 bottleneck replicas, so the
full group belongs on the evaluation cluster.
