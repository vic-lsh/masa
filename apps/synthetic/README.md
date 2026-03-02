# Synthetic Application

The synthetic application is meant to be a benchmark that is as simple as possible, so that we can better understand the basic behavior of deadline policies.

## Endpoints

### a

This endpoint makes two requests sequentially. The hops, service targets, optional per-hop latency overrides, and busy-spin probabilities are configured in `request_a_hops`. If more than two hops are configured, all hops execute, but the response fields report only the first two.

### b

This endpoint makes two requests sequentially, like `a`, but uses `request_b_hops`.

## Hop Configuration

Define `child_services` along with `request_a_hops` and `request_b_hops` in the app config. Each hop references a `service_id` from `child_services`, sets an optional `duration_us` mean for an exponential sample, and can provide a `busy_spin_dur_us` to spin for part of the sampled hop duration.

## Call Graph Configuration

The synthetic application supports configurable call graphs that recreate microservice call patterns from trace data. This allows you to define services with methods, where each method has its own latency distribution and can make probabilistic calls to other service::method combinations.

### Structure

A call graph consists of:
- **Services**: Each service has an ID, replica count, and a list of methods
- **Methods**: Each method has:
  - A name
  - A latency distribution (exponential, normal, discrete/bimodal, periodic)
  - A call sequence (vector of maps, where each map is a sequential step)
  - Optional busy spin ratio

### Call Sequence Format

The `call_sequence` field is a vector of maps (JSON objects):
- Each map represents a **sequential step**
- Map keys are `"service_name::method_name"` strings
- Map values are probabilities (0.0 to 1.0)
- Calls within a map execute **in parallel** using a structured `tokio::task::JoinSet`
- Steps execute **sequentially** (wait for all calls in step N before starting step N+1)

### Example Configuration

```json
{
  "call_graph": {
    "entry_point": "MS_56394::GqI6UW1mU4",
    "services": [
      {
        "id": "MS_56394",
        "replicas": 1,
        "methods": [
          {
            "name": "GqI6UW1mU4",
            "latency_distribution": {"Exponential": {"mean": 10000.0}},
            "call_sequence": []
          },
          {
            "name": "method1",
            "latency_distribution": {"Normal": {"mean": 10000.0, "std": 2000.0}},
            "call_sequence": [
              {"MS_37691::y_DKOh-Gts": 1.0},
              {"MS_37691::ykccIz2fkK": 1.0},
              {"MS_73106::Sbvx4Hgp0r": 0.003}
            ]
          },
          {
            "name": "method_with_fanout",
            "latency_distribution": {"Exponential": {"mean": 10000.0}},
            "call_sequence": [
              {"MS_37691::method1": 1.0, "MS_37691::method2": 0.8},
              {"MS_73106::method3": 1.0}
            ]
          }
        ]
      },
      {
        "id": "MS_37691",
        "replicas": 1,
        "methods": [
          {
            "name": "y_DKOh-Gts",
            "latency_distribution": {"Exponential": {"mean": 10000.0}},
            "call_sequence": []
          }
        ]
      }
    ]
  }
}
```

### Latency Distributions

Each method can use one of the following latency distributions:

- **Exponential**: `{"Exponential": {"lambda": 0.0001}}` or `{"Exponential": {"mean": 10000.0}}` (mean = 1/lambda)
- **Normal**: `{"Normal": {"mean": 10000.0, "std": 2000.0}}`
- **Discrete/Bimodal**: `{"Discrete": {"weights": [0.5, 0.5], "values": [5000, 50000]}}`
- **Periodic**: `{"Periodic": {"slow_latency": 50000, "fast_latency": 5000, "slow_duration_ms": 200}}`

### Validation

On startup, the application validates that:
- All referenced `service::method` targets exist in the call graph
- The entry point exists
- All service::method strings are properly formatted

Validation fails fast with clear error messages if any issues are found.

## Estimation Mode

Synthetic supports two estimation modes via config field `estimation_mode`:

- `normal` (default): uses runtime-learned estimators in local policy.
- `perfect_sampled`: synthetic pre-samples per-request child RPC critical-path latency (recursive), per-callee local work, and remaining work after each child call; it attaches these oracle metadata values on child RPCs, and local policy consumes them.

Compile-time override for experiments:

- Build with feature `prio_local_perfect` to force `perfect_sampled` mode regardless of the JSON `estimation_mode` value.
- `prio_local_perfect` is synthetic-only and aliases local-deadline policy (`prio_local`) so it can be listed as a separate policy in experiment `policies` files.

Example:

```json
{
  "estimation_mode": "perfect_sampled",
  "call_graph": {
    "entry_points": {
      "a": [{"MS_1::method1": 1.0}]
    },
    "services": [
      {
        "id": "MS_1",
        "methods": [
          {
            "name": "method1",
            "latency_distribution": {"Exponential": {"mean": 10000.0}},
            "call_sequence": []
          }
        ]
      }
    ]
  }
}
```

### Deployment

When using call graphs:
- Each service instance must have the `SERVICE_ID` environment variable set to its service ID
- Services connect to each other using hostname pattern: `local-{service-id}-service`
- The frontend automatically calls the entry point when the `/a` endpoint is invoked (if call graph is configured)
