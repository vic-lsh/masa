# Synthetic Application

The synthetic application is meant to be a benchmark that is as simple as possible, so that we can better understand the basic behavior of deadline policies.

## Endpoints

### a

This endpoint makes two requests sequentially. The hops, service targets, optional per-hop latency overrides, and busy-spin probabilities are configured in `request_a_hops`. If more than two hops are configured, all hops execute, but the response fields report only the first two.

### b

This endpoint makes two requests sequentially, like `a`, but uses `request_b_hops`.

## Hop Configuration

Define `child_services` along with `request_a_hops` and `request_b_hops` in the app config. Each hop references a `service_id` from `child_services`, sets an optional `duration_us` override, and provides a `busy_spin_prob` to randomize compute vs sleep per call.
