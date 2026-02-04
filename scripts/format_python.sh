#!/bin/bash
set -e

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
  uvx ruff format --check exp_runner scripts
else
  echo "Formatting Python code..."
  uvx ruff format exp_runner scripts
fi
