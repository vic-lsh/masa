#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_path="$repo_root/exp/mssim/config/test_config.json"
output_root="$repo_root/exp/mssim/data/experiments"
experiment_name="e2e_test"
run_root="$output_root/$experiment_name"

policies=(fifo prio_global)
rps_value=200
run_id="run_0"
latency_file="root_latencies_${rps_value}rps.csv"

if [ ! -f "$config_path" ]; then
    echo "MSSIM config not found at $config_path" >&2
    exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the MSSIM experiment test." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to run the MSSIM experiment test." >&2
    exit 1
fi

echo "Cleaning previous experiment output at $run_root"
rm -rf "$run_root"

echo "Running MSSIM experiment using $config_path"
python3 "$repo_root/exp/mssim/scripts/experiment.py" --config "$config_path"

assert_path_exists() {
    if [ ! -e "$1" ]; then
        echo "Expected path missing: $1" >&2
        exit 1
    fi
}

for policy in "${policies[@]}"; do
    policy_run_dir="$run_root/$policy/rps_${rps_value}/$run_id"
    echo "Validating output for policy '$policy' under $policy_run_dir"

    assert_path_exists "$policy_run_dir"
    assert_path_exists "$policy_run_dir/metadata.json"
    assert_path_exists "$policy_run_dir/$latency_file"
done

echo "MSSIM experiment smoke test passed."
