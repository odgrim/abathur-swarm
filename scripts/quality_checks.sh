#!/usr/bin/env bash
# quality_checks.sh - Run code quality checks
#
# Usage: ./quality_checks.sh <task_id>
#
# This hook runs various code quality checks including linting, formatting,
# static analysis, and security scans based on the project type.

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

log_info "Running code quality checks for task: $TASK_ID"

# Track check results
CHECKS_PASSED=0
CHECKS_FAILED=0
CHECKS_WARNED=0
CHECKS_RUN=0

# Function to run a quality check
run_check() {
    local check_name="$1"
    local check_cmd="$2"
    local warn_only="${3:-false}"

    log_section "$check_name"
    ((CHECKS_RUN++))

    if eval "$check_cmd"; then
        log_info "✓ $check_name passed"
        ((CHECKS_PASSED++))
        return 0
    else
        if [[ "$warn_only" == "true" ]]; then
            log_warn "⚠ $check_name had warnings (non-fatal)"
            ((CHECKS_WARNED++))
            return 0
        else
            log_error "✗ $check_name failed"
            ((CHECKS_FAILED++))
            return 1
        fi
    fi
}

# Rust project checks
if [[ -f "Cargo.toml" ]]; then
    log_info "Detected Rust project"

    # Clippy (linter)
    if command -v cargo-clippy &> /dev/null || cargo clippy --version &> /dev/null; then
        run_check "Clippy (Rust Linter)" "cargo clippy --all-targets --all-features -- -D warnings" || true
    fi

    # Rustfmt (formatter)
    if command -v rustfmt &> /dev/null; then
        run_check "Rustfmt (Code Formatting)" "cargo fmt -- --check" || true
    fi

    # Cargo check (compilation)
    run_check "Cargo Check (Compilation)" "cargo check --all-targets --all-features" || true

    # Cargo audit (security)
    if command -v cargo-audit &> /dev/null; then
        run_check "Cargo Audit (Security)" "cargo audit" "true" || true
    fi

    # Check for unused dependencies
    if command -v cargo-udeps &> /dev/null; then
        run_check "Unused Dependencies" "cargo +nightly udeps" "true" || true
    fi
fi

# Node.js/JavaScript project checks
if [[ -f "package.json" ]]; then
    log_info "Detected Node.js project"

    # ESLint
    if grep -q 'eslint' package.json || [[ -f ".eslintrc.js" ]] || [[ -f ".eslintrc.json" ]]; then
        run_check "ESLint" "npx eslint ." || true
    fi

    # Prettier
    if grep -q 'prettier' package.json || [[ -f ".prettierrc" ]]; then
        run_check "Prettier (Code Formatting)" "npx prettier --check ." || true
    fi

    # TypeScript type checking
    if [[ -f "tsconfig.json" ]]; then
        run_check "TypeScript Type Checking" "npx tsc --noEmit" || true
    fi

    # npm audit (security)
    run_check "NPM Audit (Security)" "npm audit" "true" || true

    # Check for outdated dependencies
    run_check "Outdated Dependencies" "npm outdated" "true" || true
fi

# Python project checks
if [[ -f "setup.py" ]] || [[ -f "pyproject.toml" ]] || [[ -f "requirements.txt" ]]; then
    log_info "Detected Python project"

    # Pylint
    if command -v pylint &> /dev/null; then
        run_check "Pylint" "pylint **/*.py" "true" || true
    fi

    # Flake8
    if command -v flake8 &> /dev/null; then
        run_check "Flake8 (Linter)" "flake8 ." || true
    fi

    # Black (formatter)
    if command -v black &> /dev/null; then
        run_check "Black (Code Formatting)" "black --check ." || true
    fi

    # MyPy (type checking)
    if command -v mypy &> /dev/null; then
        run_check "MyPy (Type Checking)" "mypy ." "true" || true
    fi

    # Bandit (security)
    if command -v bandit &> /dev/null; then
        run_check "Bandit (Security)" "bandit -r ." "true" || true
    fi
fi

# Go project checks
if [[ -f "go.mod" ]]; then
    log_info "Detected Go project"

    # go vet
    run_check "Go Vet (Static Analysis)" "go vet ./..." || true

    # gofmt
    run_check "Go Fmt (Code Formatting)" "test -z \$(gofmt -l .)" || true

    # golint
    if command -v golint &> /dev/null; then
        run_check "Golint" "golint ./..." "true" || true
    fi

    # staticcheck
    if command -v staticcheck &> /dev/null; then
        run_check "Staticcheck" "staticcheck ./..." || true
    fi

    # gosec (security)
    if command -v gosec &> /dev/null; then
        run_check "Gosec (Security)" "gosec ./..." "true" || true
    fi
fi

# Universal checks (any project)

# Check for TODO/FIXME comments
log_section "Code Quality Metrics"
TODO_COUNT=$(grep -r "TODO\|FIXME" --include="*.rs" --include="*.js" --include="*.ts" --include="*.py" --include="*.go" . 2>/dev/null | wc -l | tr -d ' ')
log_info "TODO/FIXME comments: $TODO_COUNT"

# Check if any checks were run
if [[ $CHECKS_RUN -eq 0 ]]; then
    log_warn "No quality checks available for this project type"
    log_warn "Please install linters and formatters for your project"
    exit 0
fi

# Report results
log_section "Quality Check Results Summary"
log_info "Checks Run: $CHECKS_RUN"
log_info "Passed: $CHECKS_PASSED"
log_info "Failed: $CHECKS_FAILED"
log_info "Warnings: $CHECKS_WARNED"

# Store results in memory
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:quality"
    TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    QUALITY_RESULTS=$(jq -n \
        --arg passed "$CHECKS_PASSED" \
        --arg failed "$CHECKS_FAILED" \
        --arg warned "$CHECKS_WARNED" \
        --arg total "$CHECKS_RUN" \
        --arg todos "$TODO_COUNT" \
        --arg timestamp "$TIMESTAMP" \
        '{
            checks_run: ($total | tonumber),
            passed: ($passed | tonumber),
            failed: ($failed | tonumber),
            warned: ($warned | tonumber),
            success_rate: (($passed | tonumber) / ($total | tonumber) * 100 | floor),
            todo_count: ($todos | tonumber),
            timestamp: $timestamp
        }')

    abathur memory add \
        --namespace "$NAMESPACE" \
        --key "results" \
        --value "$QUALITY_RESULTS" \
        --type "semantic" \
        --created-by "technical_feature_workflow" 2>/dev/null || true

    log_info "✓ Quality check results stored in memory"
fi

# Exit with error if any critical checks failed
if [[ $CHECKS_FAILED -gt 0 ]]; then
    log_error "Some quality checks failed"
    exit 1
fi

log_info "✓ All quality checks passed"
exit 0
