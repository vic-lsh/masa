#!/bin/bash
set -e

# Define colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}>>> $1${NC}"
}

log_success() {
    echo -e "${GREEN}SUCCESS: $1${NC}"
}

log_error() {
    echo -e "${RED}ERROR: $1${NC}"
}

log_warn() {
    echo -e "${YELLOW}WARNING: $1${NC}"
}

RUN_E2E=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --e2e)
            RUN_E2E=true
            shift
            ;;
        *)
            log_error "Unknown argument: $1"
            echo "Usage: $0 [--e2e]"
            exit 1
            ;;
    esac
done

# Check for required core tools
for tool in cargo uv; do
    if ! command -v $tool &> /dev/null; then
        log_error "$tool is required but not installed."
        exit 1
    fi
done

if [ "$RUN_E2E" = "true" ] && ! command -v docker &> /dev/null; then
    log_error "docker is required for --e2e but not installed."
    exit 1
fi

if ! command -v protoc &> /dev/null; then
    log_error "protoc is required but not installed."
    log_info "You can install it using: ./scripts/install_protoc.sh"
    exit 1
fi

# Check for K8s tools
RUN_K8S=true
MISSING_K8S_TOOLS=()
if [ "$RUN_E2E" = "true" ]; then
    for tool in kind kubectl helm; do
        if ! command -v $tool &> /dev/null; then
            MISSING_K8S_TOOLS+=("$tool")
            RUN_K8S=false
        fi
    done

    if [ "$RUN_K8S" = "false" ]; then
        log_warn "The following tools are missing: ${MISSING_K8S_TOOLS[*]}"
        log_warn "Kubernetes (Kind) tests will be SKIPPED."
        log_warn "Install them to run the full E2E suite."
    fi
fi

# Ensure we are in the project root
if [ ! -f "Cargo.toml" ]; then
    log_error "Please run this script from the project root."
    exit 1
fi

export CI="true"

# 1. Format Check
log_info "Running Format Check..."
./scripts/format.sh --check
log_success "Format Check Passed"

# 2. Python Lint
log_info "Running Python Lint..."
./scripts/lint_python.sh
log_success "Python Lint Passed"

# 3. Cargo Checks (Matrix)
log_info "Running Cargo Checks..."
FEATURES_LIST=(
    ""
    "sched_fifo"
    "sched_fifo,abort_slo"
    "sched_slo"
    "sched_slo,abort_slo"
    "sched_tailclipper,abort_slo"
    "sched_oracle"
    "sched_slo,ac_rajomon"
    "sched_slo,ac_pred,est_mean_var"
    "sched_pred,abort_slo,ac_pred,est_mean_var"
    "sched_pred,abort_slack,est_mean_var"
    "sched_pred,signal_slack,ac_pred,est_mean_var"
)

for features in "${FEATURES_LIST[@]}"; do
    echo "  Checking features: '$features'"
    ./scripts/check.sh "$features"
done
log_success "Cargo Checks Passed"

# 4. Cargo Check Tests
log_info "Running Cargo Check Tests..."
./scripts/check.sh --tests ""
log_success "Cargo Check Tests Passed"

# 5. Python Tests
log_info "Running Python Tests..."
./scripts/test_python.sh
log_success "Python Tests Passed"

# 6. Cargo Tests (Matrix)
log_info "Running Cargo Tests..."
TEST_FEATURES_LIST=(
    ""
    "sched_slo,trace_queue_latency"
    "sched_slo,abort_slo"
    "sched_slo,ac_rajomon"
    "sched_pred,abort_slo,ac_pred,est_mean_var"
)

for features in "${TEST_FEATURES_LIST[@]}"; do
    echo "  Testing features: '$features'"
    ./scripts/test.sh --feature "$features"
done
log_success "Cargo Tests Passed"

if [ "$RUN_E2E" = "true" ]; then
    log_info "Running E2E Hotel Test (Docker)..."
    ./scripts/test_e2e_hotel.sh --deploy-mode docker
    log_success "E2E Hotel Test Passed"

    log_info "Running SocialNet Test (Docker)..."
    ./scripts/test_socialnet.sh --deploy-mode docker
    log_success "SocialNet Test Passed"

    log_info "Running Synthbench Experiment (Docker)..."
    ./scripts/test_synthbench_experiment.sh ci --deploy-mode docker
    log_success "Synthbench Experiment Passed"

    log_info "Running Tracebench Experiment (Docker)..."
    ./scripts/test_tracebench_experiment.sh --deploy-mode docker
    log_success "Tracebench Experiment Passed"

    if [ "$RUN_K8S" = "true" ]; then
        for app in hotel socialnet synthbench tracebench; do
            log_info "Running $app E2E (Kind)..."
            case "$app" in
                hotel)
                    ./scripts/test_e2e_hotel.sh --deploy-mode kind
                    ;;
                socialnet)
                    ./scripts/test_socialnet.sh --deploy-mode kind
                    ;;
                synthbench)
                    ./scripts/test_synthbench_experiment.sh ci --deploy-mode kind
                    ;;
                tracebench)
                    ./scripts/test_tracebench_experiment.sh --deploy-mode kind
                    ;;
            esac
        done
    fi
fi

echo -e "${GREEN}=======================================${NC}"
if [ "$RUN_E2E" = "true" ] && [ "$RUN_K8S" = "true" ]; then
    echo -e "${GREEN}   ALL CI CHECKS PASSED LOCALLY!   ${NC}"
elif [ "$RUN_E2E" = "true" ]; then
    echo -e "${GREEN}   ALL CHECKS PASSED LOCALLY!      ${NC}"
    echo -e "${YELLOW}   (Kind tests were skipped)       ${NC}"
else
    echo -e "${GREEN}   ALL PR CHECKS PASSED LOCALLY!   ${NC}"
    echo -e "${YELLOW}   (E2E tests were skipped)        ${NC}"
fi
echo -e "${GREEN}=======================================${NC}"
