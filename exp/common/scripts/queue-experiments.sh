#!/bin/bash

set -e

if [[ -z "${1:-}" ]]; then
    echo "Usage: ./scripts/queue-experiments.sh \"<experiment1> ... <experimentN>\" [--plot]" >&2
    exit 1
fi

experiments="$1"
shift 1

plot_arg=""

while [[ $# -gt 0 ]]; do
    case $1 in
    --plot)
        plot_arg="--plot"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

for exp in $experiments; do
    echo "running experiment $exp"
    ./scripts/run-experiment.sh "$exp" $plot_arg
done
