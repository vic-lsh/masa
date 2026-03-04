#!/bin/bash

# Enforce no warnings
export RUSTFLAGS="-D warnings"

flag_combos=(
    "fifo"
    "prio_global"
    "prio_global,early"
    "prio_local,early"
    "prio_local,early,prio_local_transform"
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
        "masa-integration-tests"
        "hotel"
        "socialnet"
        "synthetic"
        "generic-service"
        "sim-config"
        "app-utils"
        "app-util-macros"
        "masa-benchmark"
    )

    local cmd="cargo check --tests --quiet"
    for pkg in "${packages[@]}"; do
        cmd="$cmd -p $pkg"
    done

    echo "Running: $cmd"
    $cmd
    return $?
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

    # If we only specified --tests and no specific flags, and no --all,
    # we might want to stop here?
    # The requirement is ambiguous, but typically flags are additive.
    # However, if checking tests is the only intention, running the full matrix is annoying.
    # Given the script structure, if I run `check.sh --tests`, it enters the "Otherwise" block below.
    # Let's assume that's desired behavior for a full "check".
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
