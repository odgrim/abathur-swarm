# Prompt Chain Templates

This directory contains prompt chain templates that orchestrate multi-step agent workflows with hooks.

## Available Chains

### 1. **technical_feature_workflow.yaml** - Default Technical Feature Workflow

The complete default technical feature workflow that orchestrates multi-agent workflows with automatic progression and branching support.

**Flow (Single Feature Mode):**
1. **Gather Requirements** → Research & analyze, store in memory
2. **Design Architecture** → Create system design → CREATE FEATURE BRANCH
3. **Create Technical Specs** → PREPARE FEATURE WORKTREE → Detailed specs
4. **Create Task Plan** → Break into atomic tasks → CREATE TASK WORKTREES → SPAWN IMPLEMENTATION TASKS
5. **Monitor Implementation** → Track progress
6. **Prepare Merge** → RUN TESTS → MERGE BRANCHES → CLEANUP → TAG RELEASE

**Flow (Multiple Features Mode - Branching):**
1. **Gather Requirements** → Research & analyze, store in memory
2. **Design Architecture** → Identifies 2+ distinct features → **SPAWNS N TASKS** (one per feature)
   - Each spawned task has `chain_id: "technical_feature_workflow"`
   - Each continues from step 3 (Create Technical Specs) independently
   - Original chain exits after spawning
   - Result: N parallel feature workflows

**Key Features:**
- **Adaptive Workflow**: Single chain for simple features, branching for complex multi-feature projects
- **Worktree Management**: Automatic creation and cleanup of feature and task worktrees
- **Memory Integration**: Requirements, architecture, and specs stored in memory
- **Agent Spawning**: Automatically creates implementation tasks with proper agent assignments
- **Quality Gates**: Tests and quality checks before merging
- **Git Automation**: Branch creation, merging, and cleanup

**Usage:**
```rust
let loader = ChainLoader::default();
let chain = loader.load_from_file("technical_feature_workflow.yaml")?;

let service = PromptChainService::new()
    .with_hook_executor(hook_executor);

let initial_input = json!({
    "task_description": "Implement user authentication system",
    "project_context": "Rust web service using Axum"
});

let execution = service.execute_chain_with_task(&chain, initial_input, Some(&task)).await?;
```

### 2. **research_synthesis.yaml** - Research & Synthesis Workflow

Multi-step research workflow that identifies sources, extracts information, and synthesizes findings into a coherent report.

**Steps:**
1. Identify authoritative sources
2. Extract key information from sources
3. Synthesize findings into markdown report

**Use Cases:**
- Literature reviews
- Technology evaluations
- Best practice research
- Competitive analysis

### 3. **etl_pipeline.yaml** - Data Processing Pipeline

Extract, transform, and validate data with quality metrics.

**Steps:**
1. Extract data from documents
2. Transform and normalize data
3. Validate quality with metrics

**Use Cases:**
- Data migration
- Document processing
- Data quality assurance
- Information extraction

### 4. **code_review.yaml** - Comprehensive Code Review

Multi-phase code review with structural analysis, quality checks, and security assessment.

**Steps:**
1. Analyze structure and architecture
2. Review code quality
3. Perform security analysis
4. Generate comprehensive report

**Use Cases:**
- Pre-merge code reviews
- Security audits
- Code quality assessments
- Architecture reviews

### 5. **multi_agent_workflow.yaml** - Example Multi-Agent Coordination

Demonstrates how multiple agents can coordinate with worktree preparation and cleanup.

**Features:**
- Requirements analysis
- Implementation with worktree preparation
- Code review
- Conditional merging with cleanup

## Hook Integration

All chains support hooks at the step level:

### Pre-Hooks
Execute before a step begins:
```yaml
pre_hooks:
  - type: run_script
    script_path: scripts/prepare_worktree.sh
    args:
      - "{task_id}"
      - "implementation"
```

### Post-Hooks
Execute after a step completes:
```yaml
post_hooks:
  - type: run_script
    script_path: scripts/run_tests.sh
    args:
      - "{task_id}"
  - type: merge_branch
    source: "task/{task_id}"
    target: "feature/{task_id}"
    strategy:
      type: rebase
  - type: log_message
    level: info
    message: "Completed step for task {task_id}"
```

### Available Hook Actions

- **run_script**: Execute shell scripts with variable substitution
- **spawn_task**: Create new tasks
- **merge_branch**: Merge git branches
- **delete_branch**: Delete branches and cleanup worktrees
- **create_tag**: Create git tags
- **log_message**: Log messages at various levels
- **notify_webhook**: Send webhook notifications
- **update_field**: Update task fields

