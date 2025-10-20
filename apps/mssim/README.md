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
$ cd <this-directory>/mssim
$ cargo run --bin mssim -- --alibaba-trace <masa-project-root>/trace-analysis/golden/S_86516878 --config-dir ./example_config
```

Once the Docker compose cluster has started, you can view each service container's
resource usage via `docker stats`.

To monitor logs from the load generator, run `docker logs -f load_generator`.

## Running with replicas

Provide the optional `--config-dir` option to specify replica count for some services.

See `example_config/replicas.json` for an example on how to set replica count.

```bash
$ cargo run --bin mssim -- --alibaba-trace <masa-project-root>/trace-analysis/golden/S_86516878 --config-dir ./example_config
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
