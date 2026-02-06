# Synthetic Topologies

This directory contains pre-defined call graph topologies for synthetic benchmarking experiments.

## Overview

Synthetic topologies define microservice architectures with configurable:
- Service dependencies (fanout, chain, tree structures)
- Latency distributions per method
- Call patterns and probabilities

## Available Topologies

- `fanout-3.yaml` - Simple 3-way fanout (1 root → 3 children)
- `fanout-5.yaml` - 5-way fanout for high concurrency testing
- `chain-2.yaml` - 2-hop sequential chain topology

## Usage

Reference these topologies in experiment configs:

```yaml
kind: Experiment
metadata:
  name: fanout-test
  app: synthetic

spec:
  topology_ref: fanout-3

  execution:
    policies: [fifo, prio_global]
    repeats: 3

  loadgen:
    rps: [100, 200, 300]
    apis:
      - name: api_a
        slo_us: 100000
```

## Creating Custom Topologies

Add new YAML files following the format:

```yaml
kind: Topology
metadata:
  app: synthetic
  description: "Your topology description"

call_graph:
  entry_points:
    api_a:
      - MS_service::method: 1.0

  services:
    - id: MS_service
      default_replicas: 1
      methods:
        - name: method
          latency_distribution:
            Exponential:
              mean: 5000  # microseconds
          call_sequence:
            - MS_child::process: 1.0
```

See existing files for more examples.
