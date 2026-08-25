# Shared-Service Mix

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper on Alibaba trace workload A6. Two entry APIs
(`S_86516878` and `S_98493745`) share five internal services while retaining
different end-to-end SLOs and downstream work.

The authoritative runner configuration is
[`exp/tracebench/in/shared_service_mix`](../../../exp/tracebench/in/shared_service_mix/):

- Offered load: 600 through 6000 RPS across seven load points.
- End-to-end SLOs: 100 ms for `S_86516878` and 75 ms for `S_98493745`.
- Timing: 15-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/shared_service_mix/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/shared_service_mix/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers.
