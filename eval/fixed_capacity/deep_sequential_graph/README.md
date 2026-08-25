# Deep Sequential Graph

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper on Alibaba trace workload A5 (`S_5991695`). Its long,
diverse sequential paths repeatedly reveal new downstream work, making it a
stress test for slack updates and per-service load shedding.

The authoritative runner configuration is
[`exp/tracebench/in/deep_sequential_graph`](../../../exp/tracebench/in/deep_sequential_graph/):

- Offered load: 100, 300, 450, 600, 900, and 1200 RPS.
- End-to-end SLO: 100 ms.
- Timing: 15-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/deep_sequential_graph/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/deep_sequential_graph/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers.
