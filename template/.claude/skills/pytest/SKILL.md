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
pytest -m "not performance and not benchmark and not slow"
```

### Run All Tests Including Coverage
```bash
pytest
```
Note: Default configuration includes coverage reporting to terminal and HTML.

### Run Unit Tests Only
```bash
pytest tests/unit/
```

### Run Integration Tests Only
```bash
pytest tests/integration/
```

### Run E2E Tests Only
```bash
pytest tests/e2e/
```

### Run Performance Tests
```bash
pytest -m performance
```

### Run Benchmark Tests
```bash
pytest -m benchmark
```

### Run a Specific Test File
```bash
pytest tests/unit/services/test_memory_service.py
```

### Run a Specific Test Function
```bash
pytest tests/unit/services/test_memory_service.py::test_function_name
```

### Run a Specific Test Class
```bash
pytest tests/unit/services/test_memory_service.py::TestClassName
```

### Run Tests with Verbose Output
```bash
pytest -v tests/unit/
```

### Run Tests with Extra Verbose Output (Show All Test Details)
```bash
pytest -vv tests/unit/
```

### Run Tests and Stop on First Failure
```bash
pytest -x tests/unit/
```

### Run Tests and Show Local Variables on Failure
```bash
pytest -l tests/unit/
```

### Run Failed Tests from Last Run
```bash
pytest --lf
```

### Run Failed Tests First, Then Others
```bash
pytest --ff
```

### Run Tests with Minimal Traceback
```bash
pytest --tb=line tests/unit/
```

### Run Tests with No Traceback (Just Pass/Fail)
```bash
pytest --tb=no tests/unit/
```

### Run Tests with Full Traceback
```bash
pytest --tb=long tests/unit/
```

### Run Tests with Print Statements Visible
```bash
pytest -s tests/unit/
```

### Run Tests Matching a Keyword Expression
```bash
pytest -k "memory" tests/
```

### Run Tests Excluding Slow Tests
```bash
pytest -m "not slow"
```

## Common Combinations

### Quick Feedback Loop (Fast Tests, Stop on First Failure)
```bash
pytest tests/unit/ -x --tb=line
```

### Debugging a Specific Test
```bash
pytest tests/unit/test_file.py::test_name -vv -s -l
```

### CI/CD Validation (All Tests with Coverage)
```bash
pytest -m "not performance and not benchmark and not slow"
```

### Performance Analysis
```bash
pytest -m performance -v
```

### Full Test Suite with Detailed Output
```bash
pytest -v --tb=short
```

## Project-Specific Configuration

This project has pytest configured in `pyproject.toml` with:

- **Test paths**: `tests/`
- **Async mode**: Auto (handles async tests automatically)
- **Default options**: `-ra -q --strict-markers --cov=abathur --cov-report=term-missing --cov-report=html`
- **Coverage source**: `src/`
- **Coverage reports**: Terminal with missing lines + HTML report in `htmlcov/`

## Markers

Available test markers:
- `@pytest.mark.performance` - Performance tests
- `@pytest.mark.benchmark` - Benchmark tests
- `@pytest.mark.slow` - Slow-running tests (>30 seconds)

## Best Practices for Agents

1. **Default to fast tests first**: Run unit tests before integration tests
2. **Use `-x --tb=line` for quick feedback**: Stop on first failure with minimal traceback
3. **Add `-v` for clarity**: Verbose output helps understand what's being tested
4. **Exclude slow tests during iteration**: Use `-m "not slow"` for faster feedback
5. **Run full suite before commits**: Ensure all tests pass with `pytest`
6. **Check coverage**: Review coverage reports to ensure adequate test coverage
7. **Use specific paths**: Run only relevant tests when working on specific features
8. **Debug with `-vv -s -l`**: Show all details, print statements, and local variables

## Examples for Common Scenarios

### After Writing New Code
```bash
# Run relevant unit tests
pytest tests/unit/services/test_new_service.py -v
```

### Before Committing
```bash
# Run all tests except performance/benchmarks
pytest -m "not performance and not benchmark and not slow"
```

### Investigating a Failure
```bash
# Run specific failing test with full debugging
pytest tests/unit/test_file.py::test_failing_function -vvs -l --tb=long
```

### Checking Test Coverage
```bash
# Run with coverage and open HTML report
pytest --cov=abathur --cov-report=html && open htmlcov/index.html
```

### Running Tests in Background
```bash
# Run tests in background for long-running suites
pytest tests/benchmarks/ -v --tb=line &
```

## Notes

- Always use `pytest` command (not `python -m pytest`) for consistency
- Tests automatically discover async fixtures with `asyncio_mode = "auto"`
- Coverage reports are generated by default; use `--no-cov` to disable
- HTML coverage reports are saved to `htmlcov/` directory
- Strict markers mode is enabled; use only registered markers
