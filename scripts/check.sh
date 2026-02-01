#!/bin/bash

# Simplified check script for compositional policy era.
# All policy logic is now compiled in by default, so we just need to check the workspace.

set -e

echo "========================================================="
echo "Checking workspace..."
cargo check --workspace