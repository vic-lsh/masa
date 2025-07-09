#!/bin/bash

# Test that our applications are instantiating the scheduling policies correctly,
# based on the provided policy flags.
#
# It is important to test this on a per-application basis. This is because the
# application needs to propagate its policy feature flags to Masa libraries
# (e.g., tokio). Testing on a per-application basis tests this flag propagation.

set -e

policy_flags=(
    "fifo"
    "prio_global"
    "prio_local"
    "prio_global_early"
    "prio_local_early"
)

# Todo: add other applications

echo "Testing hotel application's scheduling policy"
cd apps/hotel

echo "Testing no-op"
cargo test --test sched_policy
for policy in "${policy_flags[@]}"; do
    echo "Testing policy $policy"
    cargo test --test sched_policy --features $policy
done
