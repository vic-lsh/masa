#!/bin/bash
set -euo pipefail

# Run ruff on the Python code (exp_runner, tests, scripts).
# Using uvx to execute ruff as a tool without installing it in the project.
# When --fix is supplied, also run formatter in write mode.
FIX_MODE=false
for arg in "$@"; do
  if [[ "$arg" == "--fix" ]]; then
    FIX_MODE=true
    break
  fi
done

echo "Running ruff check..."
uvx ruff check exp_runner scripts "$@"

if [ "$FIX_MODE" = true ]; then
  echo "Formatting python code..."
  uvx ruff format exp_runner scripts
else
  echo "Checking python formatting..."
  uvx ruff format --check exp_runner scripts
fi
