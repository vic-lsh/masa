# Microservices Evaluator

This project evaluates microservices with various synthetic workloads.

It supports:

1. Arbitrary call graph definitions via a config file
2. Config file-defined service time distribution per service
3. Custom load generation logic for this call graph

## Running Experiments

The recommended way to run MSSIM experiments is using the experiment runner script. This script automates the process of building Docker images, running experiments across multiple policies and RPS values, and organizing output data.

### Prerequisites

- Docker installed and running
- Cargo (Rust toolchain) installed
- Python 3 installed

### Quick Start

1. Create an experiment configuration file (JSON format). See `exp/mssim/config/template.json` for a template:

```json
{
  "experiment_name": "my_experiment",
  "callgraph_dirs": ["trace-analysis/golden/S_86516878"],
  "config_dir": "apps/mssim/simulator/example_config/",
  "output_root": "exp/mssim/data/experiments/",
  "duration_sec": 60,
  "repeats": 1,
  "slo_ms": 100,
  "policies": ["fifo", "prio_global"],
  "rps_values": [200, 300, 400],
  "max_in_flight": 10000,
  "stats_interval_sec": 2,
  "extra_env": {}
}
```

For multiple call graphs, specify multiple directories:
```json
{
  "callgraph_dirs": [
    "trace-analysis/golden/S_86516878",
    "trace-analysis/golden/S_14677443"
  ],
  ...
}
```

2. Run the experiment:

```bash
$ cd <masa-project-root>
$ python3 exp/mssim/scripts/experiment.py --config exp/mssim/config/test_config.json
```

The script will:
- Build the load generator Docker image
- Build generic service images for each unique policy
- Run experiments for each combination of policy and RPS value
- Save results to `{output_root}/{experiment_name}/{policy}/rps_{rps_value}/run_{repeat_id}/`

### Configuration Options

- `experiment_name`: Name of the experiment (used for output directory)
- `callgraph_dirs`: List of paths to call graph directories (relative to repo root). Each directory should contain `edges.csv`, `interface_distribution.json`, `latency_percentiles.json`, and optionally `call_sequence.json`. For multiple call graphs, services are unioned (each service deployed once) and requests are routed by graph_name.
- `config_dir`: Path to the simulator config directory (relative to repo root)
- `output_root`: Root directory for experiment outputs (relative to repo root)
- `duration_sec`: Duration of each experiment run in seconds
- `repeats`: Number of times to repeat each experiment
- `slo_ms`: Service level objective latency in milliseconds
- `policies`: List of scheduling policies to test (e.g., `["fifo", "prio_global"]`)
- `rps_values`: List of requests per second values to test
- `max_in_flight`: Maximum number of in-flight requests
- `stats_interval_sec`: Interval for collecting statistics
- `extra_env`: Additional environment variables to pass to the experiment

### Example Test Script

See `scripts/test_mssim_experiment.sh` for a complete example of running an experiment and validating the output.

### Output Structure

Results are organized as:
```
{output_root}/{experiment_name}/{policy}/rps_{rps_value}/run_{repeat_id}/
├── metadata.json          # Experiment metadata
├── docker-compose.yml     # Generated docker-compose configuration
├── deployment.json        # Generated deployment configuration
├── orchestrator.log      # Orchestrator logs
└── root_latencies_{rps}rps.csv  # Latency measurements
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
