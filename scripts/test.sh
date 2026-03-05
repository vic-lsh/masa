#!/bin/bash

# Parse arguments
PARALLEL_JOBS=1

while [[ $# -gt 0 ]]; do
    case $1 in
        --parallel)
            PARALLEL_JOBS="$2"
            shift # past argument
            shift # past value
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
        local total=$(( ${#packages[@]} + 2 + 1 + ${#policy_flags[@]} ))
        echo "Starting $name (Passed: $passed, Failed: $failed, Total: $total)..."

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

# First, run tests from scripts for specialized purposes.
# We keep this sequential as it might set up things or be independent.
# ./scripts/test_sched_policy.sh

policy_flags=(
    "fifo"
    "prio_global"
    "prio_oldest"
    "prio_local"
    "prio_local,est_rms"
    "prio_local,est_hist"
    "prio_local,est_mean_var"
)

# Run scheduling policy tests
execute_test "hotel (sched_policy no-op)" cargo test -p hotel --test sched_policy
for policy in "${policy_flags[@]}"; do
    execute_test "hotel (sched_policy $policy)" cargo test -p hotel --test sched_policy --features "$policy"
done

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
    "tower"

    # our evaluation apps test suite
    "hotel"
    "socialnet"
    "synthetic"

    # simulator
    "generic-service"
    "sim-config"
)

# If testing your crate requires special feature flags, set them here
declare -A package_features=(
    ["tokio"]="--features full"
    ["tokio-util"]="--features full"
    ["tower"]="--all-features"
)

# Testing by package name can be ambiguous (e.g., we have a local crate X and
# cargo also downloads another version from crates.io). In this case, we can
# be precise about our package under test by specifying its Cargo.toml path.
declare -A package_manifest_paths=(
    # This is an example; async-task has been removed
    #["async-task"]="./libs/async-task/Cargo.toml"
)

# Loop through each package and run tests
for package in "${packages[@]}"; do
    # Check if the package has defined feature flags
    if [ -n "${package_features[$package]}" ]; then
        features="${package_features[$package]}"
    else
        features=""
    fi

    if [ -n "${package_manifest_paths[$package]}" ]; then
        # for packages with manifest path, test directly using manifest path
        execute_test "$package" cargo test --manifest-path "${package_manifest_paths[$package]}"
    else
        # otherwise, test with package name and optionally with feature flags
        # Need to be careful with word splitting for features if it contains multiple flags
        # but here it is passed as a string to cargo test...
        # If features is empty, we shouldn't pass an empty arg if it causes issues,
        # but cargo test -p pkg "" might be weird.
        # Let's construct the command array properly.
        cmd=("cargo" "test" "-p" "$package")
        if [ -n "$features" ]; then
             # split features string into args if needed, or just pass as is?
             # existing script did: cargo test -p "$package" $features
             # allowing shell expansion on $features.
             execute_test "$package" cargo test -p "$package" $features
        else
             execute_test "$package" cargo test -p "$package"
        fi
    fi
done

execute_test "tonic (prio_local,est_rms)" cargo test -p tonic --features "masa,prio_local,est_rms"
execute_test "tonic (prio_local,est_hist)" cargo test -p tonic --features "masa,prio_local,est_hist"
execute_test "tonic (prio_local,est_mean_var)" cargo test -p tonic --features "masa,prio_local,est_mean_var"
execute_test "tokio (masa priority suite)" cargo test -p tokio --features full --test masa_priority
execute_test "masa-integration-tests (prio_global)" cargo test -p masa-integration-tests --features prio_global
execute_test "masa-integration-tests (prio_global+trace-queue)" cargo test -p masa-integration-tests --features "prio_global,trace-queue"
execute_test "masa-integration-tests (prio_global+early)" cargo test -p masa-integration-tests --features "prio_global,early"

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
