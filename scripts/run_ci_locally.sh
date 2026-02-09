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

# Check for required core tools
for tool in cargo uv docker; do
    if ! command -v $tool &> /dev/null; then
        log_error "$tool is required but not installed."
        exit 1
    fi
done

if ! command -v protoc &> /dev/null; then
    log_error "protoc is required but not installed."
    log_info "You can install it using: ./scripts/install_protoc.sh"
    exit 1
fi

# Check for K8s tools
RUN_K8S=true
MISSING_K8S_TOOLS=()
for tool in kind kubectl helm; do
    if ! command -v $tool &> /dev/null; then
        MISSING_K8S_TOOLS+=("$tool")
        RUN_K8S=false
    fi
done

if [ "$RUN_K8S" = "false" ]; then
    log_warn "The following tools are missing: ${MISSING_K8S_TOOLS[*]}"
    log_warn "Kubernetes (Kind) tests will be SKIPPED."
    log_warn "Install them to run the full suite."
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
FEATURES_LIST=("" "fifo" "prio_global" "prio_global,early" "prio_local,early" "prio_oldest,early")

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

# 6. Cargo Tests
log_info "Running Cargo Tests..."
./scripts/test.sh --parallel 8
log_success "Cargo Tests Passed"

# 7. E2E Hotel Test
log_info "Running E2E Hotel Test..."
./scripts/test_e2e_hotel.sh
log_success "E2E Hotel Test Passed"

# 8. SocialNet Test
log_info "Running SocialNet Test..."
./scripts/test_socialnet.sh
log_success "SocialNet Test Passed"

# 9. Synthetic Experiment (Docker)
log_info "Running Synthetic Experiment (Docker)..."
./scripts/test_synthetic_experiment.sh ci --deploy-mode docker
log_success "Synthetic Experiment Passed"

# 10. MSSIM Experiment (Docker)
log_info "Running MSSIM Experiment (Docker)..."
./scripts/test_mssim_experiment.sh --deploy-mode docker
log_success "MSSIM Experiment Passed"

# 11. K8s Tests (Conditional)
if [ "$RUN_K8S" = "true" ]; then
    log_info "Running Synthetic Experiment (Kind)..."
    ./scripts/test_synthetic_experiment.sh ci --deploy-mode kind
    log_success "Synthetic Experiment (Kind) Passed"

    log_info "Running MSSIM Experiment (Kind)..."
    ./scripts/test_mssim_experiment.sh --deploy-mode kind
    log_success "MSSIM Experiment (Kind) Passed"
fi

echo -e "${GREEN}=======================================${NC}"
if [ "$RUN_K8S" = "true" ]; then
    echo -e "${GREEN}   ALL CI CHECKS PASSED LOCALLY!   ${NC}"
else
    echo -e "${GREEN}   ALL CHECKS PASSED LOCALLY!      ${NC}"
    echo -e "${YELLOW}   (Kind tests were skipped)       ${NC}"
fi
echo -e "${GREEN}=======================================${NC}"