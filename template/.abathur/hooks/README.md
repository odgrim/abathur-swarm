# Task Hook Scripts

This directory contains hook scripts that execute during task and branch lifecycle events.

## Overview

Hooks enable automated workflows such as:
- **Validation checks** before tasks transition to ready state
- **Integration tests** when feature branches complete
- **Auto-merging** successful task branches into feature branches
- **Failure analysis** when branches complete with errors

## Available Scripts

### `create_feature_branch.sh`

Creates a feature branch with git worktree when a technical-requirements-specialist task starts.

**Usage:**
```bash
./create_feature_branch.sh <task_id> <feature_name>
```

**Hook Configuration:**
```yaml
- event: post_start
  conditions:
    - agent_type: technical-requirements-specialist
  actions:
    - type: run_script
      script_path: ./.abathur/hooks/create_feature_branch.sh
      args: ["${task_id}", "${task_summary}"]
```

### `create_task_worktree.sh`

Creates a task branch with git worktree when an implementation task starts.

**Usage:**
```bash
./create_task_worktree.sh <task_id> <task_branch> <feature_branch> <worktree_path>
```

**Hook Configuration:**
```yaml
- event: pre_start
  conditions:
    - has_metadata_key: worktree_path
      has_metadata_key: task_branch
      has_metadata_key: feature_branch
  actions:
    - type: run_script
      script_path: ./.abathur/hooks/create_task_worktree.sh
      args: ["${task_id}", "${task_branch}", "${feature_branch}", "${worktree_path}"]
```

### `validate_tech_requirements.sh`

Validates technical requirements before allowing a technical-requirements-specialist task to become ready.

**Usage:**
```bash
./validate_tech_requirements.sh <task_id> <parent_task_id>
```

**Hook Configuration:**
```yaml
- event: pre_ready
  conditions:
    - agent_type: technical-requirements-specialist
      parent_agent_type: technical-architect
      min_children_spawned: 3
  actions:
    - type: run_script
      script_path: ./hooks/validate_tech_requirements.sh
      args: ["${task_id}", "${parent_task_id}"]
```

### `integration_test.sh`

Runs integration tests when a feature branch completes all its tasks.

**Usage:**
```bash
./integration_test.sh <feature_branch_name>
```

**Hook Configuration:**
```yaml
- event:
    on_branch_complete:
      branch_type: feature_branch
  conditions:
    - all_tasks_succeeded: true
  actions:
    - type: run_script
      script_path: ./hooks/integration_test.sh
      args: ["${branch_name}"]
```

## Creating Custom Hooks

### Script Requirements

1. **Executable**: Scripts must have execute permissions (`chmod +x script.sh`)
2. **Shebang**: Start with `#!/usr/bin/env bash` or appropriate interpreter
3. **Exit codes**: Return 0 for success, non-zero for failure
4. **Error handling**: Use `set -euo pipefail` for strict error handling

### Environment Variables

Hook scripts receive these environment variables:

- `TASK_ID`: UUID of the current task
- `TASK_AGENT_TYPE`: Agent type of the current task
- `TASK_SUMMARY`: Task summary/title

### Template Variables

Hook configurations support variable substitution:

**Task-level variables:**
- `${task_id}` - Task UUID
- `${task_agent_type}` - Agent type
- `${task_summary}` - Task summary
- `${task_status}` - Current status
- `${task_branch}` - Task branch name
- `${feature_branch}` - Feature branch name
- `${parent_task_id}` - Parent task UUID

**Branch-level variables:**
- `${branch_name}` - Branch name
- `${branch_type}` - TaskBranch or FeatureBranch
- `${total_tasks}` - Total task count
- `${failed_task_count}` - Number of failed tasks
- `${all_succeeded}` - true/false
- `${completed_task_ids}` - Comma-separated task IDs
- `${failed_task_ids}` - Comma-separated failed task IDs

### Example Custom Hook

```bash
#!/usr/bin/env bash
# custom_validation.sh - Example custom validation hook

set -euo pipefail

TASK_ID="${1:-}"
WORKTREE_PATH="${2:-}"

echo "[INFO] Running custom validation for task: $TASK_ID"

# Your validation logic here
if [[ -d "$WORKTREE_PATH" ]]; then
    cd "$WORKTREE_PATH"
    cargo clippy -- -D warnings
    cargo fmt -- --check
fi

echo "[INFO] Validation passed"
exit 0
```

**Hook Configuration:**
```yaml
- id: custom-validation
  description: "Run custom validation checks"
  event: pre_ready
  conditions:
    - agent_type: rust-implementation-specialist
  actions:
    - type: run_script
      script_path: ./hooks/custom_validation.sh
      args: ["${task_id}", "${worktree_path}"]
  priority: 8
  enabled: true
```

## Hook Configuration

Hooks are configured in `.abathur/hooks.yaml`. See that file for complete documentation and examples.

For details on variable substitution and context passing between tasks, see `.abathur/HOOK_CONTEXT_GUIDE.md`.

## Testing Hooks

Test your hook scripts manually before enabling them:

```bash
# Test validation script
./hooks/validate_tech_requirements.sh "test-task-id" "parent-task-id"

# Test integration script
./hooks/integration_test.sh "feature/test-branch"
```

## Debugging

Enable debug logging in hooks:

```bash
# Add to your script
set -x  # Enable command tracing

# Or run with bash -x
bash -x ./hooks/your_script.sh
```

## Best Practices

1. **Keep scripts simple**: Complex logic should be in the application, not hooks
2. **Fail fast**: Exit immediately on errors with meaningful error messages
3. **Log clearly**: Use color-coded output (INFO/WARN/ERROR)
4. **Test thoroughly**: Test hooks in isolation before enabling
5. **Document well**: Add comments explaining what the hook does and why
6. **Version control**: Keep hooks in git with the rest of your code
7. **Idempotent**: Hooks should be safe to run multiple times

## Security Considerations

- **No secrets**: Never hardcode secrets in hook scripts
- **Input validation**: Always validate script arguments
- **Sandboxing**: Hooks run in the same security context as the application
- **Audit logging**: Hook executions are logged for audit trails

## Troubleshooting

### Hook not executing

- Check if hook is enabled in `config/hooks.yaml`
- Verify script has execute permissions
- Check hook conditions match your task/branch
- Review application logs for hook execution errors

### Script failing

- Run script manually to reproduce issue
- Check script exit code: `echo $?`
- Enable debug mode: `bash -x script.sh`
- Review script logs and error messages

### Permission errors

```bash
chmod +x hooks/*.sh  # Make all scripts executable
```

## Contributing

When adding new hooks:

1. Create the script in `hooks/` directory
2. Make it executable: `chmod +x hooks/script.sh`
3. Add hook configuration to `config/hooks.yaml`
4. Document the hook in this README
5. Add tests if applicable
6. Update init templates if the hook should be included by default
