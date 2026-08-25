# Mixed Paths

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper when Alibaba trace workloads A3 (`S_14677443`) and A4
(`S_32048416`) contend at a shared downstream service. The two APIs have
different path depth and remaining work but the same end-to-end SLO.

The authoritative runner configuration is
[`exp/tracebench/in/mixed_paths`](../../../exp/tracebench/in/mixed_paths/):

- Offered load: 600, 1000, 2000, 3000, 4000, and 6000 RPS.
- End-to-end SLO: 100 ms for both APIs.
- Timing: 15-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/mixed_paths/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/mixed_paths/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers.
