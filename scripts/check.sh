#!/bin/bash

# Enforce no warnings
export RUSTFLAGS="-D warnings"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PYTHON_BIN="${PYTHON:-python3}"

"$PYTHON_BIN" "$REPO_ROOT/scripts/validate_policy_matrix.py"
"$PYTHON_BIN" "$REPO_ROOT/scripts/validate_tonic_masa_boundary.py"

flag_combos=(
    "sched_fifo"
    "sched_fifo,abort_slo"
    "sched_slo"
    "sched_slo,abort_slo"
    "sched_tailclipper,abort_slo"
    "sched_oracle"
    "sched_slo,trace_queue_latency"
    "sched_slo,ac_rajomon"
    "sched_slo,ac_pred,est_mean_var"
    "sched_pred,abort_slo,ac_pred,est_mean_var"
    "sched_pred,abort_slack,est_mean_var"
    "sched_pred,signal_slack,ac_pred,est_mean_var"
    "sched_mt"
    "sched_mt,abort_slo"
    "sched_mt,ac_rajomon"
    "sched_mt_multiqueue"
    "sched_mt_multiqueue,abort_slo"
)

CONTINUE_ON_ERROR=false
CHECK_TESTS=false
SPECIFIC_FLAGS=""
HAS_SPECIFIC_FLAGS=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --all)
            CONTINUE_ON_ERROR=true
            shift
            ;;
        --tests)
            CHECK_TESTS=true
            shift
            ;;
        *)
            if [ "$HAS_SPECIFIC_FLAGS" = true ]; then
                echo "Error: Specific flags already set to '$SPECIFIC_FLAGS', cannot set to '$1'"
                exit 1
            fi
            SPECIFIC_FLAGS="$1"
            HAS_SPECIFIC_FLAGS=true
            shift
            ;;
    esac
done

check_combo() {
    local flags="$1"
    echo "========================================================="
    if [ -z "$flags" ]; then
        echo "Checking without features"
        cargo check --quiet
    else
        echo "Checking flags '$flags'"
        cargo check --quiet --features "$flags"
    fi
    return $?
}

check_tests() {
    echo "========================================================="
    echo "Checking tests for Masa crates and apps"
    # List of packages to check tests for (excluding forked libs that fail strict checks)
    local packages=(
        "masa"
        "masa-core"
        "masa-policy"
        "masa-integration-tests"
        "hotel"
        "socialnet"
        "synthbench"
        "generic-service"
        "trace-config"
        "app-utils"
        "app-util-macros"
        "masa-benchmark"
    )

    local cmd="cargo check --tests --quiet"
    for pkg in "${packages[@]}"; do
        cmd="$cmd -p $pkg"
    done

    echo "Running: $cmd"
    $cmd || return $?

    echo "========================================================="
    echo "Checking featured masa-policy tests"
    for flags in "${flag_combos[@]}"; do
        case "$flags" in
            # Runtime-only flags (sched_mt*) live on tokio/masa, not masa-policy.
            *sched_mt*) continue ;;
            *ac_pred* | *ac_rajomon* | *abort_slack* | *signal_slack*) ;;
            *) continue ;;
        esac

        local featured_cmd="cargo check --tests --quiet -p masa-policy --features $flags"
        echo "Running: $featured_cmd"
        cargo check --tests --quiet -p masa-policy --features "$flags" || return $?
    done

    return 0
}

FAILED=false
FAILED_COMBOS=()

# 0. Check tests if requested
if [ "$CHECK_TESTS" = true ]; then
    check_tests
    status=$?
    if [ $status -ne 0 ]; then
        echo "Error: failed to check tests"
        if [ "$CONTINUE_ON_ERROR" = false ]; then
            exit $status
        fi
        FAILED=true
        FAILED_COMBOS+=("tests")
    fi

    if [ "$HAS_SPECIFIC_FLAGS" = false ]; then
        if [ "$FAILED" = true ]; then
            exit 1
        fi
        exit 0
    fi
fi

# If specific flags are provided, run only that check
if [ "$HAS_SPECIFIC_FLAGS" = true ]; then
    check_combo "$SPECIFIC_FLAGS"
    status=$?
    if [ $status -ne 0 ]; then
        echo "Error: failed to check with flags '$SPECIFIC_FLAGS'"
        exit $status
    fi
    # If we had failures in tests (and continued), exit 1
    if [ "$FAILED" = true ]; then
        exit 1
    fi
    exit 0
fi

# Otherwise, check all combinations

# 1. Check default (no features)
check_combo ""
status=$?
if [ $status -ne 0 ]; then
    echo "Error: failed to check without features"
    if [ "$CONTINUE_ON_ERROR" = false ]; then
        exit $status
    fi
    FAILED=true
    FAILED_COMBOS+=("default")
fi

# 2. Check feature flag combinations
for flags in "${flag_combos[@]}"; do
    check_combo "$flags"
    status=$?
    if [ $status -ne 0 ]; then
        echo "Error: failed to check with flags '$flags'"
        if [ "$CONTINUE_ON_ERROR" = false ]; then
            exit $status
        fi
        FAILED=true
        FAILED_COMBOS+=("$flags")
    fi
done

if [ "$FAILED" = true ]; then
    echo "========================================================="
    echo "The following checks failed:"
    for combo in "${FAILED_COMBOS[@]}"; do
        echo "  - $combo"
    done
    exit 1
fi

echo "========================================================="
echo "All checks passed."
exit 0
