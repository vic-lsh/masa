# Scheduling with Overload Control

This Hotel ablation isolates dispatch order while holding Masa's per-service
load shedding and gateway admission control enabled. It compares FIFO,
TailClipper's oldest-request-first order, and least-slack-first scheduling.

The authoritative runner configuration is
[`exp/hotel/in/scheduling_overload_control`](../../../exp/hotel/in/scheduling_overload_control/).
It uses the Search/Reservation workload, their 200/100 ms SLOs, ten load
points through 4000 RPS, a 25-second warmup, and a 60-second measurement.

```bash
./eval/ablations/scheduling_overload_control/run.sh --k8s --plot
```

For deployment validation on local Kubernetes:

```bash
./eval/ablations/scheduling_overload_control/run.sh --kind
```
