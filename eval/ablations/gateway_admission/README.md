# Gateway Admission

This Hotel ablation holds least-slack-first scheduling and per-service load
shedding fixed while toggling progressive gateway admission. It isolates the
additional value of bounding incoming work before it enters the service graph.

The authoritative runner configuration is
[`exp/hotel/in/gateway_admission`](../../../exp/hotel/in/gateway_admission/).
It uses the Search/Reservation workload, their 200/100 ms SLOs, eleven load
points through 4000 RPS, a 25-second warmup, and a 60-second measurement.

```bash
./eval/ablations/gateway_admission/run.sh --k8s --plot
```

For deployment validation on local Kubernetes:

```bash
./eval/ablations/gateway_admission/run.sh --kind
```
