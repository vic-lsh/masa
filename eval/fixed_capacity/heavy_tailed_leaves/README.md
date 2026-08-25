# Heavy-Tailed Leaves

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper on Alibaba trace workload A2 (`S_38528029`). Its shallow
fan-out reaches leaves with highly variable service time, so the downstream
subset revealed by each request matters more than request age alone.

The authoritative runner configuration is
[`exp/tracebench/in/heavy_tailed_leaves`](../../../exp/tracebench/in/heavy_tailed_leaves/):

- Offered load: 650 through 4400 RPS across seven load points.
- End-to-end SLO: 200 ms.
- Timing: 15-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.
- Trace format: the original marginal-probability call sequence used by this
  workload; Tracebench preserves each child probability independently.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/heavy_tailed_leaves/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/heavy_tailed_leaves/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers.
