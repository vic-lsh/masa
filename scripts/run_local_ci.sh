#!/bin/bash
# Local CI Runner Script
# 
# This script executes the CI pipeline locally, including formatting, linting, 
# and various testing stages (unit, E2E, experiments).
#
# Usage:
#   ./scripts/run_local_ci.sh [FILTER]
#
# Arguments:
#   FILTER (Optional): A string to filter which steps to run. It can be:
#     - A Job ID (e.g., 'cargo_check', 'pytest', 'test_socialnet')
#     - A tag (e.g., 'quick' to run only fast checks)
#     - A partial step name (e.g., 'fifo' to run only the FIFO matrix variant)
#     - 'all' (default) to run the entire pipeline.
#
# Environment Variables:
#   RUN_KIND: Set to 'false' to skip Kubernetes (kind) based tests. Defaults to 'true'.
#
set -u

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO] $1${NC}"
}

log_warn() {
    echo -e "${YELLOW}[WARN] $1${NC}"
}

log_error() {
    echo -e "${RED}[ERROR] $1${NC}"
}

# Argument 1: Filter (Job ID, partial name, or tag)
FILTER="${1:-all}"

# Usage: run_step "Step Name" "Command" "Job ID" "Tags"
run_step() {
    local step_name="$1"
    local cmd="$2"
    local job_id="$3"
    local tags="${4:-}"
    
    local should_run=false

    if [ "$FILTER" == "all" ]; then
        should_run=true
    else
        # 1. Exact match on Job ID (e.g., "cargo_check" runs all matrix variants)
        if [ "$FILTER" == "$job_id" ]; then
            should_run=true
        # 2. Exact match on Tags (e.g., "quick")
        elif [[ " $tags " =~ " $FILTER " ]]; then
            should_run=true
        # 3. Partial match on Step Name (e.g., "fifo" to run only that variant)
        elif [[ "$step_name" == *"$FILTER"* ]]; then
            should_run=true
        fi
    fi

    if [ "$should_run" = false ]; then
        return 0
    fi

    log_info "Running step: $step_name (Job: $job_id)"
    log_info "Command: $cmd"
    
    if eval "$cmd"; then
        log_info "Step '$step_name' PASSED"
    else
        log_error "Step '$step_name' FAILED"
        exit 1
    fi
    echo "------------------------------------------------"
}

# Check prerequisites
check_prereqs() {
    local missing=0
    for cmd in cargo python3 docker; do
        if ! command -v $cmd &> /dev/null; then
            log_error "$cmd is not installed or not in PATH."
            missing=1
        fi
    done
    
    if ! command -v uv &> /dev/null; then
        log_warn "uv is not installed. Scripts might try to install it."
    fi

    if [ $missing -eq 1 ]; then
        exit 1
    fi
}

# Configuration
RUN_KIND=${RUN_KIND:-true}

log_info "Starting Local CI Run"
log_info "Filter: $FILTER"
log_info "Note: Kubernetes (kind) tests are enabled by default. Set RUN_KIND=false to disable them."
log_info "Available Job IDs: format_check, cargo_check, cargo_check_tests, pytest, lint_python, cargo_test, test_e2e_hotel, test_mssim_experiment, test_socialnet, test_synthetic_experiment"

check_prereqs

# Ensure scripts are executable
chmod +x scripts/*.sh

# STAGE: quick-checks

# 1. Format Check
run_step "Format Check" "./scripts/format.sh --check" "format_check" "quick"

# 2. Cargo Check (Matrix)
features_list=(
    ""
    "fifo"
    "prio_global"
    "prio_global,early"
    "prio_local,early"
    "prio_oldest,early"
)

for features in "${features_list[@]}"; do
    if [ -z "$features" ]; then
        feat_name="default"
    else
        feat_name="$features"
    fi
    # Use exact same Job ID for all matrix items
    run_step "Cargo Check ($feat_name)" "./scripts/check.sh \"$features\"" "cargo_check" "quick"
done

# 3. Cargo Check Tests
run_step "Cargo Check Tests" "./scripts/check.sh --tests \"\"" "cargo_check_tests" "quick"

# 4. Python Tests
run_step "Python Tests" "./scripts/test_python.sh" "pytest" "quick"

# 5. Python Lint
run_step "Python Lint" "./scripts/lint_python.sh" "lint_python" "quick"

# STAGE: test

# 6. Cargo Test
run_step "Cargo Test" "./scripts/test.sh --parallel 8" "cargo_test"

# 7. E2E Hotel
run_step "E2E Hotel (docker)" "./scripts/test_e2e_hotel.sh --deploy-mode docker" "test_e2e_hotel"

if [ "$RUN_KIND" = "true" ]; then
    run_step "E2E Hotel (kind)" "./scripts/test_e2e_hotel.sh --deploy-mode kind" "test_e2e_hotel"
fi

# 8. MSSIM Experiment
run_step "MSSIM Experiment (docker)" "./scripts/test_mssim_experiment.sh --deploy-mode docker" "test_mssim_experiment"

if [ "$RUN_KIND" = "true" ]; then
    run_step "MSSIM Experiment (kind)" "./scripts/test_mssim_experiment.sh --deploy-mode kind" "test_mssim_experiment"
fi

# 9. SocialNet
run_step "SocialNet" "./scripts/test_socialnet.sh" "test_socialnet"

# 10. Synthetic Experiment
run_step "Synthetic Experiment (docker)" "./scripts/test_synthetic_experiment.sh ci --deploy-mode docker" "test_synthetic_experiment"

if [ "$RUN_KIND" = "true" ]; then
    run_step "Synthetic Experiment (kind)" "./scripts/test_synthetic_experiment.sh ci --deploy-mode kind" "test_synthetic_experiment"
fi

log_info "Local CI Run Complete!"
