# Worktree Testing Guide

## Critical Infrastructure Issue

Git worktrees in `.abathur/worktrees/` have a Python import path issue that prevents tests from loading the worktree's code. By default, `poetry run pytest` loads code from the **main repository**, NOT the worktree.

## Problem Description

When you run tests in a worktree:

```bash
cd .abathur/worktrees/my-worktree
poetry run pytest tests/ -v
```

Python imports load from `/path/to/main/repo/src/`, not `/path/to/worktree/src/`. This causes all tests to fail even if the implementation is correct in the worktree.

### Evidence

```bash
# In worktree
$ pwd
/Users/odgrim/dev/home/agentics/abathur/.abathur/worktrees/my-worktree

$ poetry run python -c "import abathur.cli.main; import inspect; print(inspect.getfile(abathur.cli.main))"
/Users/odgrim/dev/home/agentics/abathur/src/abathur/cli/main.py  # ← Wrong! Main repo, not worktree
```

## Solution: PYTHONPATH Override

Use this command pattern for ALL worktree testing:

```bash
PYTHONPATH=/path/to/worktree/src:$PYTHONPATH poetry run pytest tests/ -v
```

### Example

```bash
cd /Users/odgrim/dev/home/agentics/abathur/.abathur/worktrees/cli-unit-tests

# WRONG (loads from main repo):
poetry run pytest tests/cli/test_cli_exclude_status.py -v

# CORRECT (loads from worktree):
PYTHONPATH=/Users/odgrim/dev/home/agentics/abathur/.abathur/worktrees/cli-unit-tests/src:$PYTHONPATH \
    poetry run pytest tests/cli/test_cli_exclude_status.py -v
```

## Verification Steps

### 1. Verify Python Loads from Worktree

Before running tests, verify the import path:

```bash
PYTHONPATH=/path/to/worktree/src:$PYTHONPATH \
    poetry run python -c "import abathur.cli.main; import inspect; print(inspect.getfile(abathur.cli.main))"
```

**Expected output:**
```
/path/to/worktree/src/abathur/cli/main.py  # ← Correct! Worktree path
```

### 2. Run Tests with Correct Path

```bash
PYTHONPATH=/path/to/worktree/src:$PYTHONPATH \
    poetry run pytest tests/ -v
```

### 3. Run Tests with Coverage

```bash
PYTHONPATH=/path/to/worktree/src:$PYTHONPATH \
    poetry run pytest tests/ -v --cov=src --cov-report=term-missing
```

## Integration with Git Worktree Skill

The `git-worktree` skill should be updated to automatically use PYTHONPATH override when running tests.

### Recommended Skill Update

In `.claude/skills/git-worktree.md`, update the testing section:

```bash
# Test in worktree (with PYTHONPATH override)
PYTHONPATH=$(pwd)/src:$PYTHONPATH poetry run pytest tests/ -v
```

## Alternative Solutions

### Option 2: Reinstall Package in Worktree (Less Reliable)

```bash
cd /path/to/worktree

# Uninstall current abathur
poetry run pip uninstall -y abathur

# Reinstall in editable mode from worktree
poetry run pip install -e .

# Verify it loads from worktree
poetry run python -c "import abathur; import inspect; print(inspect.getfile(abathur))"
```

**Note:** This approach is less reliable because Poetry may still reference the main repository's installation.

### Option 3: Create Isolated Virtual Environment (Overkill)

```bash
cd /path/to/worktree

# Create dedicated venv for this worktree
python -m venv .venv

# Activate it
source .venv/bin/activate

# Install package in editable mode
pip install -e .

# Install dev dependencies
pip install pytest pytest-asyncio pytest-cov

# Now tests should load from worktree
pytest tests/ -v
```

**Note:** This creates an entirely separate Python environment, which may cause dependency version mismatches.

## Best Practices

### 1. Always Use PYTHONPATH Override in Worktrees

Create a shell alias or wrapper script:

```bash
# Add to ~/.bashrc or ~/.zshrc
alias worktree-pytest='PYTHONPATH=$(pwd)/src:$PYTHONPATH poetry run pytest'

# Usage
cd /path/to/worktree
worktree-pytest tests/ -v
```

### 2. Document Worktree Commands

In each worktree's README or commit messages, include the correct test command:

```markdown
## Testing

Run tests with PYTHONPATH override to load worktree code:

```bash
PYTHONPATH=$(pwd)/src:$PYTHONPATH poetry run pytest tests/ -v
```
```

### 3. Verify Before Committing

Before committing changes in a worktree, always verify:

1. Python loads from worktree (not main repo)
2. Tests pass with PYTHONPATH override
3. Implementation is correct in worktree source files

## Troubleshooting

### Tests Still Load from Main Repo

**Symptom:** Even with PYTHONPATH override, imports still load from main repo.

**Solution:** Check if the package is installed in development mode:

```bash
poetry run pip show abathur | grep Location
```

If it shows the main repo path, reinstall:

```bash
poetry run pip uninstall -y abathur
poetry run pip install -e .
```

### "No module named abathur" Error

**Symptom:** `ModuleNotFoundError: No module named 'abathur'`

**Solution:** The PYTHONPATH must point to the directory **containing** the package, not the package itself:

```bash
# WRONG
PYTHONPATH=/path/to/worktree/src/abathur:$PYTHONPATH

# CORRECT
PYTHONPATH=/path/to/worktree/src:$PYTHONPATH
```

### Tests Pass in Worktree But Fail in Main Repo

**Symptom:** Tests pass in worktree with PYTHONPATH override, but fail after merging to main.

**Possible causes:**

1. **Missing files:** Worktree has uncommitted files
2. **Different dependencies:** Poetry.lock differs between worktree and main
3. **Path-dependent code:** Code relies on worktree-specific paths

**Solution:** Before merging:

```bash
# In worktree
git status  # Check for uncommitted files
git diff main..HEAD  # Review all changes
poetry lock --check  # Verify lock file consistency
```

## Summary

**Golden Rule:** Always use PYTHONPATH override when running tests in git worktrees.

```bash
PYTHONPATH=$(pwd)/src:$PYTHONPATH poetry run pytest tests/ -v
```

This ensures Python loads code from the worktree, not the main repository, allowing accurate validation of worktree implementations.
