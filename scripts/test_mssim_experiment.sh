#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_root="$repo_root/exp/mssim/out"
experiment_name="e2e_test"
run_root="$output_root/$experiment_name"

policies=(fifo prio_global)
rps_value=200
iteration=0
run_id="run_0"
latency_file="root_latencies_${rps_value}rps.csv"

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the MSSIM experiment test." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to run the MSSIM experiment test." >&2
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

echo "Running MSSIM experiment using exp_runner.runner"
python -m exp_runner.runner run mssim "$experiment_name" --rm-data

assert_path_exists() {
    if [ -e "$1" ]; then
        echo "File exists: $1"
    else
        echo "Expected path missing: $1" >&2
        exit 1
    fi
}

gen_config="$repo_root/exp/mssim/in/$experiment_name/gen_config.json"
duration="$(python -c 'import json,sys; print(int(json.load(open(sys.argv[1]))["DurationSecs"]))' "$gen_config")"

for policy in "${policies[@]}"; do
    policy_run_dir="$run_root/$iteration/$policy/$run_id"
    echo "Validating output for policy '$policy' under $policy_run_dir"

    assert_path_exists "$policy_run_dir"
    assert_path_exists "$policy_run_dir/metadata.json"
    assert_path_exists "$policy_run_dir/$latency_file"

    # Check goodput
    # Count rows where is_err (column 3) is false
    goodput=$(awk -F, '$3 == "false" {count++} END {print count+0}' "$policy_run_dir/$latency_file")

    # Expected requests = RPS * Duration
    # MSSIM loadgen uses Poisson arrival process for the target RPS
    expected_total=$(( rps_value * duration ))

    # Calculate observed RPS
    observed_rps=$(python -c "print(f'{ $goodput / $duration :.2f}')")
    echo "  Goodput: $goodput requests (Observed RPS: $observed_rps, Expected RPS: $rps_value)"

    # Allow 20% margin
    lower=$(( expected_total * 8 / 10 ))
    upper=$(( expected_total * 12 / 10 ))

    if (( goodput < lower )) || (( goodput > upper )); then
        echo "Error: Goodput mismatch in $policy_run_dir/$latency_file. Expected ~$expected_total (+/- 20%), got $goodput" >&2
        exit 1
    fi
done

echo "MSSIM experiment smoke test passed."
