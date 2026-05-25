#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PYTHON_BIN="${PYTHON:-python3}"

"$PYTHON_BIN" "$REPO_ROOT/scripts/validate_policy_matrix.py"
"$PYTHON_BIN" "$REPO_ROOT/scripts/validate_tonic_masa_boundary.py"

# Parse arguments
PARALLEL_JOBS=1
FEATURE_FLAG=""
IS_MATRIX=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --parallel)
            PARALLEL_JOBS="$2"
            shift # past argument
            shift # past value
            ;;
        --feature)
            FEATURE_FLAG="$2"
            IS_MATRIX=true
            shift
            shift
            ;;
        *)
            # unknown option
            shift
            ;;
    esac
done

# Setup for parallel execution
if [ "$PARALLEL_JOBS" -gt 1 ]; then
    RESULTS_DIR=$(mktemp -d)
    trap "rm -rf $RESULTS_DIR" EXIT
    TEST_NAMES=()
fi

# Initialize a variable to track the overall test status
overall_status=0
failed_packages=()

execute_test() {
    local name="$1"
    shift
    local cmd=("$@")

    if [ "$PARALLEL_JOBS" -gt 1 ]; then
        # Limit concurrency
        while [ $(jobs -r | wc -l) -ge $PARALLEL_JOBS ]; do
            wait -n 2>/dev/null || sleep 0.1
        done

        local passed=0
        local failed=0
        for f in "$RESULTS_DIR"/*.status; do
            [ -f "$f" ] || continue
            read -r status < "$f"
            if [ "$status" -eq 0 ]; then
                passed=$((passed+1))
            else
                failed=$((failed+1))
            fi
        done
        echo "Starting $name (Passed: $passed, Failed: $failed, Queued: ${#TEST_NAMES[@]})..."

        (
            outfile="$RESULTS_DIR/$name.log"
            echo "======== Testing $name ========" > "$outfile"
            # execute command
            "${cmd[@]}" >> "$outfile" 2>&1
            echo $? > "$RESULTS_DIR/$name.status"
        ) &
        TEST_NAMES+=("$name")
    else
        echo "======== Testing $name ========"
        "${cmd[@]}"
        local status=$?
        if [ $status -ne 0 ]; then
            overall_status=1
            failed_packages+=("$name")
        fi
    fi
}

# Feature flag combinations to test (aligned with policy_matrix.toml).
feature_combos=(
    "sched_slo,trace_queue_latency"
    "sched_slo,abort_slo"
    "sched_slo,ac_rajomon"
    "sched_pred,abort_slo,ac_pred,est_mean_var"
)

# Per-combo test dispatch: runs the hotel sched_policy test for every combo,
# plus additional feature-specific tests where applicable.
run_feature_tests() {
    local feat="$1"
    execute_test "hotel (sched_policy $feat)" cargo test -p hotel --test sched_policy --features "$feat"
    case "$feat" in
        sched_slo,trace_queue_latency)
            execute_test "masa-integration-tests (sched_slo+trace_queue_latency)" \
                cargo test -p masa-integration-tests --features sched_slo,trace_queue_latency
            ;;
        sched_slo,abort_slo)
            execute_test "masa-integration-tests (sched_slo+abort_slo)" \
                cargo test -p masa-integration-tests --features sched_slo,abort_slo
            ;;
        sched_slo,ac_rajomon)
            execute_test "masa-integration-tests (ac_rajomon)" \
                cargo test -p masa-integration-tests --features sched_slo,ac_rajomon
            execute_test "masa-policy (sched_slo+ac_rajomon)" \
                cargo test -p masa-policy --features sched_slo,ac_rajomon
            ;;
        sched_pred,abort_slo,ac_pred,est_mean_var)
            execute_test "masa-policy (sched_pred+abort_slo+ac_pred+est_mean_var)" \
                cargo test -p masa-policy --features sched_pred,abort_slo,ac_pred,est_mean_var
            ;;
    esac
}

# Run scheduling policy test with no features
if [ "$IS_MATRIX" = false ] || [ -z "$FEATURE_FLAG" ]; then
    execute_test "hotel (sched_policy no-op)" cargo test -p hotel --test sched_policy
fi

# Run feature-specific tests
if [ "$IS_MATRIX" = false ]; then
    for feat in "${feature_combos[@]}"; do
        run_feature_tests "$feat"
    done
elif [[ " ${feature_combos[*]} " =~ " $FEATURE_FLAG " ]]; then
    run_feature_tests "$FEATURE_FLAG"
elif [ -n "$FEATURE_FLAG" ]; then
    echo "Unsupported cargo test feature combo: '$FEATURE_FLAG'"
    echo "Supported combos:"
    printf '  - %s\n' "${feature_combos[@]}"
    exit 1
fi

# because not all tests build right now, we only test the modules we know to build successfully.

packages=(
    "masa-core"
    "masa-integration-tests"
    "tonic"
    "tonic-build"
    "masa"
    "tonic-health"
    #"tonic-reflection"
    "tonic-types"
    "tonic-web"
    "hyper"
    # tokio's tests are flaky -- reenable when we start to modify them
    # "tokio"
    # "tokio-stream"
    # "tokio-util"
    # "tokio-io-timeout"
    # "tokio-openssl"
    # our evaluation apps test suite
    "hotel"
    "socialnet"
    "synthbench"

    # tracebench
    "generic-service"
    "trace-config"
)

# Loop through each package and run tests
if [ "$IS_MATRIX" = false ] || [ -z "$FEATURE_FLAG" ]; then
    for package in "${packages[@]}"; do
        case "$package" in
            tokio | tokio-util)
                execute_test "$package" cargo test -p "$package" --features full
                ;;
            tower)
                execute_test "$package" cargo test -p "$package" --all-features
                ;;
            *)
                execute_test "$package" cargo test -p "$package"
                ;;
        esac
    done

    execute_test "tokio (masa priority suite)" cargo test -p tokio --features full --test masa_priority
fi

# Collect results if parallel
if [ "$PARALLEL_JOBS" -gt 1 ]; then
    wait
    for name in "${TEST_NAMES[@]}"; do
        if [ -f "$RESULTS_DIR/$name.log" ]; then
            cat "$RESULTS_DIR/$name.log"
        fi

        if [ -f "$RESULTS_DIR/$name.status" ]; then
            st=$(cat "$RESULTS_DIR/$name.status")
            if [ "$st" -ne 0 ]; then
                overall_status=1
                failed_packages+=("$name")
            fi
        else
            echo "Error: Status file not found for $name"
            overall_status=1
            failed_packages+=("$name - system error")
        fi
    done
fi

if [ ${#failed_packages[@]} -ne 0 ]; then
    echo "The following packages failed their tests:"
    for package in "${failed_packages[@]}"; do
        echo "- $package"
    done
else
    echo "All packages passed their tests."
fi

exit $overall_status
