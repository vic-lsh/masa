# Hotel Search and Reservation

This fixed-capacity experiment compares Masa, Rajomon-FIFO, and
Rajomon-TailClipper while Hotel's Search and Reservation APIs contend at shared
services. Search has a looser SLO and variable database work; Reservation has a
tighter SLO. Queue-latency tracing from the same run supports the per-API CDFs.

The authoritative runner configuration is
[`exp/hotel/in/hotel_search_reservation`](../../../exp/hotel/in/hotel_search_reservation/):

- Offered load: 100 through 8000 RPS across nine load points.
- End-to-end SLOs: 200 ms for Search and 100 ms for Reservation.
- Timing: 25-second warmup and 60-second measurement.
- Policies: Rajomon-FIFO, Rajomon-TailClipper, and Masa.
- Queue-latency tracing: enabled for every policy.

Run on the evaluation Kubernetes environment and generate plots:

```bash
./eval/fixed_capacity/hotel_search_reservation/run.sh --k8s --plot
```

Test deployment wiring on a local Kind cluster:

```bash
./eval/fixed_capacity/hotel_search_reservation/run.sh --kind
```

Kind is suitable for functional validation, not comparison with the paper's
performance numbers. The generic `--smoke-test` check should not be used for a
saturation sweep whose high-load points intentionally exceed system capacity.
