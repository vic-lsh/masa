# Short sequential branching

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper as offered load crosses system capacity. It uses Alibaba
trace workload `S_14677443`, which has a short sequential path and probabilistic
fan-out at successive hops.

The authoritative runner configuration is
[`exp/tracebench/in/short_sequential_branching`](../../../exp/tracebench/in/short_sequential_branching/):

- Offered load: 500, 600, 800, 1000, 1300, and 1800 RPS.
- End-to-end SLO: 100 ms.
- Timing: 15-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/short_sequential_branching/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/short_sequential_branching/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers. The generic `--smoke-test` check also should not be used
for this saturation sweep because it expects achieved goodput to remain within
20% of every offered load.
