# Synthetic Application

The synthetic application is meant to be a benchmark that is as simple as possible, so that we can better understand the basic behavior of deadline policies.

## Call Graph Configuration

The synthetic application supports configurable call graphs that recreate microservice call patterns from trace data. This allows you to define services with methods, where each method has its own latency distribution and can make probabilistic calls to other service::method combinations.

### Structure

The config is organized into:
- **services**: Shared services available across graphs
- **call_graphs**: Named graphs, each with:
  - `entry_point` (service::method)
  - `services` (graph-specific services)
  - `service_refs` (IDs of shared services)
- **apis**: API names mapped to call graphs with traffic weights

### Call Sequence Format

The `call_sequence` field is a vector of maps (JSON objects):
- Each map represents a **sequential step**
- Map keys are `"service_name::method_name"` strings
- Map values are probabilities (0.0 to 1.0)
- Calls within a map execute **in parallel** using `tokio::spawn` and `join`
- Steps execute **sequentially** (wait for all calls in step N before starting step N+1)

### Example Configuration

```json
{
  "services": [
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
  ],
  "call_graphs": {
    "graph1": {
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
              "name": "method_with_fanout",
              "latency_distribution": {"Exponential": {"mean": 10000.0}},
              "call_sequence": [
                {"MS_37691::y_DKOh-Gts": 1.0}
              ]
            }
          ]
        }
      ],
      "service_refs": ["MS_37691"]
    }
  },
  "apis": [
    {"name": "web_checkout_api", "call_graph": "graph1", "traffic_weight": 0.8},
    {"name": "mobile_status_api", "call_graph": "graph1", "traffic_weight": 0.2}
  ]
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
- All referenced `service::method` targets exist in the merged services list
- Each graph entry point exists
- Each API references a valid call graph

Validation fails fast with clear error messages if any issues are found.

### Deployment

When using call graphs:
- Each service instance must have the `SERVICE_ID` environment variable set to its service ID
- Services connect to each other using hostname pattern: `local-{service-id}-service`
- The frontend routes based on the API name carried in the request context
