# Service-time shift

This adaptation experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper when one API becomes slower during a run. Two equally
weighted APIs share a CPU-bound front service and use separate leaves.

The authoritative runner configuration is
[`exp/synthbench/in/service_time_shift`](../../../exp/synthbench/in/service_time_shift/):

- Offered load: 500 RPS per API.
- End-to-end SLOs: 50 ms for API A and 500 ms for API B.
- API A leaf: Normal(5 ms, 0.5 ms), shifting to Normal(22 ms, 1.5 ms).
- API B leaf: unchanged Exponential distribution with a 5 ms mean.
- Timing: 15-second warmup and 90-second measurement; the shift occurs 30
  seconds into the measurement.

The shifted distribution starts its timer on the first request. Consequently,
the committed `after_secs` value is 45 seconds: the 15-second warmup plus 30
seconds of measured traffic before the shift.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/adaptation/service_time_shift/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/adaptation/service_time_shift/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers.