### Variable Substitution in Hooks

Available variables:
- `{task_id}` - Current task ID
- `{chain_step_id}` - Current step ID in the chain
- `{hook_type}` - "pre" or "post"
- `{agent_type}` - Agent type executing the chain
- `{parent_task_id}` - Parent task ID (if applicable)
- Any output fields from previous steps (JSON path)

## Supporting Scripts

Located in `scripts/`:

- **create_feature_branch.sh** - Create feature branch for work
- **prepare_feature_worktree.sh** - Set up feature branch worktree
- **create_task_worktrees.sh** - Create worktrees for implementation tasks
- **cleanup_all_worktrees.sh** - Clean up all worktrees after merge
- **store_requirements.sh** - Store requirements in memory
- **store_architecture.sh** - Store architecture in memory
- **store_technical_specs.sh** - Store technical specs in memory
- **spawn_implementation_tasks.sh** - Create implementation tasks
- **run_all_tests.sh** - Execute full test suite
- **quality_checks.sh** - Run quality checks
- **merge_branches.sh** - Merge task branches to feature branch

## Creating Custom Chains

### Basic Structure

```yaml
name: my_workflow
description: "Description of what this workflow does"

steps:
  - id: step_identifier
    role: Agent Role
    prompt: |
      Prompt template with {variable} substitution
    expected_output:
      type: json|xml|markdown|plain
      schema: # Optional JSON schema
    pre_hooks: []  # Optional
    post_hooks: [] # Optional
    next: next_step_id  # Optional
    timeout_secs: 300   # Optional

validation_rules: # Optional
  - step_id: step_identifier
    rule_type:
      type: json_schema|regex_match|custom_validator
    error_message: "Error message if validation fails"
```

### Best Practices

1. **Clear Step Boundaries**: Each step should have a single, well-defined responsibility
2. **Structured Output**: Use JSON with schemas for steps that feed into other steps
3. **Variable Passing**: Use JSON output from previous steps as input to next steps
4. **Worktree Hooks**: Use pre-hooks for setup, post-hooks for cleanup
5. **Validation**: Add validation rules to catch errors early
6. **Timeouts**: Set realistic timeouts based on step complexity
7. **Error Messages**: Provide clear, actionable error messages
8. **Documentation**: Include descriptions and comments in YAML

### Testing Chains

```rust
#[tokio::test]
async fn test_my_workflow() {
    let loader = ChainLoader::default();
    let chain = loader.load_from_file("my_workflow.yaml")?;

    // Validate chain structure
    assert!(chain.validate().is_ok());

    // Test execution
    let service = PromptChainService::new();
    let input = json!({"key": "value"});
    let result = service.execute_chain(&chain, input).await?;

    assert_eq!(result.status, ChainStatus::Completed);
}
```

## Integration with Task Queue

Chains can be triggered automatically by the task queue:

```rust
// In task coordinator
match task.agent_type.as_str() {
    "requirements-gatherer" => {
        // Load and execute technical feature workflow chain
        let chain = chain_loader.load_from_file("technical_feature_workflow.yaml")?;
        chain_service.execute_chain_with_task(&chain, initial_input, Some(&task)).await?;
    }
    _ => {
        // Traditional single-agent execution
        execute_agent(&task).await?;
    }
}
```

## Monitoring & Debugging

### Execution Logs

Chains produce detailed logs at each step:
```
INFO Executing step 1/6: gather_requirements (role: requirements-gatherer)
INFO Executing 1 pre-hooks for step gather_requirements
INFO Hook 1/1 executed successfully
INFO Step gather_requirements completed successfully in 45.2s
INFO Executing 1 post-hooks for step gather_requirements
```

### Chain Execution Records

All executions are stored in the database:
```sql
SELECT * FROM chain_executions WHERE chain_id = 'technical_feature_workflow';
```

### Step Results

Access individual step outputs:
```rust
for result in execution.step_results {
    println!("Step: {} - Duration: {:?}", result.step_id, result.duration);
    println!("Output: {}", result.output);
}
```

## Future Enhancements

- **Parallel Step Execution**: Execute independent steps concurrently
- **Sub-Chain Composition**: Chains can spawn other chains
- **Dynamic Step Selection**: Conditional branching based on results
- **Rollback Support**: Undo changes if later steps fail
- **Progress Streaming**: Real-time progress updates
- **Chain Templates**: Parameterized chains with variables
