# Experiment Workflow in Masa

This document outlines the workflow for running performance experiments in the Masa repository. The system uses a unified Python-based runner to orchestrate experiments across different applications.

## General Workflow

All experiments follow this common pattern:

1.  **Configure**: Create an experiment directory in `exp/<app>/data/in/<experiment_name>/` containing the required configuration files.
2.  **Run**: Execute the experiment using the Python runner: `uv run python -m exp_runner run <app> <experiment_name>`.
3.  **Analyze**: Results are automatically saved to `exp/<app>/data/out/`, and plots can be generated with the `--plot` flag or a separate command.

### 1. Configuration (`exp/<app>/data/in/<experiment_name>/`)

Each application has a committed example configuration folder that you should copy and modify.

**Example Config Locations:**
*   **Hotel**: `exp/hotel/data/in/ci`
*   **MSSIM**: `exp/mssim/data/in/e2e_test`
*   **Socialnet**: `exp/socialnet/data/in/ci`
*   **Synthetic**: `exp/synthetic/data/in/ci`

Every experiment directory **must** contain these two files:

*   **`gen_config.json`**: Configures the load generator.
    ```json
    {
      "Repeats": 1,
      "Apis": ["Search", "Reservation"],
      "Slos": [50000, 50000],
      "Rps": [100, 200, 400],
      "Gap": "const",
      "WarmupSecs": 20,
      "DurationSecs": 40,
      "MaxInFlight": 0,
      "Addr": "http://[::1]:8659"
    }
    ```
*   **`policies`**: A newline-separated list of scheduling policies to test.

In addition to these, each application requires its own specific configuration file (detailed below).

### 2. Running Experiments

Run commands from the repository root.

**Single Experiment:**
```bash
uv run python -m exp_runner run <app> <experiment_name> --plot
```
*   **`<app>`**: `hotel`, `mssim`, `socialnet`, or `synthetic`.
*   **`<experiment_name>`**: The name of the directory created in step 1.
*   **`--plot`**: (Optional) Automatically generate plots after the run.
*   **`--verbose`** / **`-v`**: (Optional) Enable debug logging.

**Multiple Experiments:**
```bash
uv run python -m exp_runner run-multiple <app> "<exp1> <exp2>" --plot
```

### 3. Viewing Results

*   **Raw Data**: Saved in `exp/<app>/data/out/<experiment_name>/`.
    *   Organized by iteration (`0`, `1`, ...) and policy (`fifo`, `prio_global`, etc.).
    *   Contains logs (`*.log`) and trace CSVs.
*   **Plots**: Saved in `exp/<app>/data/plots/<experiment_name>/`.
    *   Includes goodput, latency CDFs, and CPU usage.
    *   To generate plots later: `uv run python -m exp_runner plot <app> <experiment_name>`.

---

## Application-Specific Details

### Hotel
A microservices-based hotel reservation system.

*   **Config File**: `hotel.json`
    *   Defines service topology, replica counts, and database addresses.
    *   Example:
        ```json
        {
          "global": { "hotels": 1000, ... },
          "frontend": { "replicas": 1, ... },
          "rate": { "replicas": 1, ... }
        }
        ```
*   **Additional Plots**: `uv run python -m exp_runner plot-replicas hotel` (generates replica-specific metrics).

### MSSIM
A trace-driven microservice simulator.

*   **Config Files**:
    *   **`mssim.json`**: Points to trace data and simulation parameters.
        ```json
        {
          "callgraph_dirs": ["trace-analysis/graphs/S_14677443"],
          "slo_ms": 100
        }
        ```
    *   **`replicas.json`**: Defines replica counts for the simulated services.
        ```json
        {
          "default": 1,
          "overrides": { "ms-73106": 2 }
        }
        ```

### Synthetic
A configurable synthetic workload for testing specific behaviors.

*   **Config File**: `config.docker.json`
    *   Configures child service behavior (latency distributions, replicas).
    *   Example:
        ```json
        {
            "child_constant_replicas": 1,
            "child_random_latency": {
                "Discrete": { "weights": [0.9, 0.1], "values": [100, 5000] }
            }
        }
        ```

### Socialnet
A social network microservice benchmark.

*   **Config File**: `socialnet.json`
    *   Required to exist but serves as a placeholder for Docker build configurations. Can effectively be an empty JSON object if default build settings are used.
