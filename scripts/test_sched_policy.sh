#!/bin/bash

# Test that our applications are instantiating the scheduling policies correctly.
#
# Previously, this script tested feature flag propagation.
# Now, it tests that the runtime can be dynamically configured with different policies.

set -e

echo "Testing hotel application's scheduling policy"
cd apps/hotel

echo "Running sched_policy tests..."
cargo test --test sched_policy