# Masa

Masa is a new RPC system that improves RPC goodput via dynamic RPC prioritization.
RPCs typically operate with SLO constraints in their end-to-end, user-facing workflow (e.g., a social media feed refresh sets its SLO at 500ms, and all the RPCs in service of the refresh must complete within 500ms).
Based on this observation, Masa detects which RPCs are _running late_ in runtime, and dynamically adjusts RPC priority accordingly.

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

### Running the Hotel application

Currently, Masa experiments based on the Hotel application in DeathStarBench. We've ported this application to Rust for Masa compatibility. The port is in `./apps/hotel`.

We include tooling to run the Hotel application in the following ways:

#### Docker compose (single-server)

```bash
cd apps/hotel
./scripts/docker-build.sh         # build each svc as a rust binary; place binaries in docker containers.
./scripts/docker-start.sh         # start all services and their databases via docker compose
./scripts/docker-stop.sh          # stop all containers in docker compose
```

#### tmux-based workload run scripts

Running the Hotel application contains many configuration choices, including but not limited to:

1. How many user-facing workflows to run in parallel (single workflow? multiple?)
2. Server queueing policy (FIFO? Deadline-based?)
3. Workload generation (How many requests per second? Does this change over time?)

We have included scripts to simplify configuration. The scripts are in the `apps/hotel/snippets` folder. Snippets are grouped into sub-folders of workflow combinations. For example, `single/search` only runs search, whereas `search-reservation` generates requests for both workflows in parallel.

Within each workflow subfolder, you will find a `run_snippet.sh` file. This file lists typical configurations one may want to run through (e.g., first run FIFO policy, then run deadline-based.)

You will also find `gen_config.json` in each subfolder. This describes how the user workload is generated. The committed `gen_config.json` file incrementally builds up requests-per-second to increase load.

#### K8s

You should install k8s on your system before running scripts in this section. For local setups, [minikube](https://minikube.sigs.k8s.io/docs/) is recommeded.

For a one-click setup, run `apps/hotel/snippets/k8s/run_snippet.sh`.

To see how to run the K8s step by step, read this ![README](apps/hotel/scripts/k8s/README.md) file in the k8s folder.
