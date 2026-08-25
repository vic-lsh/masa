# Scheduling Only

This Hotel ablation isolates head-of-line blocking by disabling both gateway
admission and per-service load shedding, so every admitted request runs to
completion. It compares FIFO, current-RPC-deadline, TailClipper, and
least-slack-first dispatch.

The authoritative runner configuration is
[`exp/hotel/in/scheduling_only`](../../../exp/hotel/in/scheduling_only/). It
uses the Search/Reservation workload, their 200/100 ms SLOs, nine load points
around saturation, a 25-second warmup, and a 60-second measurement.

```bash
./eval/ablations/scheduling_only/run.sh --k8s --plot
```

For deployment validation on local Kubernetes:

```bash
./eval/ablations/scheduling_only/run.sh --kind
```
