# MASA Experiment Runner

A Python-based experiment orchestration system for running performance experiments across different applications with various scheduling policies.

## Overview

The experiment runner replaces the previous bash script system with a well-structured Python module that provides better maintainability, error handling, and extensibility while maintaining full compatibility with existing experiment configurations.

## Features

- **Multiple Applications**: Supports hotel and synthetic applications with extensible plugin architecture
- **Policy Testing**: Run experiments with different scheduling policies (fifo, prio_global, prio_local, etc.)
- **Automated Workflow**: Handles Docker builds, service orchestration, load generation, and log collection
- **Result Analysis**: Integrated plotting for goodput and latency metrics
- **Type Safety**: Uses Python dataclasses for configuration validation
- **Better Logging**: Comprehensive logging with configurable verbosity

## Installation

The experiment runner is part of the MASA repository. Ensure you have Python 3.10+ and the required dependencies:

```bash
# The runner uses existing Python dependencies from pyproject.toml
# No additional installation required
```

## Quick Start

### Run a Single Experiment

```bash
# Run experiment 'exp1' for the hotel application
python -m exp.runner run hotel exp1

# Run with plot generation
python -m exp.runner run hotel exp1 --plot

# Run with verbose logging
python -m exp.runner run hotel exp1 --plot --verbose
```

### Queue Multiple Experiments

```bash
# Run multiple experiments sequentially
python -m exp.runner run-multiple hotel "exp1 exp2 exp3" --plot

# For synthetic application
python -m exp.runner run-multiple synthetic "quick_test template-presampled" --plot
```

### Generate Plots Only

```bash
# Generate plots from existing experiment output
python -m exp.runner plot hotel exp1
```

## Experiment Configuration

Experiments are configured using files in `exp/<app>/data/in/<experiment_name>/`:

### Required Files

1. **`gen_config.json`** - Load generator configuration
   ```json
   {
     "Repeats": 1,
     "Apis": ["Search", "Reservation"],
     "Slos": [50000, 50000],
     "Rps": [100, 200, 400, 600, 800],
     "Gap": "const",
     "WarmupSecs": 20,
     "DurationSecs": 40,
     "Concurrency": 0,
     "Addr": "http://[::1]:8659"
   }
   ```

2. **`policies`** - Whitespace-separated list of scheduling policies to test
   ```
   fifo prio_global prio_local
   ```

3. **Application-specific config** (varies by app):
   - Hotel: `hotel.json` - Service replica counts and configuration
   - Synthetic: `config.docker.json` - Child service configuration (optional)

### Example: Hotel Application

```bash
exp/hotel/data/in/exp1/
├── gen_config.json       # Load generator settings
├── hotel.json            # Hotel-specific configuration
└── policies              # Scheduling policies to test
```

### Example: Synthetic Application

```bash
exp/synthetic/data/in/quick_test/
├── gen_config.json       # Load generator settings
├── config.docker.json    # Optional synthetic config
└── policies              # Scheduling policies to test
```

## Output Structure

Results are saved to `exp/<app>/data/out/<experiment_name>/`:

```
exp/hotel/data/out/exp1/
├── 0/                           # First iteration
│   ├── fifo/                    # Results for fifo policy
│   │   ├── loadgen.log         # Load generator output
│   │   ├── hotel_frontend.log  # Frontend container logs
│   │   ├── local-rate-service-1.log
│   │   ├── *.csv               # Trace files
│   │   └── ...
│   ├── prio_global/            # Results for prio_global policy
│   └── prio_local/             # Results for prio_local policy
├── 1/                          # Second iteration (if Repeats > 1)
└── done                        # Marker file when complete
```

Plots are generated in `exp/<app>/data/plots/<experiment_name>/`.

## Command Reference

### run

Run a single performance experiment.

```bash
python -m exp.runner run <app> <experiment> [options]
```

**Arguments:**
- `<app>`: Application name (`hotel` or `synthetic`)
- `<experiment>`: Experiment name (must exist in `exp/<app>/data/in/`)

**Options:**
- `--plot`: Generate plots after experiment completion
- `--no-cache`: Disable Docker cache during build
- `--verbose, -v`: Enable verbose (DEBUG) logging

**Example:**
```bash
python -m exp.runner run hotel exp1 --plot --verbose
```

### run-multiple

Run multiple experiments sequentially.

```bash
python -m exp.runner run-multiple <app> "<exp1> <exp2> ..." [options]
```

**Arguments:**
- `<app>`: Application name (`hotel` or `synthetic`)
- `"<experiments>"`: Space-separated list of experiment names (must be quoted)

**Options:**
- `--plot`: Generate plots after each experiment
- `--no-cache`: Disable Docker cache during builds
- `--verbose, -v`: Enable verbose (DEBUG) logging

**Example:**
```bash
python -m exp.runner run-multiple hotel "exp1 exp2 exp3" --plot
```

### plot

Generate plots for an existing experiment.

```bash
python -m exp.runner plot <app> <experiment>
```

**Arguments:**
- `<app>`: Application name (`hotel` or `synthetic`)
- `<experiment>`: Experiment name to generate plots for

**Example:**
```bash
python -m exp.runner plot hotel exp1
```

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
python -m exp.runner run hotel exp1 --plot
```

### Key Differences

1. **Location**: Python runner can be called from anywhere in the repo (no need to `cd` to exp directory)
2. **Explicit app name**: Must specify app name as first argument (`hotel`, `synthetic`)
3. **Better error messages**: Clearer validation and error reporting
4. **Logging**: Use `--verbose` flag for detailed logs instead of shell tracing

## Adding New Applications

To add support for a new application:

1. Create a new plugin class in `exp/runner/apps/your_app.py`:

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

2. Register the plugin in `exp/runner/apps/__init__.py`:

```python
from .your_app import YourApp

def get_app_plugin(app_name: str) -> AppPlugin:
    apps = {
        "hotel": HotelApp,
        "synthetic": SyntheticApp,
        "your_app": YourApp,  # Add your app
    }
    # ...
```

3. Update CLI choices in `exp/runner/cli.py` to include your app name.

## Architecture

The runner is organized into several modules:

- **`cli.py`** - Command-line interface using argparse
- **`config.py`** - Configuration loading and validation
- **`experiment.py`** - Main orchestration logic
- **`docker_manager.py`** - Docker operations (build, run, stop, logs)
- **`apps/`** - Application plugins
  - `base.py` - Abstract base class for app plugins
  - `hotel.py` - Hotel application implementation
  - `synthetic.py` - Synthetic application implementation
- **`plotting/`** - Result visualization
  - `goodput.py` - Goodput plot generation
  - `latency.py` - Latency plot generation
  - `util.py` - Plotting utilities

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
python -m exp.runner --help
python -m exp.runner run --help

# Run a quick test experiment
python -m exp.runner run synthetic quick_test --verbose
```

### Logging
Set logging levels programmatically or use `--verbose` flag:
```python
import logging
logging.getLogger('exp.runner').setLevel(logging.DEBUG)
```

## License

Part of the MASA project. See repository root for license information.
