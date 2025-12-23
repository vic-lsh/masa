#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/hotel"
exp_name="ci"
out_dir="$exp_dir/data/out/$exp_name"
no_cache=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        echo "Usage: $0 [--no-cache]"
        exit 1
        ;;
    esac
done

if [ ! -d "$exp_dir" ]; then
    echo "Hotel experiment directory not found at $exp_dir" >&2
    exit 1
fi

if [ ! -d "$exp_dir/data/in/$exp_name" ]; then
    echo "CI experiment config not found at $exp_dir/data/in/$exp_name" >&2
    exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the hotel experiment test." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to run the hotel experiment test." >&2
    exit 1
fi

echo "Cleaning previous experiment output at $out_dir"
rm -rf "$out_dir"

mapfile -t api_array < <(jq -r '.Apis[]' "$exp_dir/data/in/$exp_name/gen_config.json")

echo "APIs: ${api_array[@]}"


echo "Running hotel experiment: $exp_name"
cd "$exp_dir"
"$exp_dir/scripts/run-experiment.sh" "$exp_name" $no_cache

assert_path_exists() {
    if [ -e "$1" ]; then
        echo "File exists: $1"
    else
        echo "Expected path missing: $1" >&2
        exit 1
    fi
}

echo "Validating experiment output..."

# Check that the done marker exists
assert_path_exists "$out_dir/done"

# Read policies from the config
policies=$(cat "$exp_dir/data/in/$exp_name/policies" | tr -d '\n')
read -ra policy_array <<< "$policies"

# Read RPS values from gen_config.json
mapfile -t rps_array < <(jq -r '.Rps[]' "$exp_dir/data/in/$exp_name/gen_config.json")

# Read APIs from gen_config.json
mapfile -t api_array < <(jq -r '.Apis[]' "$exp_dir/data/in/$exp_name/gen_config.json")

# Read Repeats from gen_config.json
repeats=$(jq -r '.Repeats' "$exp_dir/data/in/$exp_name/gen_config.json")

# Validate output files for each repeat, policy, RPS, and API
for i in $(seq 0 $((repeats - 1))); do
    for policy in "${policy_array[@]}"; do
        for rps in "${rps_array[@]}"; do
            for api in "${api_array[@]}"; do
                expected_file="$out_dir/$i/$policy/r${rps}_${api}.csv"
                echo "Checking: $expected_file"
                assert_path_exists "$expected_file"
            done
        done
    done
done

echo "Generating plots..."
cd "$repo_root"
python3 -m exp.runner plot hotel "$exp_name"

echo "Hotel CI experiment test passed."
