# Project Context Integration Guide

## Overview

This document describes how to integrate the `project-context-scanner` agent with the existing Abathur workflow to enable multi-language/multi-project support.

## Components

### 1. Project Context Scanner Agent

**Location**: `template/.claude/agents/abathur/project-context-scanner.md`

**Purpose**: Fast Haiku-based agent that scans the project once at initialization to detect language, frameworks, tooling, and conventions.

**Execution**: Auto-enqueued with highest priority (10) when swarm starts if project context doesn't exist in memory.

**Memory Output**:
- Namespace: `project:context`
- Key: `metadata`
- Contains: language, frameworks, build system, tooling commands, validation requirements

### 2. Auto-Enqueue Mechanism

**Implementation Location**: `src/cli/commands/swarm.rs` in `handle_start()` function OR `src/application/swarm_orchestrator.rs` in `start()` method

**Logic**:

```rust
// Pseudo-code for auto-enqueue mechanism
async fn handle_start(task_service: &TaskQueueServiceAdapter, max_agents: usize) -> Result<()> {
    // ... existing startup code ...

    // After swarm orchestrator starts, check for project context
    let memory_service = MemoryService::new(/* ... */);

    // Check if project context exists
    let context_exists = memory_service
        .get("project:context", "metadata")
        .await
        .is_ok();

    if !context_exists {
        // Auto-enqueue project-context-scanner task with highest priority
        task_service.enqueue(CreateTaskRequest {
            summary: "Scan project context".to_string(),
            description: "Initial project scan to detect language, framework, and conventions.".to_string(),
            agent_type: "project-context-scanner".to_string(),
            priority: 10, // HIGHEST priority - runs first
            parent_task_id: None,
            dependencies: vec![],
            dependency_type: DependencyType::Sequential,
        }).await?;

        println!("✓ Auto-enqueued project-context-scanner task (runs first)");
    } else {
        println!("✓ Project context already detected");
    }

    // Continue with swarm startup
    Ok(())
}
```

**Key Points**:
- Only enqueues once per project (checks if context exists)
- Uses priority 10 (highest) to ensure it runs before any other tasks
- Does not block swarm startup
- Task executes asynchronously in the task queue

### 3. Agent Updates to Consume Project Context

All core agents and worker agents should load project context early in their workflow.

#### A. Requirements Gatherer

**File**: `template/.claude/agents/abathur/requirements-gatherer.md`

**Update**: Add Step 1.5 to workflow (after Analyze, before Research):

```markdown
## Workflow

1. **Analyze**: Parse task description for problem, requirements, constraints
1.5. **Load Project Context**: Retrieve project metadata from memory (REQUIRED)
   ```python
   # Load project context
   project_context = memory_get({
       "namespace": "project:context",
       "key": "metadata"
   })

   # Extract key information
   language = project_context["language"]["primary"]
   frameworks = project_context["frameworks"]
   architecture = project_context["conventions"]["architecture"]
   ```
2. **Research**: Use WebFetch/WebSearch for best practices, Glob/Read/Grep for codebase analysis
   - Consider project's existing tech stack when researching
   - Look for patterns matching detected architecture
   - Research language-specific best practices for {language}
```

**Benefits**:
- Understands project language before researching
- Can tailor requirements to existing tech stack
- Maintains consistency with project conventions

#### B. Task Planner

**File**: `template/.claude/agents/abathur/task-planner.md`

**Update**: Add project context loading and language-aware agent selection:

```markdown
## Workflow

1. **Load Technical Specs**: Retrieve from memory namespace `task:{tech_spec_id}:technical_specs`
1.5. **Load Project Context**: Retrieve project metadata (REQUIRED)
   ```python
   # Load project context
   project_context = memory_get({
       "namespace": "project:context",
       "key": "metadata"
   })

   # Extract validation requirements
   language = project_context["language"]["primary"]
   validation_agent = project_context["validation_requirements"]["validation_agent"]
   build_command = project_context["tooling"]["build_command"]
   test_command = project_context["tooling"]["test_runner"]["command"]
   ```
2. **Analyze Scope**: Understand component boundaries, avoid duplicating other planners' work
```

