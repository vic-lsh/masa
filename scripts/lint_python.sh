#!/bin/bash
set -e

# Run ruff linter on the python code (exp_runner, tests, scripts)
# Using uvx to execute ruff as a tool without installing it in the project
echo "Running ruff check..."
uvx ruff check exp_runner scripts "$@"

echo "Checking python formatting..."
uvx ruff format --check exp_runner scripts
