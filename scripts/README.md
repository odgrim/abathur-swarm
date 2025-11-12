# Project Scripts

This directory is for **project-specific** scripts.

## Core Abathur Scripts

The core Abathur framework scripts have been moved to `.abathur/scripts/` and are automatically copied during `abathur init`. These include:

- Feature branch creation and worktree management
- Task orchestration and branch management
- Memory storage and progress tracking

See `template/.abathur/scripts/README.md` for documentation on core scripts.

## Project-Specific Scripts

This directory should contain scripts specific to your project, such as:

- `run_all_tests.sh` - Project-specific test runner
- `quality_checks.sh` - Project-specific linting and quality gates
- Custom build scripts
- Deployment scripts
- Project-specific automation

These scripts can be customized per project and are referenced by the technical feature workflow chain.
