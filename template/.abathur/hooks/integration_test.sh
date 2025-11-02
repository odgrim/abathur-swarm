#!/usr/bin/env bash
# Integration Test Hook
#
# Runs integration tests when a feature branch completes all its tasks.
# This ensures that the combined work from multiple task branches integrates correctly.
#
# Usage: integration_test.sh <feature_branch_name>

set -euo pipefail

FEATURE_BRANCH="${1:-}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $*"
}

# Validate inputs
if [[ -z "$FEATURE_BRANCH" ]]; then
    log_error "Feature branch name is required"
    exit 1
fi

log_info "Running integration tests for feature branch: $FEATURE_BRANCH"

# Step 1: Verify we're in a git repository
log_step "1/5 Verifying git repository"
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    log_error "Not in a git repository"
    exit 1
fi
log_info "Git repository verified"

# Step 2: Check if feature branch exists
log_step "2/5 Checking if feature branch exists"
if ! git rev-parse --verify "$FEATURE_BRANCH" > /dev/null 2>&1; then
    log_error "Feature branch '$FEATURE_BRANCH' does not exist"
    exit 1
fi
log_info "Feature branch exists"

# Step 3: Checkout feature branch
log_step "3/5 Checking out feature branch"
CURRENT_BRANCH=$(git branch --show-current)
log_info "Current branch: $CURRENT_BRANCH"

if [[ "$CURRENT_BRANCH" != "$FEATURE_BRANCH" ]]; then
    log_warn "Switching to feature branch $FEATURE_BRANCH"
    git checkout "$FEATURE_BRANCH"
fi

# Step 4: Run cargo build
log_step "4/5 Building project"
if ! cargo build --all-features 2>&1 | tee /tmp/build_output.log; then
    log_error "Build failed"
    cat /tmp/build_output.log
    exit 1
fi
log_info "Build successful"

# Step 5: Run integration tests
log_step "5/5 Running integration tests"
if ! cargo test --test '*' --all-features 2>&1 | tee /tmp/test_output.log; then
    log_error "Integration tests failed"
    cat /tmp/test_output.log
    exit 1
fi
log_info "Integration tests passed"

log_info "✓ All integration tests passed for feature branch: $FEATURE_BRANCH"
exit 0
