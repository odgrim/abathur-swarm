---
name: pytest
description: Run Python tests using pytest with standardized options for unit, integration, performance, and benchmark tests
version: 1.0.0
---

# pytest Skill

This skill provides standardized pytest execution commands for running tests in Python projects. Use this skill whenever you need to run tests to ensure consistency across test runs.

## When to Use This Skill

- Running unit tests during development
- Running integration or E2E tests
- Running performance/benchmark tests
- Verifying code changes with test coverage
- Debugging test failures
- Running specific test files, directories, or test functions

## Test Categories

This project organizes tests into the following categories:

1. **Unit tests** (`tests/unit/`) - Fast, isolated component tests
2. **Integration tests** (`tests/integration/`) - Tests that verify component interactions
3. **E2E tests** (`tests/e2e/`) - End-to-end workflow tests
4. **Performance tests** (`tests/performance/`) - Tests marked with `@pytest.mark.performance`
5. **Benchmark tests** (`tests/benchmarks/`) - Tests marked with `@pytest.mark.benchmark`
6. **Manual tests** (`tests/manual/`) - Manual verification tests (not run in CI)

## Standard pytest Commands

### Run All Tests (Excluding Performance/Benchmarks)
```bash
pytest -n auto -m "not performance and not benchmark and not slow"
```

### Run All Tests Including Coverage
```bash
pytest -n auto
```
Note: Default configuration includes coverage reporting to terminal and HTML. The `-n auto` flag automatically detects available CPU cores for parallel execution.

### Run Unit Tests Only
```bash
pytest -n auto tests/unit/
```

### Run Integration Tests Only
```bash
pytest -n auto tests/integration/
```

### Run E2E Tests Only
```bash
pytest -n auto tests/e2e/
```

### Run Performance Tests
```bash
pytest -n auto -m performance
```

### Run Benchmark Tests
```bash
pytest -n auto -m benchmark
```

### Run a Specific Test File
```bash
pytest -n auto tests/unit/services/test_memory_service.py
```

### Run a Specific Test Function
```bash
pytest tests/unit/services/test_memory_service.py::test_function_name
```
Note: Single test functions don't benefit from parallelization, so `-n auto` is omitted here.

### Run a Specific Test Class
```bash
pytest tests/unit/services/test_memory_service.py::TestClassName
```

### Run Tests with Verbose Output
```bash
pytest -n auto -v tests/unit/
```

### Run Tests with Extra Verbose Output (Show All Test Details)
```bash
pytest -n auto -vv tests/unit/
```

### Run Tests and Stop on First Failure
```bash
pytest -n auto -x tests/unit/
```

### Run Tests and Show Local Variables on Failure
```bash
pytest -n auto -l tests/unit/
```

### Run Failed Tests from Last Run
```bash
pytest -n auto --lf
```

### Run Failed Tests First, Then Others
```bash
pytest -n auto --ff
```

### Run Tests with Minimal Traceback
```bash
pytest -n auto --tb=line tests/unit/
```

### Run Tests with No Traceback (Just Pass/Fail)
```bash
pytest -n auto --tb=no tests/unit/
```

### Run Tests with Full Traceback
```bash
pytest -n auto --tb=long tests/unit/
```

### Run Tests with Print Statements Visible
```bash
pytest -n auto -s tests/unit/
```

### Run Tests Matching a Keyword Expression
```bash
pytest -n auto -k "memory" tests/
```

### Run Tests Excluding Slow Tests
```bash
pytest -n auto -m "not slow"
```

## Common Combinations

### Quick Feedback Loop (Fast Tests, Stop on First Failure)
```bash
pytest -n auto tests/unit/ -x --tb=line
```

### Debugging a Specific Test
```bash
pytest tests/unit/test_file.py::test_name -vv -s -l
```
Note: Debugging specific tests doesn't use `-n auto` as it's more effective to run single tests serially.

### CI/CD Validation (All Tests with Coverage)
```bash
pytest -n auto -m "not performance and not benchmark and not slow"
```

### Performance Analysis
```bash
pytest -n auto -m performance -v
```

### Full Test Suite with Detailed Output
```bash
pytest -n auto -v --tb=short
```

## Project-Specific Configuration

This project has pytest configured in `pyproject.toml` with:

- **Test paths**: `tests/`
- **Async mode**: Auto (handles async tests automatically)
- **Default options**: `-ra -q --strict-markers --cov=abathur --cov-report=term-missing --cov-report=html`
- **Coverage source**: `src/`
- **Coverage reports**: Terminal with missing lines + HTML report in `htmlcov/`
- **Parallel execution**: pytest-xdist installed for parallel test execution with `-n auto`

## Markers

Available test markers:
- `@pytest.mark.performance` - Performance tests
- `@pytest.mark.benchmark` - Benchmark tests
- `@pytest.mark.slow` - Slow-running tests (>30 seconds)

## Best Practices for Agents

1. **Default to fast tests first**: Run unit tests before integration tests
2. **Use `-n auto` for speed**: Parallel execution automatically uses all CPU cores for faster test runs
3. **Use `-x --tb=line` for quick feedback**: Stop on first failure with minimal traceback
4. **Add `-v` for clarity**: Verbose output helps understand what's being tested
5. **Exclude slow tests during iteration**: Use `-m "not slow"` for faster feedback
6. **Run full suite before commits**: Ensure all tests pass with `pytest -n auto`
7. **Check coverage**: Review coverage reports to ensure adequate test coverage
8. **Use specific paths**: Run only relevant tests when working on specific features
9. **Debug with `-vv -s -l`**: Show all details, print statements, and local variables (omit `-n auto` when debugging)
10. **Serial execution when needed**: Omit `-n auto` for single tests, debugging, or tests requiring shared state

## Examples for Common Scenarios

### After Writing New Code
```bash
# Run relevant unit tests in parallel
pytest -n auto tests/unit/services/test_new_service.py -v
```

### Before Committing
```bash
# Run all tests except performance/benchmarks in parallel
pytest -n auto -m "not performance and not benchmark and not slow"
```

### Investigating a Failure
```bash
# Run specific failing test with full debugging (serial execution)
pytest tests/unit/test_file.py::test_failing_function -vvs -l --tb=long
```

### Checking Test Coverage
```bash
# Run with coverage in parallel and open HTML report
pytest -n auto --cov=abathur --cov-report=html && open htmlcov/index.html
```

### Running Tests in Background
```bash
# Run tests in background for long-running suites (parallel)
pytest -n auto tests/benchmarks/ -v --tb=line &
```

## Notes

- Always use `pytest` command (not `python -m pytest`) for consistency
- Tests automatically discover async fixtures with `asyncio_mode = "auto"`
- Coverage reports are generated by default; use `--no-cov` to disable
- HTML coverage reports are saved to `htmlcov/` directory
- Strict markers mode is enabled; use only registered markers
- **Parallel execution with `-n auto`**: Automatically detects CPU cores and runs tests in parallel for significant speed improvements
- **When to skip `-n auto`**: Single test functions, debugging sessions, or tests that require shared state/resources
