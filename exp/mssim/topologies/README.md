# MSSIM Topologies

This directory contains topology definitions for MSSIM (Microservice Simulator) experiments.

## Overview

MSSIM topologies are generated from real-world Alibaba cluster traces. The topology files here serve as references and allow for customization (e.g., replica count overrides).

## Trace Sources

The actual call graphs, latency distributions, and method definitions are stored in:
- `trace-analysis/golden/<trace_id>/call_sequence.json` - Call graph structure
- `trace-analysis/golden/<trace_id>/latency_percentiles.json` - Latency distributions
- `trace-analysis/golden/<trace_id>/interface_distribution.json` - Method probabilities

## Available Topologies

- `S_14677443.yaml` - Alibaba trace S_14677443 (6 microservices)
- `S_32048416.yaml` - Alibaba trace S_32048416

## Usage

Reference these topologies in experiment configs:

```yaml
spec:
  topology_ref: S_14677443
  replica_overrides:
    MS_56394: 2  # Scale up this service
```

The MSSIM plugin will load the full topology from trace-analysis at runtime.