**Update**: Agent selection logic (Section: "Agent Orchestration"):

```markdown
## Agent Orchestration

**CRITICAL: Use Project Context for Agent Selection**

1. **Load Project Context**: Get language and validation requirements
2. **Determine Agent Types**: Use language prefix for all worker agents
   - Rust project: `rust-domain-models-specialist`, `rust-testing-specialist`
   - Python project: `python-domain-models-specialist`, `python-testing-specialist`
   - TypeScript project: `typescript-domain-models-specialist`, `typescript-testing-specialist`
   - Go project: `go-domain-models-specialist`, `go-testing-specialist`

3. **Select Validation Agent**: ALWAYS use `{language}-validation-specialist`
   ```python
   # Get validation agent from project context
   validation_agent = project_context["validation_requirements"]["validation_agent"]
   # Examples:
   # - rust-validation-specialist
   # - python-validation-specialist
   # - typescript-validation-specialist
   # - go-validation-specialist
   ```

4. **Check Existing Agents**: `Glob(f".claude/agents/**/{language}-*.md")`
5. **Spawn Agent-Creator**: For missing agents, pass language context
```

**Update**: Validation task pattern (Section: "Validation Task Pattern"):

```markdown
## Validation Task Pattern (MANDATORY)

For EVERY implementation task, spawn validation task using project-specific validator:

```json
{
  "summary": "Validate {component} implementation",
  "agent_type": "{validation_agent from project_context}",
  "priority": 4,
  "prerequisite_task_ids": ["{implementation_task_id}"],
  "metadata": {
    "worktree_path": "{same_as_implementation}",
    "task_branch": "{same_as_implementation}",
    "validation_checks": [
      "compilation",
      "linting",
      "formatting",
      "unit_tests"
    ],
    "build_command": "{from project_context}",
    "test_command": "{from project_context}"
  }
}
```

**Validation is MANDATORY**:
- Compilation/build MUST pass
- Linting MUST pass
- Formatting MUST pass
- All tests MUST pass
- If ANY check fails, task fails (triggers remediation)
```

#### C. Agent Creator

**File**: `template/.claude/agents/abathur/agent-creator.md`

**Update**: Add language awareness:

```markdown
## Workflow

1. **Load Context**: Get current task and agent requirement details
1.5. **Load Project Context**: Retrieve language and framework info (REQUIRED)
   ```python
   # Load project context
   project_context = memory_get({
       "namespace": "project:context",
       "key": "metadata"
   })

   language = project_context["language"]["primary"]
   frameworks = project_context["frameworks"]
   ```

2. **Research Domain**: Use WebFetch for best practices
   - Research {language}-specific patterns
   - Research {framework}-specific patterns
   - Look for official docs and style guides

3. **Check Existing Agents**: Verify agent doesn't already exist
   - Check for exact match
   - Check for similar agents in same language
   - Prevent duplication

4. **Design Agent**: Create comprehensive prompt with language-specific guidance
   - Use naming convention: `{language}-{domain}-specialist`
   - Include language-specific tool recommendations
   - Add framework-specific examples
   - Reference project conventions
```

**Update**: Agent creation (add language context):

```markdown
## Agent Creation

**Naming Convention**: `{language}-{domain}-specialist.md`

Examples:
- `rust-websocket-specialist.md`
- `python-celery-specialist.md`
- `typescript-graphql-specialist.md`
- `go-grpc-specialist.md`

**Agent Template Structure**:

```markdown
---
name: {language}-{domain}-specialist
description: "Use proactively for {domain-specific task} in {language} projects. Keywords: {language}, {domain}, {key-terms}"
model: sonnet
color: Green
tools:
  - Read
  - Write
  - Edit
  - Bash  # For running language-specific commands
  - Glob
  - Grep
mcp_servers:
  - abathur-memory
  - abathur-task-queue
---

## Purpose

You are a {Language} {Domain} Specialist...

## Language-Specific Commands

