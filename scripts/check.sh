#!/bin/bash

flag_combos=(
    "fifo"
    "prio_local_direct"
    "prio_local_indirect"
    "prio_global"

    "fifo_span_tracing"
    "fifo_queue_tracing"
    "prio_global_queue_tracing"
)

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
