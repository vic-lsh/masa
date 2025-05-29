# Masa

Masa is a new RPC system that improves RPC goodput via dynamic RPC prioritization.
RPCs typically operate with SLO constraints in their end-to-end, user-facing workflow (e.g., a social media feed refresh sets its SLO at 500ms, and all the RPCs in service of the refresh must complete within 500ms).
Based on this observation, Masa detects which RPCs are _running late_ at runtime, and dynamically adjusts RPC priority accordingly.

Masa is implemented based on [Tonic](https://github.com/hyperium/tonic), [Hyper](https://hyper.rs/), and [Tokio](https://tokio.rs/).

## Project organization

The codebase is structured as follows:

```
.
├── 3rd_party     # vendored in dependencies; not modified
├── apps          # microservice applications and testbeds       
└── libs          # Masa libraries (based on Tonic, Tokio, and Hyper)
```

Masa is implemented by modifying a few crates; these are contained in `libs`. To evaluate Masa, we have a few microservice applications in `apps`.

We have also included the source code of a few 3rd-party crates in `3rd_party`. These crates are included by source to make it simpler to link them against Masa's crates.

## Getting started

### Running an application

Currently, Masa has three applications for performing experiments under `apps/<app>`:
- `hotel`: Based on the Hotel application in Deathstarbench. We've ported this application to Rust for Masa compatibility.
- `socialnet`: TODO
- `synthetic`: A synthetic application with very simple behavior, for understanding Masa in the simplest scenario.

Docker compose is the recommended way to run an application. See instructions in the section below.

#### Docker compose (single-server)

NOTE: This is currently only supported for `hotel` and `synthetic`.

```bash
cd apps/<app>

# Build and start the services (and their databases) as docker containers.
# Specify the policy you want Masa to use
./scripts/docker-run.sh --features <policy>

# Start generating load to the application
# Note: load generation config is expected to be at ./scripts/gen_config.json
# To start, make a copy of ./scripts/gen_config.template.json.
./scripts/loadgen-run.sh

# To view logs from the containers (w/ tmux), run this script in another terminal.
./scripts/docker-view-logs.sh

# To view resource usage across containers, run this:
docker stats

# Teardown the docker services and their databases.
./scripts/docker-stop.sh
```

where `<policy>` is one of the policy feature flags. If you just want to get the application to run, use `fifo`.

#### Docker compose automated (single-server)

NOTE: This is currently only supported for `hotel` and `synthetic`.

We have some basic scripts to automate running experiments on an application. Assuming you are in `apps/<app>`, an experiment takes the following files as input:
```
./data/in/<experiment>
├── gen_config.json     # load gen config
├── policies            # list of policies to run the experiment with
└── config.docker.json  # optional: app config (if not provided, whatever config is already present in ./scripts/local/config.docker.json will be used)
```
See `./data/in/template` for an example experiment.

Execute the folowing command to run the experiment:
```bash
./scripts/run_experiment.sh "<experiment>"
```

For every policy, the script generates a folder with the following structure
```
./data/out/<experiment>/<policy>
├── client.log          # logs from load gen
├── r<rps1>.csv         # trace for each RPS level provided in the load gen config
├── ...                 
├── r<rpsN>.csv         
├── <service1>.log      # logs for each container
├── ...
└── <serviceM>.log 
```

To run multiple experiments sequentially, execute
```bash
./scripts/queue_experiments.sh "<experiment1> ... <experimentN>"
```

#### tmux-based workload run scripts (legacy)

NOTE: This is currently only supported for `hotel`.
NOTE: some scripts no longer work out-of-the-box. Please run with docker compose instead.

Running the Hotel application contains many configuration choices, including but not limited to:

1. How many user-facing workflows to run in parallel (single workflow? multiple?)
2. Server queueing policy (FIFO? Deadline-based?)
3. Workload generation (How many requests per second? Does this change over time?)

We have included scripts to simplify configuration. The scripts are in the `apps/hotel/snippets` folder. Snippets are grouped into sub-folders of workflow combinations. For example, `single/search` only runs search, whereas `search-reservation` generates requests for both workflows in parallel.

Within each workflow subfolder, you will find a `run_snippet.sh` file. This file lists typical configurations one may want to run through (e.g., first run FIFO policy, then run deadline-based.)

You will also find `gen_config.json` in each subfolder. This describes how the user workload is generated. The committed `gen_config.json` file incrementally builds up requests-per-second to increase load.

#### K8s (work-in-progress)

NOTE: This is currently only supported for `hotel`.
NOTE: Running with k8s hasn't been well-tested. Please report issues if you find any.

You should install k8s on your system before running scripts in this section. For local setups, [minikube](https://minikube.sigs.k8s.io/docs/) is recommeded.

For a one-click setup, run `apps/hotel/snippets/k8s/run_snippet.sh`.

To see how to run the K8s step by step, read this ![README](apps/hotel/scripts/k8s/README.md) file in the k8s folder.

## List of policies

Each key in the following list corresponds to a feature flag in the codebase.

- `fifo`: requests are served in first-in-first-out order.
- `prio_global`: requests are served based on their end-to-end SLO end time, which is their SLO added to the time at which they arrived at the frontend server.
- `prio_local`: requests are served based on their local deadline (talk to the project leads if you're interested in how this is calculated).
- `prio_local_direct`: similar to `prio_local`
- `prio_local_indirect`: similar to `prio_local` 
- `prio_global_early`: same as `prio_global`, but aborts requests if their deadline is past.
- `prio_local_early`: same as `prio_local`, but aborts requests if their deadline is past.
