# MASA Experiment Runner

A Python-based experiment orchestration system for running performance experiments across different applications with various scheduling policies.

## Overview

The experiment runner replaces the previous bash script system with a well-structured Python module that provides better maintainability, error handling, and extensibility while maintaining full compatibility with existing experiment configurations.

## Features

- **Multiple Applications**: Supports hotel, socialnet, synthbench, and tracebench applications with extensible plugin architecture
- **Policy Testing**: Run experiments with different scheduling policies (sched_fifo, sched_slo, sched_slo,sched_pred, etc.)
- **Automated Workflow**: Handles Docker builds, service orchestration, load generation, and log collection
- **Result Analysis**: Integrated plotting for goodput, latency, and hotel replica metrics
- **Type Safety**: Uses Python dataclasses for configuration validation
- **Better Logging**: Comprehensive logging with configurable verbosity

## Installation

The experiment runner is part of the MASA repository. Ensure you have Python 3.10+ and the required dependencies.

### Ubuntu System Dependencies

```bash
sudo apt update
sudo apt install build-essential graphviz libgraphviz-dev
```

### Python Dependencies

```bash
# Install uv if not already present
curl -LsSf https://astral.sh/uv/install.sh | sh

# Install Python dependencies
uv sync
```

## Quick Start

### Run a Single Experiment

```bash
# Run experiment 'exp1' for the hotel application
uv run -m exp_runner run hotel exp1

# Run experiment 'exp1' for the socialnet application
uv run -m exp_runner run socialnet exp1

# Run a Tracebench experiment
uv run -m exp_runner run tracebench e2e_test

# Run with plot generation
uv run -m exp_runner run hotel exp1 --plot
uv run -m exp_runner run socialnet exp1 --plot

# Run with verbose logging
uv run -m exp_runner run hotel exp1 --plot --verbose

# Print what would run (no containers started)
uv run -m exp_runner run tracebench e2e_test --dry-run
```

### Queue Multiple Experiments

```bash
# Run multiple experiments sequentially
uv run -m exp_runner run-multiple hotel "exp1 exp2 exp3" --plot

# For synthbench application
uv run -m exp_runner run-multiple synthbench "quick_test template-presampled" --plot
```

### Generate Plots Only

```bash
# Generate plots from existing experiment output
uv run -m exp_runner plot hotel exp1
uv run -m exp_runner plot tracebench e2e_test

# Generate replica plots from hotel inputs
uv run -m exp_runner plot-replicas hotel
```

## Experiment Configuration

Experiments are configured using files in `exp/<app>/data/in/<experiment_name>/`:

### Required Files

1. **`gen_config.json`** - Load generator configuration
   ```json
   {
     "Repeats": 1,
     "Apis": ["Search", "Reservation"],
     "ApiWeights": [1, 1],
     "Slos": [50000, 50000],
     "Timeouts_ms": [1000, 1000],
     "Rps": [100, 200, 400, 600, 800],
     "Gap": "const",
     "WarmupSecs": 20,
     "DurationSecs": 40,
     "MaxInFlight": 0,
     "MatchedDeadlineBurstSize": 1,
     "Addr": "http://[::1]:8659"
   }
   ```
   
   **Configuration Fields:**
   - `MaxInFlight`: Maximum number of concurrent in-flight requests (0 = unlimited, default: 0)
   - `MatchedDeadlineBurstSize`: Requests emitted together with one gateway-entry
     timestamp; the configured RPS remains the total request rate (default: 1).
   - `ApiWeights`: Optional relative request mix weights for `Apis`; omitted means uniform.

2. **`policies`** - Whitespace-separated list of scheduling policies to test
   ```
   sched_fifo sched_slo sched_slo,sched_pred
   ```

3. **Application-specific config** (varies by app):
   - Hotel: `hotel.json` - Service replica counts and configuration
   - Socialnet: `socialnet.json` - Placeholder config for Docker builds (can be empty)
   - Synthbench: `config.docker.json` - Child service configuration (optional)
   - Tracebench: `tracebench.json` - Trace/config inputs and Tracebench-specific parameters

### Tracebench Configuration

Tracebench experiments live under `exp/tracebench/data/in/<experiment_name>/` and require:

1. **`gen_config.json`** (Tracebench subset)
   ```json
   {
     "Repeats": 1,
     "Rps": [200, 300, 400],
     "DurationSecs": 60,
     "WarmupSecs": 10,
     "MaxInFlight": 10000
   }
   ```

2. **`policies`** (whitespace-separated)
   ```
   sched_fifo sched_slo
   ```

