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
└── libs          # Masa libraries and modified libraries
```

Masa is implemented by modifying a few crates; these are contained in `libs`. To evaluate Masa, we have a few microservice applications in `apps`.


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
See `EXPERIMENT_WORKFLOW.md` for a detailed guide on running experiments.
Masa currently has four applications for experimentation:

- `hotel`: Based on the Hotel application in Deathstarbench. We've ported this application to Rust for Masa compatibility.
- `socialnet`: Social network microservice workload inspired by the SocialNetwork benchmark.
- `synthetic`: A synthetic application with configurable behavior, for understanding Masa in simple scenarios.
- `mssim`: A microservice simulator-driven workload for trace-based experiments.

Docker compose is the recommended way to run an application. See instructions in the section below.

#### Docker compose automated (single-server)

NOTE: This is currently only supported for `hotel` and `synthetic`.

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
uv run -m exp_runner plot synthetic quick_test
uv run -m exp_runner plot mssim e2e_test
uv run -m exp_runner plot socialnet exp1
```

The plots will be saved at `exp/<app>/data/plots/<experiment>`.

You can also pass a `--plot` option when running experiments to automatically generate plots after completion:

```bash
uv run python -m exp_runner run <app> <experiment-name> --plot
```

#### Docker compose manual (single-server)

NOTE: This is currently supported for `hotel`, `synthetic`, and `socialnet`.

For running experiments, use the Python experiment runner:

```bash
# Run a full experiment (builds, starts services, runs load generator, collects logs)
uv run -m exp_runner run <app> <experiment-name> --plot

# For example:
uv run -m exp_runner run hotel exp1 --plot
uv run -m exp_runner run synthetic quick_test --plot
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

#### K8s (work-in-progress)

WARNING: This section is outdated and the functionality is likely broken.

NOTE: This is currently only supported for `hotel`.
NOTE: Running with k8s hasn't been well-tested. Please report issues if you find any.

You should install k8s on your system before running scripts in this section. For local setups, [minikube](https://minikube.sigs.k8s.io/docs/) is recommeded.

For a one-click setup, run `apps/hotel/snippets/k8s/run_snippet.sh`.

To see how to run the K8s step by step, read this ![README](apps/hotel/scripts/k8s/README.md) file in the k8s folder.

## List of policies

Each key in the following list corresponds to a feature flag in the codebase.

- `fifo`: requests are served in first-in-first-out order.
- `prio_global`: requests are served based on their end-to-end SLO end time, which is their SLO added to the time at which they arrived at the frontend server.
- `prio_local`: requests are served based on their local deadline (talk to the project leads if you're interested in how this is calculated); currently this only works for the `hotel` application, as it requires a description of the call graph.
- `fifo_span_tracing`: Based on fifo flag while printing the trace spans when run experiement script
- `fifo_queue_tracing`: Based on fifo flag while recording the queue latency
- `prio_global_queue_tracing`: Based on prio_global flag while recording the queue latency
