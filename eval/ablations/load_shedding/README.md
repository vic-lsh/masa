# Per-Service Load Shedding

This Hotel ablation holds least-slack-first scheduling and gateway admission
fixed while changing what happens when an in-flight request no longer has
enough slack. Full Masa returns the request; the no-drop variant reports the
same signal to admission control but continues executing the request.

The authoritative runner configuration is
[`exp/hotel/in/load_shedding`](../../../exp/hotel/in/load_shedding/). It uses
the Search/Reservation workload, their 200/100 ms SLOs, six load points
through 4000 RPS, a 25-second warmup, and a 60-second measurement.

```bash
./eval/ablations/load_shedding/run.sh --k8s --plot
```

For deployment validation on local Kubernetes:

```bash
./eval/ablations/load_shedding/run.sh --kind
```
