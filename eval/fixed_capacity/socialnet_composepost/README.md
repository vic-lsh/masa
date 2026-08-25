# SocialNet ComposePost

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper on SocialNet's ComposePost API. ComposePost has two
sequential fan-out phases, while cache misses make the remaining work vary
between requests at the same point in the API.

The authoritative runner configuration is
[`exp/socialnet/in/socialnet_composepost`](../../../exp/socialnet/in/socialnet_composepost/):

- Offered load: 800 through 3000 RPS across nine load points.
- End-to-end SLO: 50 ms.
- Timing: 10-second warmup and 30-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/socialnet_composepost/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/socialnet_composepost/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers. The generic `--smoke-test` check should not be used for a
saturation sweep whose high-load points intentionally exceed system capacity.
