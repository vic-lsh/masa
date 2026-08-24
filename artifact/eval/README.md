# Semantic evaluation declarations

This directory describes evaluation experiments by their scientific meaning.
Experiment IDs intentionally do not contain manuscript figure or section numbers,
because those identifiers change as the paper evolves.

The initial pilot declares a fixed-capacity, short sequential Tracebench workload:

```text
fixed_capacity/trace_short_sequential_branching
```

The semantic declaration is the reviewable description of the workload, offered
load, execution window, policies, hypothesis, and primary analysis. Its `runner`
block is a temporary adapter to the existing application-oriented experiment
directory. Validation compares the two representations and fails if the runner
input drifts.

## Commands

Validate the suite and all executable bindings:

```bash
uv run -m exp_runner reproduce validate artifact/eval/suite.yaml
```

Inspect the execution plan without building or running anything:

```bash
uv run -m exp_runner reproduce plan artifact/eval/suite.yaml
```

Run the pilot on Kubernetes:

```bash
uv run -m exp_runner reproduce run artifact/eval/suite.yaml \
  --select fixed_capacity/trace_short_sequential_branching \
  --k8s --plot
```

Use `--kind --dry-run` while validating deployment wiring. Kind is not an
appropriate environment for comparing performance numbers with the evaluation.
The runner's generic `--smoke-test` expects every offered request to complete
within the SLO, so it should not be used for saturation sweeps whose high-load
points deliberately exceed system capacity.

## Adding an experiment

1. Add reusable policy profiles under `policies/` when needed.
2. Add a semantic declaration under `experiments/<study>/`.
3. Register it under the appropriate research question in `suite.yaml`.
4. Run `reproduce validate` to detect schema or runner-config drift.

Per-experiment shell scripts are intentionally avoided. YAML declares what to
run; the shared Python runner owns validation and execution.
