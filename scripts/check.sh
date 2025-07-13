#!/bin/bash

flag_combos=(
    "fifo"
    "fifo_infra"
    "fifo_early"
    "prio_local"
    "prio_local_early"
    "prio_local_direct"
    "prio_local_indirect"
    "prio_global"
    "prio_global_early"
)

cargo check
for flags in "${flag_combos[@]}"; do
    cargo check --features $flags 
    status=$?
    if [ $status -ne 0 ]; then
        echo "Error: failed to check with flags '$flags'"
        exit $status
    fi
done
