# Synthetic Application

The synthetic application is meant to be a benchmark that is as simple as possible, so that we can better understand the basic behavior of deadline policies.

## Endpoints

### a

This endpoint makes two requests sequentially. The first request uses the child random latency distribution from the app config. The second request sends a constant-latency RPC with a per-request duration sampled from an exponential distribution.

### b

This endpoint makes two requests sequentially, like `a`, but with a longer exponential duration for the constant-latency call.

### c

This endpoint executes a configurable call graph defined in the app config under `child_callgraph_c`.

### d

This endpoint executes a configurable call graph defined in the app config under `child_callgraph_d`.

## Call Graph Configuration

To use endpoints `c` and `d`, define `child_callgraph_services` along with `child_callgraph_c` and `child_callgraph_d` in the app config. Each hop references a `service_id` from `child_callgraph_services`, chooses `latency_kind` (`random` or `constant`), and sets a `busy_spin_prob` to randomize compute vs sleep per call. For constant hops, `duration_us` can override the default constant latency distribution.
