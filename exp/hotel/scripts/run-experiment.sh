#!/bin/bash
#
# Python-based experiment runner wrapper for hotel application.
# This is a convenience wrapper that calls the Python experiment runner.
#
# Usage: ./run-experiment.sh <experiment> [--plot] [--no-cache]
#

set -e

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(git rev-parse --show-toplevel)

if [[ -z "${1:-}" ]]; then
    echo "Usage: $0 <experiment-name> [--plot] [--no-cache]" >&2
    echo "" >&2
    echo "This script wraps the Python experiment runner for convenience." >&2
    echo "For more options, use: python3 -m exp.runner run --help" >&2
    exit 1
fi

experiment=$1
shift 1

cd "$REPO_ROOT"
exec python3 -m exp.runner run hotel "$experiment" "$@"
