# Deployment Generators

This directory contains deployment generators that create platform-specific manifests from topology and experiment configurations.

## Overview

Phase 3 of the deployment scheme redesign introduces two deployment generators:

1. **ComposeGenerator** - Generates `docker-compose.yaml` for Docker Compose deployments
2. **HelmValuesGenerator** - Generates `values.yaml` for Kubernetes/Helm deployments

Both generators take the same inputs (TopologySpec + ExperimentConfigV2) and produce platform-specific outputs.

## Architecture

```
TopologySpec (apps/*/topology.yaml)
    +
ExperimentConfigV2 (exp/*/data/in/*/experiment.yaml)
    |
    v
DeploymentGenerator.generate()
    |
    v
docker-compose.yaml OR values.yaml
```

## Usage

### Programmatic Usage

```python
from exp_runner.runner.generators import ComposeGenerator, HelmValuesGenerator
from exp_runner.runner.topology import TopologySpec
from exp_runner.runner.experiment_config_v2 import ExperimentConfigV2
from pathlib import Path

# Load topology and experiment
topology = TopologySpec.from_yaml(Path("apps/hotel/topology.yaml"))
experiment = ExperimentConfigV2.from_yaml(Path("exp/hotel/data/in/my-exp/experiment.yaml"))

# Generate Docker Compose deployment
compose_gen = ComposeGenerator()
result = compose_gen.generate(
    topology=topology,
    experiment=experiment,
    output_dir=Path("/tmp/output"),
    project_name="hotel-test",
    policy="fifo",
    image_tag="latest",
)
# Result: docker-compose.yaml created at /tmp/output/docker-compose.yaml

# Generate Helm deployment
helm_gen = HelmValuesGenerator()
result = helm_gen.generate(
    topology=topology,
    experiment=experiment,
    output_dir=Path("/tmp/output"),
    project_name="hotel-test",
    policy="fifo",
    image_tag="latest",
)
# Result: values.yaml created at /tmp/output/values.yaml
```

### CLI Usage

The generators are opt-in via the `--use-new-generator` flag:

```bash
# Use new generators (Phase 3)
uv run -m exp_runner run hotel my-exp --use-new-generator

# Default behavior (existing code path)
uv run -m exp_runner run hotel my-exp
```

## ComposeGenerator

Generates `docker-compose.yaml` with:

- **Services**: Application services with correct image tags and replicas
- **Infrastructure**: Databases, caches, message queues
- **Dependencies**: `depends_on` from topology
- **Environment Variables**: Service discovery variables for inter-service communication
- **Resource Limits**: CPU and memory limits
- **Networks and Volumes**: Docker networking and persistent storage

### Environment Variable Mapping

The generator creates environment variables for service discovery:

- `${SERVICE_NAME}_IP` - Service hostname (Docker DNS name)
- `${SERVICE_NAME}_PORT` - Service port
- `${SERVICE_NAME}_REPLICAS` - Number of replicas

For infrastructure services:
- MongoDB: `${NAME}_URI=mongodb://host:27017/dbname`
- Redis: `${NAME}_URL=redis://host:6379`
- Memcached: `${NAME}_ADDR=tcp://host:11211`
- RabbitMQ: `RABBITMQ_URL=amqp://guest:guest@rabbitmq:5672`

### Example Output

```yaml
services:
  frontend:
    image: hotel_frontend:fifo
    scale: 1
    restart: always
    networks:
      - hotel-network
    environment:
      - BINARY_NAME=frontend
      - RATE_SERVICE_IP=${PROJECT_NAME}-rate-service
      - RATE_SERVICE_PORT=8080
      - RATE_SERVICE_REPLICAS=1
    depends_on:
      - rate-service
    volumes:
      - ${APP_CONFIG_PATH}:/usr/config.json:ro
    deploy:
      resources:
        limits:
          cpus: "4"
          memory: "4G"

  rate-service:
    image: rate_service:fifo
    scale: 1
    # ... (similar structure)

  rate-mongo:
    image: mongo:7.0
    restart: always
    networks:
      - hotel-network
    volumes:
      - rate_mongo_data:/data/db
    deploy:
      resources:
        limits:
          cpus: "4"
          memory: "4G"

volumes:
  rate_mongo_data: {}

networks:
  hotel-network:
    driver: bridge
```

## HelmValuesGenerator

Generates `values.yaml` for Helm charts with:

- **Service Configuration**: Replicas, ports, dependencies from topology
- **Image Configuration**: Tags and pull policies
- **Application Config**: API specs, call graphs, methods
- **Resource Limits**: CPU and memory requests/limits
- **Infrastructure**: Database and cache configurations

### Example Output

```yaml
fullnameOverride: hotel-test

image:
  tag: fifo
  pullPolicy: IfNotPresent

services:
  frontend:
    replicas: 1
    port: 8660
    dependsOn:
      - rate-service
      - profile-service

  rate-service:
    replicas: 1
    port: 8080
    dependsOn:
      - rate-mongo
      - rate-redis

infrastructure:
  rate-mongo:
    image: mongo:7.0
    replicas: 1
    port: 27017

  rate-redis:
    image: redis:7.2
    replicas: 1
    port: 6379

resources:
  default:
    limits:
      cpu: "4"
      memory: "4Gi"
    requests:
      cpu: "100m"
      memory: "128Mi"

  frontend:
    limits:
      cpu: "4"
      memory: "4Gi"
    requests:
      cpu: "500m"
      memory: "512Mi"

logLevel: info

serviceAccount:
  create: true
  name: hotel-test-sa
```

## Validation

Both generators validate inputs before generation:

### Topology Validation
- Must specify an app name
- Must have at least one service (or infrastructure for Compose)
- Services should have ports defined (warning if missing)

### Experiment Validation
- Replica overrides must reference valid services
- Experiment must have execution specification (Helm only)

## Replica Overrides

Experiment configs can override default replicas from topology:

```yaml
# topology.yaml
services:
  rate-service:
    default_replicas: 1

# experiment.yaml
spec:
  replica_overrides:
    rate-service: 4  # Override to 4 replicas
```

Both generators apply these overrides consistently.

## Testing

Comprehensive tests in `exp_runner/tests/runner/test_generators.py`:

```bash
# Run generator tests
uv run pytest exp_runner/tests/runner/test_generators.py -v

# Run all tests
uv run pytest exp_runner/tests/
```

Test coverage includes:
- Basic generation for both generators
- Replica override application
- Infrastructure service handling
- Validation errors
- Integration tests comparing both generators

## Design Principles

1. **Separation of Concerns**: Topology (structure) is separate from experiment config (parameters)
2. **Non-Breaking**: Opt-in via `--use-new-generator` flag
3. **Consistency**: Both generators produce equivalent deployments
4. **Validation**: Early validation catches errors before deployment
5. **Testability**: Pure functions for most logic, effects isolated to file I/O

## Future Work

- Phase 4: Simplify plugin interfaces to use generators
- Phase 5: Add K8s support for all apps using HelmValuesGenerator
- Automatic migration of existing experiments to new format