3. **`tracebench.json`**
   ```json
   {
     "callgraph_dirs": ["trace-analysis/graphs/S_14677443"],
     "config_dir": "apps/tracebench/tracebench/example_config/",
     "slo_ms": 100,
     "orchestrator": "localhost:50051",
     "replay_path": null,
     "stats_interval_sec": 2,
     "max_in_flight": 10000,
     "extra_env": {}
   }
   ```
   
   **Note:** `callgraph_dirs` is a list of call graph directories. For multiple call graphs:
   ```json
   {
     "callgraph_dirs": [
       "trace-analysis/graphs/S_14677443",
       "trace-analysis/graphs/S_86516878"
     ],
     ...
   }
   ```

### Example: Hotel Application

```bash
exp/hotel/data/in/exp1/
├── gen_config.json       # Load generator settings
├── hotel.json            # Hotel-specific configuration
└── policies              # Scheduling policies to test
```

### Example: Socialnet Application

```bash
exp/socialnet/data/in/exp1/
├── gen_config.json       # Load generator settings
├── socialnet.json        # Placeholder config (can be empty)
└── policies              # Scheduling policies to test
```

### Example: Synthbench Application

```bash
exp/synthbench/data/in/quick_test/
├── gen_config.json       # Load generator settings
├── config.docker.json    # Optional synthbench config
└── policies              # Scheduling policies to test
```

## Output Structure

Results are saved to `exp/<app>/data/out/<experiment_name>/`:

```
exp/hotel/data/out/exp1/
├── 0/                           # First iteration
│   ├── sched_fifo/              # Results for sched_fifo policy
│   │   ├── loadgen.log         # Load generator output
│   │   ├── local-hotel-frontend-service-1.log  # Frontend container logs
│   │   ├── local-rate-service-1.log
│   │   ├── *.csv               # Trace files
│   │   └── ...
│   ├── sched_slo/              # Results for sched_slo policy
│   └── sched_slo,sched_pred/   # Results for sched_slo,sched_pred policy
├── 1/                          # Second iteration (if Repeats > 1)
└── done                        # Marker file when complete
```

Plots are generated in `exp/<app>/data/plots/<experiment_name>/`.
Replica plots are generated in `exp/hotel/data/plots/replicas/`.

### Tracebench Output Layout

Tracebench uses the standard runner output root, with per-RPS subdirectories under each policy:

```
exp/tracebench/data/out/e2e_test/
├── 0/
│   ├── sched_fifo/
│   │   └── rps_200/
│   │       └── run_0/
│   │           ├── docker-compose.yml
│   │           ├── deployment.json
│   │           ├── metadata.json
│   │           └── orchestrator.log
│   └── sched_slo/
│       └── rps_200/
│           └── run_0/
└── done
```

## Command Reference

### run

Run a single performance experiment.

```bash
uv run -m exp_runner run <app> <experiment> [options]
```

**Arguments:**
- `<app>`: Application name (`hotel`, `tracebench`, or `synthbench`)
- `<experiment>`: Experiment name (must exist in `exp/<app>/data/in/`)

**Options:**
- `--plot`: Generate plots after experiment completion
- `--no-cache`: Disable Docker cache during build
- `--verbose, -v`: Enable verbose (DEBUG) logging
- `--dry-run`: Print what would be executed without running containers

**Example:**
```bash
uv run -m exp_runner run hotel exp1 --plot --verbose
```

### run-multiple

Run multiple experiments sequentially.

```bash
uv run -m exp_runner run-multiple <app> "<exp1> <exp2> ..." [options]
```

**Arguments:**
- `<app>`: Application name (`hotel`, `tracebench`, or `synthbench`)
- `"<experiments>"`: Space-separated list of experiment names (must be quoted)

**Options:**
- `--plot`: Generate plots after each experiment
- `--no-cache`: Disable Docker cache during builds
- `--verbose, -v`: Enable verbose (DEBUG) logging
- `--dry-run`: Print what would be executed without running containers

**Example:**
```bash
uv run -m exp_runner run-multiple hotel "exp1 exp2 exp3" --plot
```

### plot

Generate plots for an existing experiment.

```bash
uv run -m exp_runner plot <app> <experiment>
```

**Arguments:**
- `<app>`: Application name (`hotel`, `tracebench`, or `synthbench`)
- `<experiment>`: Experiment name to generate plots for

**Example:**
```bash
uv run -m exp_runner plot hotel exp1
uv run -m exp_runner plot tracebench e2e_test
```

### plot-replicas

Generate replica plots for hotel experiments by scanning `exp/hotel/data/in`.

```bash
uv run -m exp_runner plot-replicas hotel
```

**Arguments:**
- `<app>`: Application name (`hotel` only)

**Output:**
- `exp/hotel/data/plots/replicas/`

