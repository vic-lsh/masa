# Wide shallow fan-out

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper on Alibaba trace workload `S_130831269`. The gateway issues
one wide parallel fan-out to 25 services with relatively homogeneous service
times, limiting both downstream-work variation and later scheduling decisions.

The authoritative runner configuration is
[`exp/tracebench/in/wide_shallow_fanout`](../../../exp/tracebench/in/wide_shallow_fanout/):

- Offered load: 550 through 5400 RPS across nine load points.
- End-to-end SLO: 11 ms.
- Timing: 15-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/wide_shallow_fanout/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/wide_shallow_fanout/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers. The generic `--smoke-test` check should not be used for a
saturation sweep whose high-load points intentionally exceed system capacity.
