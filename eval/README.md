# Evaluation experiments

Each leaf directory under `eval/` is a semantically named experiment containing:

- `README.md`: purpose, authoritative configuration, and usage notes.
- `run.sh`: a thin wrapper around the shared experiment runner.

Directories may be nested to group related experiments. Experiment and runner
configuration names should match by default, remain concise, and describe the
experiment rather than its location in a paper.

The configuration under `exp/<app>/in/<experiment>/` is the source of truth.
Wrappers accept and forward normal runner flags such as `--k8s`, `--kind`,
`--plot`, `--dry-run`, and `--rm-data`.