**Build**: {build_command from project_context}
**Test**: {test_command from project_context}
**Lint**: {linter.command from project_context}
**Format**: {formatter.command from project_context}

## Instructions

1. Load project context for language-specific settings
2. Follow {language} conventions: {naming_convention}
3. Use {framework} patterns where applicable
4. Ensure all code passes validation checks
```
```

#### D. Technical Architect

**File**: `template/.claude/agents/abathur/technical-architect.md`

**Update**: Add project awareness:

```markdown
## Workflow

1. **Load Requirements**: Retrieve from memory namespace `task:{requirements_task_id}:requirements`
1.5. **Load Project Context**: Understand existing tech stack (REQUIRED)
   ```python
   project_context = memory_get({
       "namespace": "project:context",
       "key": "metadata"
   })

   existing_language = project_context["language"]["primary"]
   existing_frameworks = project_context["frameworks"]
   existing_architecture = project_context["conventions"]["architecture"]
   ```

2. **Check Duplicates**: Search memory for existing architecture work
3. **Research**: Use WebFetch/WebSearch for best practices
   - Research MUST align with existing tech stack
   - Consider integration with existing {existing_language} codebase
   - Respect existing architecture pattern: {existing_architecture}

4. **Analyze Architecture**: Identify components, boundaries, integration points
   - Design must integrate with existing {existing_frameworks}
   - Maintain consistency with project conventions
```

### 4. Validation Specialist Templates

Create language-specific validation agents that follow identical patterns:

#### Template: Language Validation Specialist

**Files to Create**:
- `template/.claude/agents/workers/rust-validation-specialist.md` (already covered by rust-testing-specialist)
- `template/.claude/agents/workers/python-validation-specialist.md`
- `template/.claude/agents/workers/typescript-validation-specialist.md`
- `template/.claude/agents/workers/go-validation-specialist.md`

**Common Structure** (all validators follow this pattern):

```markdown
---
name: {language}-validation-specialist
description: "Validates {language} code by running compilation, linting, formatting, and tests. All checks must pass for validation to succeed. Keywords: {language}, validation, quality gates"
model: sonnet
color: Red
tools:
  - Bash
  - Read
  - Grep
  - Glob
mcp_servers:
  - abathur-memory
  - abathur-task-queue
---

## Purpose

Quality gate validator for {language} implementations. Runs mandatory checks:
1. Compilation/Build
2. Linting
3. Formatting
4. Unit Tests

ALL checks must pass for task to be marked as validated.

## Workflow

1. **Load Task Context**: Get implementation task details
2. **Load Project Context**: Get language-specific commands
3. **Run Compilation**: Execute build command → MUST PASS
4. **Run Linter**: Execute lint command → MUST PASS
5. **Check Formatting**: Execute format check → MUST PASS
6. **Run Tests**: Execute test suite → MUST PASS
7. **Report Results**: Mark validation as passed/failed

## Validation Commands

### {Language}-Specific

**Load from project context**:
```python
project_context = memory_get({"namespace": "project:context", "key": "metadata"})

build_cmd = project_context["tooling"]["build_command"]
lint_cmd = project_context["tooling"]["linter"]["command"]
format_check_cmd = project_context["tooling"]["formatter"]["check_command"]
test_cmd = project_context["tooling"]["test_runner"]["command"]
```

## Execution

### 1. Compilation Check
```bash
cd {worktree_path}
{build_command}
```
**Expected**: Exit code 0, no errors
**If fails**: Mark task as failed, store error output

### 2. Linting Check
```bash
cd {worktree_path}
{lint_command}
```
**Expected**: Exit code 0, no linting errors
**If fails**: Mark task as failed, store warnings/errors

### 3. Formatting Check
```bash
cd {worktree_path}
{format_check_command}
```
**Expected**: Exit code 0, all files properly formatted
**If fails**: Mark task as failed, list unformatted files

### 4. Test Execution
```bash
cd {worktree_path}
{test_command}
```
**Expected**: Exit code 0, all tests pass
**If fails**: Mark task as failed, store test failures

## Failure Handling

If ANY check fails:
1. Mark task status as Failed
2. Store failure details in memory
3. Trigger remediation workflow (task-planner respawns with fixes)

## Success Criteria

ALL of the following must be true:
- ✓ Build/compilation succeeds
- ✓ Linter reports no errors
- ✓ Code is properly formatted
- ✓ All tests pass

Only when ALL checks pass: Mark validation as Completed
```

