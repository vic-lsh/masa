#!/bin/bash
set -e

RUFF_VERSION="0.15.6"

# Parse arguments
CHECK_MODE=false
for arg in "$@"; do
  if [[ "$arg" == "--check" ]]; then
    CHECK_MODE=true
    break
  fi
done

if [ "$CHECK_MODE" = true ]; then
  echo "Checking Python formatting..."
  uvx --from "ruff==$RUFF_VERSION" ruff format --check exp_runner scripts
else
  echo "Formatting Python code..."
  uvx --from "ruff==$RUFF_VERSION" ruff format exp_runner scripts
fi