## Migration from Bash Scripts

The Python runner is a drop-in replacement for the bash scripts with identical behavior:

### Before (Bash)
```bash
cd exp/hotel
./scripts/run-experiment.sh exp1 --plot

# Or from anywhere
cd /path/to/masa
./exp/hotel/scripts/run-experiment.sh exp1 --plot
```

### After (Python)
```bash
cd /path/to/masa
uv run -m exp_runner run hotel exp1 --plot
```

### Key Differences

1. **Location**: Python runner can be called from anywhere in the repo (no need to `cd` to exp directory)
2. **Explicit app name**: Must specify app name as first argument (`hotel`, `synthbench`)
3. **Better error messages**: Clearer validation and error reporting
4. **Logging**: Use `--verbose` flag for detailed logs instead of shell tracing

## Adding New Applications

To add support for a new application:

1. Create a new plugin class in `exp_runner/runner/apps/your_app.py`:

```python
from .base import AppPlugin, DockerConfig

class YourApp(AppPlugin):
    def get_app_name(self) -> str:
        return "your_app"
    
    def load_app_config(self, config_path: Path) -> dict:
        # Load your app's config file
        pass
    
    def generate_env_vars(self, gen_config: dict, app_config: dict, app_dir: Path) -> dict:
        # Generate environment variables for docker-compose
        pass
    
    def get_docker_config(self) -> DockerConfig:
        # Return Docker configuration
        pass
    
    def get_container_names(self, env_vars: dict) -> list[str]:
        # Return container names for log collection
        pass
```

2. Register the plugin in `exp_runner/runner/apps/__init__.py`:

```python
from .your_app import YourApp

def get_app_plugin(app_name: str) -> AppPlugin:
    apps = {
        "hotel": HotelApp,
        "synthbench": SynthbenchApp,
        "your_app": YourApp,  # Add your app
    }
    # ...
```

3. Update CLI choices in `exp_runner/runner/cli.py` to include your app name.

## Architecture

The runner is organized into several modules:

- **`cli.py`** - Command-line interface using argparse
- **`config.py`** - Configuration loading and validation
- **`experiment.py`** - Main orchestration logic
- **`experiment_driver.py`** - Single workload orchestrator (`ExpDriver`)
- **`deployment_manager.py`** - Abstract base for deployment backends
- **`docker_manager.py`** - Docker operations
- **`k8s_manager.py`** - Kubernetes operations
- **`utils.py`** - Common utilities (polling, etc.)
- **`exceptions.py`** - Custom exception types
- **`apps/`** - Application plugins
  - `base.py` - Abstract base class for app plugins
  - `hotel.py` - Hotel application implementation
  - `synthbench.py` - Synthbench application implementation
  - `tracebench.py` - Tracebench application implementation (Unified in Phase 2)
- **`plotting/`** - Result visualization
  - `goodput.py` - Goodput plot generation
  - `latency.py` - Latency plot generation
  - `replicas.py` - Hotel replica plot generation
  - `util.py` - Plotting utilities

## Unified Architecture (Phase 2 & 3)

The runner now uses a unified architecture for all applications, including Tracebench which was previously separate.
- **`ExpDriver`**: Central orchestrator for all workloads.
- **`DeploymentManager`**: Abstract base class for Docker and Kubernetes backends.
- **`AppPlugin`**: Interface for application-specific logic.

Key improvements in Phase 3:
- **Robustness**: Replaced fixed sleeps with smart polling (`wait_until`).
- **Error Handling**: Specific exceptions (`DeploymentError`, `LoadGenError`) for better failure reporting.
- **K8s Support**: Unified K8s deployment logic via `K8sManager`.

## Troubleshooting

### "Not in a git repository" error
Ensure you're running the command from within the MASA repository.

### "Experiment directory not found" error
Check that the experiment exists: `ls exp/<app>/data/in/`

### "Required app config not found" error
For hotel experiments, ensure `hotel.json` exists in the experiment input directory.

### Docker errors
- Ensure Docker daemon is running: `docker ps`
- Check Docker Compose is installed: `docker compose version`
- Verify sufficient disk space for images and containers

### Import errors
Ensure you're using Python 3.10+ and all dependencies from `pyproject.toml` are installed.

## Development

### Running Tests
```bash
# Test CLI with dry-run commands
uv run -m exp_runner --help
uv run -m exp_runner run --help

# Run a quick test experiment
uv run -m exp_runner run synthbench quick_test --verbose
```

### Logging
Set logging levels programmatically or use `--verbose` flag:
```python
import logging
logging.getLogger('exp_runner.runner').setLevel(logging.DEBUG)
```

## License

Part of the MASA project. See repository root for license information.