### 5. Implementation Checklist

**Phase 1: Core Infrastructure** (Blocks multi-language support)

- [ ] Create `project-context-scanner.md` agent (✓ DONE)
- [ ] Implement auto-enqueue in `src/cli/commands/swarm.rs` or `src/application/swarm_orchestrator.rs`
  - Check if `project:context/metadata` exists in memory
  - If not exists, enqueue project-context-scanner with priority 10
  - Log status to user
- [ ] Test auto-enqueue with fresh project initialization

**Phase 2: Agent Updates** (Enable context consumption)

- [ ] Update `requirements-gatherer.md` - Add Step 1.5 to load project context
- [ ] Update `task-planner.md` - Load context, use language-specific agent names
- [ ] Update `agent-creator.md` - Use language prefix, include language-specific patterns
- [ ] Update `technical-architect.md` - Respect existing tech stack
- [ ] Test with Rust project (existing agents)

**Phase 3: Validation Specialists** (Mandatory quality gates)

- [ ] Create `python-validation-specialist.md` template
- [ ] Create `typescript-validation-specialist.md` template
- [ ] Create `go-validation-specialist.md` template
- [ ] Update `task-planner.md` validation task creation to use `{language}-validation-specialist`
- [ ] Test validation with failing linting/formatting/tests

**Phase 4: Worker Agent Templates** (Language-specific implementations)

- [ ] Create Python worker agents (domain, testing, fastapi, sqlalchemy, etc.)
- [ ] Create TypeScript worker agents (domain, testing, express, prisma, etc.)
- [ ] Create Go worker agents (domain, testing, http, database, etc.)
- [ ] Test agent-creator can dynamically generate language-specific agents

**Phase 5: Testing** (Validation)

- [ ] Test with pure Rust project (existing workflow)
- [ ] Test with pure Python project (new workflow)
- [ ] Test with pure TypeScript project (new workflow)
- [ ] Test with pure Go project (new workflow)
- [ ] Test validation failures trigger remediation correctly

### 6. Expected User Experience

**Initial Project Setup**:

```bash
# User initializes Abathur in their Python project
cd my-python-project
abathur init
abathur swarm start

# Output:
# ✓ Auto-enqueued project-context-scanner task (runs first)
# ✓ Starting swarm orchestrator with 10 max agents...
# ✓ Swarm orchestrator started successfully

# Project context scanner runs automatically (< 2 min)
# Detects: Python, FastAPI, pytest, SQLAlchemy
# Stores in memory: project:context/metadata

# User submits first feature request
abathur task enqueue "Add user authentication"

# Workflow:
# 1. requirements-gatherer loads project context → knows it's Python
# 2. technical-architect respects existing FastAPI stack
# 3. task-planner selects python-* worker agents
# 4. task-planner uses python-validation-specialist for quality gates
# 5. All implementations validated with pytest + pylint + black
```

**Subsequent Feature Requests**:

```bash
# Project context already exists in memory
abathur task enqueue "Add password reset functionality"

# Workflow uses cached context → immediate Python-aware planning
# No need to re-scan project
```

## Summary

The project-context-scanner agent enables Abathur to work on **any project** by:

1. **Auto-detecting** language, framework, conventions once per project
2. **Storing** comprehensive context in memory for all agents
3. **Language-aware** agent selection (rust-*, python-*, typescript-*, go-*)
4. **Mandatory validation** with language-specific quality gates
5. **Zero human intervention** - fully autonomous multi-language support

This design maintains the "one Abathur instance per project" model while enabling Abathur to work on Rust, Python, TypeScript, Go, Java, and any future languages.
