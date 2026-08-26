# Masa

Masa is a new RPC system that improves RPC goodput via dynamic RPC prioritization.
RPCs typically operate with SLO constraints in their end-to-end, user-facing workflow (e.g., a social media feed refresh sets its SLO at 500ms, and all the RPCs in service of the refresh must complete within 500ms).
Based on this observation, Masa detects which RPCs are _running late_ at runtime, and dynamically adjusts RPC priority accordingly.

Masa is implemented based on [Tonic](https://github.com/hyperium/tonic), [Hyper](https://hyper.rs/), and [Tokio](https://tokio.rs/).

## Project organization

The codebase is structured as follows:

```
.
├── apps          # microservice applications and testbeds
├── exp_runner    # Python experiment orchestration and analysis
├── libs          # Masa libraries and modified libraries
└── tests         # cross-cutting repository invariant tests
```

Masa is implemented by modifying a few crates; these are contained in `libs`. To evaluate Masa, we have a few microservice applications in `apps`.
Repository-wide Python checks that span multiple subsystems live in `tests`; Python experiment runner tests live in `exp_runner/tests`.


## Getting started

### Python Environment Setup

This project uses [uv](https://github.com/astral-sh/uv) for Python dependency management. You'll need Python 3.10+ and `uv` installed to run experiments and generate plots.

#### Installing uv

Install `uv` using the official installer:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

Or using other methods as described in the [uv documentation](https://github.com/astral-sh/uv#installation).

#### Setting up the Python environment

After installing `uv`, sync the project dependencies:

```bash
uv sync
```

This will:
- Create a virtual environment (`.venv`) in the project root
- Install all Python dependencies from `pyproject.toml` and `uv.lock`

#### Using the Python environment

After running `uv sync`, you can use Python commands directly with `uv run`:

```bash
uv run -m exp_runner plot hotel exp1
```

The virtual environment includes all required dependencies (matplotlib, pandas, numpy, etc.) needed for running experiments and generating plots.

### Running an application

Application source lives under `apps/<app>`, and the experiment assets for each app live under `exp/<app>`.
See `docs/experiments/workflow.md` for a detailed guide on running experiments.
Masa currently has four applications for experimentation:

- `hotel`: Based on the Hotel application in Deathstarbench. We've ported this application to Rust for Masa compatibility.
- `socialnet`: Social network microservice workload inspired by the SocialNetwork benchmark.
- `synthbench`: A synthbench application with configurable behavior, for understanding Masa in simple scenarios.
- `tracebench`: A trace-driven RPC benchmark-driven workload for trace-based experiments.

Docker compose is the recommended way to run an application. See instructions in the section below.

#### Docker compose automated (single-server)

NOTE: This is currently only supported for `hotel` and `synthbench`.

We have some basic scripts to automate running experiments on an application. Assuming you are in `exp/<app>`, an experiment takes the following files as input:

```
./data/in/<experiment>
├── gen_config.json     # load gen config
├── policies            # list of policies to run the experiment with
├── config.docker.json  # optional: app config for non-hotel apps (falls back to ./scripts/local/config.docker.json)
└── hotel.json          # required: hotel app config copied to apps/<app>/scripts/local/hotel.json during experiment setup
```

See `./data/in/template` for an example experiment.

Execute the following command to run the experiment:

```bash
./scripts/run-experiment.sh "<experiment>"
```

The load gen configuration allows you to specify the number of times that the experiment should be repeated.
For the `i`-th repetition of the experiment, the script generates a folder with the following structure for every policy

```
./data/out/<experiment>/i/<policy>
├── loadgen.log         # logs from load gen
├── r<rps1>_<api1>.csv         # trace for each RPS level provided in the load gen config
├── ...                        # traces for different APIs are stored in different files
├── r<rps1>_<apiK>.csv
├── ...
├── r<rpsN>_<apiK>.csv
├── <service1>.log      # logs for each container
├── ...
└── <serviceM>.log
```

To run multiple experiments sequentially, execute

```bash
./scripts/queue-experiments.sh "<experiment1> ... <experimentN>"
```

##### Generating plots for an experiment

To generate plots for visualizing goodput and latency of an experiment, use the experiment runner:

```bash
# Generate plots for an existing experiment
uv run -m exp_runner plot <app> <experiment-name>

# For example:
uv run -m exp_runner plot hotel exp1
uv run -m exp_runner plot synthbench quick_test
uv run -m exp_runner plot tracebench e2e_test
uv run -m exp_runner plot socialnet exp1
```

The plots will be saved at `exp/<app>/data/plots/<experiment>`.

You can also pass a `--plot` option when running experiments to automatically generate plots after completion:

```bash
uv run python -m exp_runner run <app> <experiment-name> --plot
```

#### Docker compose manual (single-server)

NOTE: This is currently supported for `hotel`, `synthbench`, and `socialnet`.

For running experiments, use the Python experiment runner:

```bash
# Run a full experiment (builds, starts services, runs load generator, collects logs)
uv run -m exp_runner run <app> <experiment-name> --plot

# For example:
uv run -m exp_runner run hotel exp1 --plot
uv run -m exp_runner run synthbench quick_test --plot
uv run -m exp_runner run socialnet exp1 --plot

# Run multiple experiments sequentially
uv run -m exp_runner run-multiple hotel "exp1 exp2 exp3" --plot
```

See `exp_runner/runner/README.md` for full documentation on the experiment runner.

For manual Docker operations (without the full experiment workflow):

```bash
cd exp/<app>

# To view resource usage across containers:
docker stats
```

Note: The old bash scripts (`get-env.sh`, `docker-run.sh`, `loadgen-run.sh`, `docker-stop.sh`) have been replaced by the Python experiment runner.

#### K8s / Kind

NOTE: This is currently only supported for `synthbench`.

You can run experiments on Kubernetes (K8s) or Kind (Kubernetes in Docker) using the experiment runner.

For Kind (recommended for local development):
1. Ensure [Kind](https://kind.sigs.k8s.io/) is installed.
2. Run with the `--kind` flag:

```bash
uv run -m exp_runner run synthbench <experiment-name> --kind --plot
```

The `--kind` flag implies `--k8s` and handles loading images into the Kind cluster automatically.

For standard Kubernetes clusters:
1. Ensure `kubectl` is configured to point to your cluster.
2. Run with the `--k8s` flag:

```bash
uv run -m exp_runner run synthbench <experiment-name> --k8s --plot
```

Note: When using `--k8s` without `--kind`, you must ensure the container images are available to your cluster (e.g., pushed to a registry).

## List of policies

Each key in the following list corresponds to a feature flag in the codebase.
`policy_matrix.toml` is the canonical source for the supported policy flags,
their implications, and the check/test matrices. Run
`python3 scripts/validate_policy_matrix.py` to check local drift before changing
policy features.

- `sched_fifo`: requests are served in first-in-first-out order.
- `sched_slo`: requests are served based on their end-to-end SLO end time, which is their SLO added to the time at which they arrived at the frontend server.
- `sched_tailclipper`: oldest request first, implementing the TailClipper paper's policy with round-robin fairness.
- `sched_oracle`: synthbench-only perfect-information priority scheduling using configured remaining work headers.
- `sched_pred`: requests are served based on their local deadline with deadline tightening using latency estimates.
- `eval_oracle_continuation`: synthbench-only exact-continuation intervention on the normal `sched_pred` path.
- `eval_estimator_audit`: synthbench-only learned/reference telemetry and soft-priority magnitude ablation.
