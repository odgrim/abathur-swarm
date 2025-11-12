# Abathur Core Scripts

This directory contains the core Abathur framework scripts that manage branch creation, worktree management, and task orchestration.

## Scripts

### Branch Management

- **create_feature_branch.sh** - Creates feature branches with human-readable names
  - Usage: `./create_feature_branch.sh <task_id> <feature_name> <decomposition_strategy>`
  - Example: `./create_feature_branch.sh abc123 user-authentication single`
  - Creates: `feature/user-authentication`

- **prepare_feature_worktree.sh** - Creates worktrees for feature branches
  - Usage: `./prepare_feature_worktree.sh <task_id> <feature_name>`
  - Example: `./prepare_feature_worktree.sh abc123 user-authentication`
  - Creates: `.abathur/worktrees/feature-user-authentication`

- **create_task_worktrees.sh** - Creates worktrees for individual tasks
  - Usage: `./create_task_worktrees.sh <task_id> <tasks_json>`
  - Creates: `.abathur/worktrees/tasks/{feature-name}-{task-id}`
  - Example: `.abathur/worktrees/tasks/user-authentication-implement-user-model`

### Memory Management

- **store_requirements.sh** - Stores requirements in Abathur memory
- **store_architecture.sh** - Stores architecture decisions in Abathur memory
- **store_technical_specs.sh** - Stores technical specifications in Abathur memory
- **update_progress.sh** - Updates task progress in Abathur memory

### Task Orchestration

- **spawn_implementation_tasks.sh** - Spawns implementation tasks via task queue
- **merge_task_branches.sh** - Merges task branches back to feature branch
- **cleanup_all_worktrees.sh** - Cleans up completed worktrees and branches

## Branch Naming Conventions

### Feature Branches
- Format: `feature/{feature-name}`
- Example: `feature/user-authentication`
- Names are kebab-case, derived from the feature description

### Task Branches
- Format: `task/{feature-name}/{task-id}`
- Example: `task/user-authentication/implement-user-model`
- Task IDs are kebab-case slugs derived from task summaries

### Worktree Paths
- Feature: `.abathur/worktrees/feature-{feature-name}`
- Tasks: `.abathur/worktrees/tasks/{feature-name}-{task-id}`

## Notes

- These scripts are automatically copied during `abathur init`
- Scripts are made executable (755 permissions on Unix systems)
- All scripts use the `.abathur/` directory for state management
- Scripts integrate with Abathur's memory system for persistence
