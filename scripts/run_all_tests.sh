#!/usr/bin/env bash
# run_all_tests.sh - Run comprehensive test suite
#
# Usage: ./run_all_tests.sh <task_id>
#
# This hook runs all tests (unit, integration, etc.) for the project.
# It detects the project type and runs appropriate test commands.

set -euo pipefail

TASK_ID="${1:-}"

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

log_section() {
    echo -e "\n${BLUE}[====] $* [====]${NC}\n"
}

# Validate inputs
if [[ -z "$TASK_ID" ]]; then
    log_error "Task ID is required"
    exit 1
fi

log_info "Running comprehensive test suite for task: $TASK_ID"

# Track test results
TESTS_PASSED=0
TESTS_FAILED=0
TEST_COMMANDS_RUN=0

# Function to run a test command
run_test() {
    local test_name="$1"
    local test_cmd="$2"

    log_section "$test_name"
    ((TEST_COMMANDS_RUN++))

    if eval "$test_cmd"; then
        log_info "✓ $test_name passed"
        ((TESTS_PASSED++))
        return 0
    else
        log_error "✗ $test_name failed"
        ((TESTS_FAILED++))
        return 1
    fi
}

# Detect project type and run appropriate tests

# Rust project
if [[ -f "Cargo.toml" ]]; then
    log_info "Detected Rust project"

    # Run cargo test
    run_test "Rust Unit Tests" "cargo test --lib" || true

    # Run cargo test for all targets
    run_test "Rust All Tests" "cargo test --all-targets" || true

    # Run doc tests
    run_test "Rust Doc Tests" "cargo test --doc" || true

    # Run integration tests if they exist
    if [[ -d "tests" ]]; then
        run_test "Rust Integration Tests" "cargo test --test '*'" || true
    fi
fi

# Node.js/JavaScript project
if [[ -f "package.json" ]]; then
    log_info "Detected Node.js project"

    # Check if npm test is configured
    if grep -q '"test"' package.json; then
        run_test "NPM Tests" "npm test" || true
    fi

    # Check for jest
    if grep -q 'jest' package.json; then
        run_test "Jest Tests" "npx jest --coverage" || true
    fi

    # Check for vitest
    if grep -q 'vitest' package.json; then
        run_test "Vitest Tests" "npx vitest run" || true
    fi
fi

# Python project
if [[ -f "setup.py" ]] || [[ -f "pyproject.toml" ]] || [[ -f "requirements.txt" ]]; then
    log_info "Detected Python project"

    # Run pytest if available
    if command -v pytest &> /dev/null; then
        run_test "Pytest" "pytest -v" || true
    fi

    # Run unittest if no pytest
    if [[ $TEST_COMMANDS_RUN -eq 0 ]] && command -v python &> /dev/null; then
        run_test "Python Unittest" "python -m unittest discover" || true
    fi
fi

# Go project
if [[ -f "go.mod" ]]; then
    log_info "Detected Go project"

    run_test "Go Tests" "go test ./..." || true
    run_test "Go Tests with Coverage" "go test -cover ./..." || true
fi

# Check if any tests were run
if [[ $TEST_COMMANDS_RUN -eq 0 ]]; then
    log_warn "No tests detected for this project type"
    log_warn "Please configure tests for your project"
    exit 0
fi

# Report results
log_section "Test Results Summary"
log_info "Test Commands Run: $TEST_COMMANDS_RUN"
log_info "Passed: $TESTS_PASSED"
log_info "Failed: $TESTS_FAILED"

# Store results in memory
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:tests"
    TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    TEST_RESULTS=$(jq -n \
        --arg passed "$TESTS_PASSED" \
        --arg failed "$TESTS_FAILED" \
        --arg total "$TEST_COMMANDS_RUN" \
        --arg timestamp "$TIMESTAMP" \
        '{
            commands_run: ($total | tonumber),
            passed: ($passed | tonumber),
            failed: ($failed | tonumber),
            success_rate: (($passed | tonumber) / ($total | tonumber) * 100 | floor),
            timestamp: $timestamp
        }')

    abathur memory add \
        --namespace "$NAMESPACE" \
        --key "results" \
        --value "$TEST_RESULTS" \
        --type "semantic" \
        --created-by "technical_feature_workflow" 2>/dev/null || true

    log_info "✓ Test results stored in memory"
fi

# Exit with error if any tests failed
if [[ $TESTS_FAILED -gt 0 ]]; then
    log_error "Some tests failed"
    exit 1
fi

log_info "✓ All tests passed"
exit 0
