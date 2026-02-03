#!/bin/bash

flag_combos=(
    "fifo"
    "prio_global"
    "prio_global,early"
    "prio_local,early"
)

# If a flag combo is provided as an argument, only check that one
# Empty string means check without features
if [ $# -gt 0 ]; then
    flags="$1"
    echo "========================================================="
    if [ -z "$flags" ]; then
        echo "Checking without features"
        cargo check --quiet
    else
        echo "Checking flags '$flags'"
        cargo check --quiet --features $flags
    fi
    status=$?
    if [ $status -ne 0 ]; then
        echo "Error: failed to check with flags '$flags'"
        exit $status
    fi
else
    # Otherwise, check all combos (for local use)
    cargo check
    for flags in "${flag_combos[@]}"; do
        echo "========================================================="
        echo "Checking flags '$flags'"
        cargo check --quiet --features $flags
        status=$?
        if [ $status -ne 0 ]; then
            echo "Error: failed to check with flags '$flags'"
            exit $status
        fi
    done
fi
