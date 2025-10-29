# Microservices Evaluator

This project evaluates microservices with various synthetic workloads.

It supports:

1. Arbitrary call graph definitions via a config file
2. Config file-defined service time distribution per service
3. Custom load generation logic for this call graph

NOTE: the following docs are up-to-date as of commit 6aad65c3fb6b6472d593eaff8af6e81882d0c099 .

## Getting started

Minimal example to run Alibaba trace replay:

```bash
$ cd <this-directory>/simulator
$ cargo run -- --alibaba-trace <masa-project-root>/trace-analysis/golden/S_86516878 --config-dir ./example_config
```

Once the Docker compose cluster has started, you can view each service container's
resource usage via `docker stats`.

To monitor logs from the load generator, run `docker logs -f load_generator`.

## Orchestrator backends

The simulator supports multiple execution backends when replaying traces. By default it
generates a `docker-compose.yml` file and runs the workload with Docker Compose. To opt
into the Kubernetes backend pass `--orchestrator-backend k8s` to the simulator (for
example `cargo run -- --alibaba-trace ./traces/... --orchestrator-backend k8s`). The K8s
path builds Docker images locally, creates ConfigMaps from trace/config files, writes a
`k8s-manifest.yaml`, applies it with `kubectl`, and tears it down on Ctrl-C.

### Kubernetes Setup

The simulator works with any Kubernetes cluster (Minikube, kind, Docker Desktop, GKE, EKS, AKS, etc.).

**Requirements:**
- `kubectl` configured to access your cluster
- `docker` for building images

**How it works:**

1. The simulator builds Docker images locally using your local Docker daemon
2. Trace data and service configurations are packaged into Kubernetes ConfigMaps
3. The generated manifest uses these ConfigMaps to provide configuration to pods
4. Images use `imagePullPolicy: IfNotPresent` for flexibility

**For local clusters (Minikube, kind, Docker Desktop):**

The simulator builds images locally, which you can then make available to your cluster:

```bash
# For Minikube: Load images after building
# The simulator builds them, then run:
minikube image load mssim/generic-service:latest
minikube image load mssim/load-generator:latest

# Alternative for Minikube: Build directly in Minikube's daemon
# Run this BEFORE starting the simulator:
eval $(minikube docker-env)
# Then run the simulator - images will be built in Minikube's daemon

# For kind: Load images after building
kind load docker-image mssim/generic-service:latest
kind load docker-image mssim/load-generator:latest

# For Docker Desktop: Images are already available (shares host Docker daemon)
```

**For remote clusters (GKE, EKS, AKS, etc.):**

Push images to a container registry accessible by your cluster:

```bash
# Tag and push to your registry
docker tag mssim/generic-service:latest your-registry.com/mssim/generic-service:latest
docker tag mssim/load-generator:latest your-registry.com/mssim/load-generator:latest
docker push your-registry.com/mssim/generic-service:latest
docker push your-registry.com/mssim/load-generator:latest

# Update image tags in the code or manifest before applying
```

## Running on new Alibaba call graphs

The above example uses pre-generated call graphs ran from previous Alibaba trace
analysis. You may also generate new graphs using graph analysis scripts in
this repository.

1. Fetch the Alibaba trace submodule by running this in `<masa-project-root>`:
   ```bash
   git submodule update --init --recursive
   ```
2. Fetch compressed Alibaba trace:
   ```bash
   # Note that we use the 2022 version, not 2021
   $ cd <masa-project-root>/traces/alibaba/cluster-trace-microservices-v2022

   # Fetch data using Alibaba-supplied script. See the README in this directory.
   # You likely only need to fetch a subset of the dataset.
   $ ./fetchData.sh <args>

   # Decompress data util script
   $ cd <masa-project-root>/trace-analysis/
   $ ./decompress_call_graph.sh <dataset_start_idx> <dataset_end_idx>
   ```
3. Generate call graphs using our analysis script:
   ```bash

   $ cd <masa-project-root>/trace-analysis

   # We use uv to maange python dependencies, but feel free to use your own venv.
   # There're various parameters you can tune in the script itself.
   $ uv run ./analysis.py
   ```

You should see call graphs being generated in directories `graph_reports` and
`plots` under `<masa-project-root>/trace-analysis/`.
