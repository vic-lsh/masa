#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/hotel"
exp_name="ci"
out_dir="$exp_dir/out/$exp_name"
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

if [ ! -d "$exp_dir/in/$exp_name" ]; then
    echo "CI experiment config not found at $exp_dir/in/$exp_name" >&2
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

# Setup uv and virtual environment
if ! command -v uv >/dev/null 2>&1; then
    echo "Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
fi

# Only sync if not in a virtual environment (CI already does this)
if [ -z "${VIRTUAL_ENV:-}" ]; then
    echo "Syncing Python dependencies with uv..."
    cd "$repo_root"
    uv sync
    source .venv/bin/activate
else
    echo "Using existing virtual environment: $VIRTUAL_ENV"
    cd "$repo_root"
fi

echo "Cleaning previous experiment output at $out_dir"
rm -rf "$out_dir"

mapfile -t api_array < <(jq -r '.Apis[]' "$exp_dir/in/$exp_name/gen_config.json")

echo "APIs: ${api_array[@]}"


echo "Running hotel experiment: $exp_name"
cd "$repo_root"
python -m exp_runner.runner run hotel "$exp_name" $no_cache

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
policies=$(cat "$exp_dir/in/$exp_name/policies" | tr -d '\n')
read -ra policy_array <<< "$policies"

# Read RPS values from gen_config.json
mapfile -t rps_array < <(jq -r '.Rps[]' "$exp_dir/in/$exp_name/gen_config.json")

# Read APIs from gen_config.json
mapfile -t api_array < <(jq -r '.Apis[]' "$exp_dir/in/$exp_name/gen_config.json")

# Read Repeats from gen_config.json
repeats=$(jq -r '.Repeats' "$exp_dir/in/$exp_name/gen_config.json")

# Read DurationSecs from gen_config.json
duration=$(jq -r '.DurationSecs' "$exp_dir/in/$exp_name/gen_config.json")
num_apis=${#api_array[@]}

# Validate output files for each repeat, policy, RPS, and API
for i in $(seq 0 $((repeats - 1))); do
    for policy in "${policy_array[@]}"; do
        for rps in "${rps_array[@]}"; do
            for api in "${api_array[@]}"; do
                expected_file="$out_dir/$i/$policy/r${rps}_${api}.csv"
                echo "Checking: $expected_file"
                assert_path_exists "$expected_file"

                # Check goodput
                # Count rows where error (column 7) is /None
                goodput=$(awk -F, '$7 == "/None" {count++} END {print count+0}' "$expected_file")

                # Expected requests per API = (RPS * Duration) / NumApis
                # Note: This assumes uniform distribution across APIs which is how load_gen works.
                expected_total=$(( rps * duration ))
                expected_per_api=$(( expected_total / num_apis ))

                # Calculate observed RPS
                observed_rps=$(python -c "print(f'{ $goodput / $duration :.2f}')")
                expected_rps_per_api=$(python -c "print(f'{ $expected_per_api / $duration :.2f}')")
                echo "  Goodput: $goodput requests (Observed RPS: $observed_rps, Expected RPS: $expected_rps_per_api)"

                # Allow 20% margin
                lower=$(( expected_per_api * 8 / 10 ))
                upper=$(( expected_per_api * 12 / 10 ))

                if (( goodput < lower )) || (( goodput > upper )); then
                    echo "Error: Goodput mismatch in $expected_file. Expected ~$expected_per_api (+/- 20%), got $goodput" >&2
                    exit 1
                fi
            done
        done
    done
done

echo "Generating plots..."
cd "$repo_root"
python -m exp_runner.runner plot hotel "$exp_name"

echo "Hotel CI experiment test passed."
