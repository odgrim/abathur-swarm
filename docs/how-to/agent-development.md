# How to Create Custom Agents

Learn how to extend Abathur Swarm with custom specialized agents tailored to your project's needs.

## Prerequisites

- Abathur CLI installed and configured
- Basic understanding of agent orchestration
- Familiarity with markdown and YAML
- Knowledge of your target domain

## Overview

This guide shows you how to create custom specialized agents for Abathur Swarm. You'll learn:
- When to create a custom agent vs using existing ones
- Agent file structure and frontmatter configuration
- Step-by-step agent creation process
- Best practices for agent specialization

## When to Create a Custom Agent

Create a custom agent when:

- **Existing agents don't match your use case**: You need specialized behavior not covered by built-in agents
- **Repetitive specialized tasks**: You frequently perform domain-specific tasks that require specialized knowledge
- **Tool access patterns**: You need a specific combination of tools with unique workflows
- **Domain expertise**: Your project requires deep expertise in a particular technology or domain

Use existing agents when:
- A built-in agent already handles your use case
- You can accomplish the task with general-purpose agents
- The task is one-off and doesn't warrant agent creation

!!! tip "Start with Existing Agents"
    Before creating a custom agent, review the [Agent Reference](../reference/agents.md) to ensure similar functionality doesn't already exist.

## Solution

### Approach 1: Create Agent from Template

This is the recommended approach for most use cases.

#### Step 1: Create Agent File

Create a new markdown file in `.claude/agents/workers/`:

```bash
touch .claude/agents/workers/your-agent-name.md
```

**Naming Conventions**:
- Use lowercase with hyphens: `rust-testing-specialist.md`
- Be specific and descriptive: `python-django-api-specialist.md` not `web-agent.md`
- Include technology and purpose: `<technology>-<purpose>-specialist.md`

#### Step 2: Write Agent Frontmatter

Open the agent file and add YAML frontmatter with required metadata:

```yaml
---
name: your-agent-name
description: "Use proactively for [specific purpose]. Keywords: keyword1, keyword2, keyword3"
model: sonnet
color: Blue
tools:
  - Read
  - Write
  - Edit
  - Bash
mcp_servers:
  - abathur-memory
  - abathur-task-queue
---
```

**Frontmatter Fields**:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Unique agent identifier (matches filename) |
| `description` | string | Yes | When to use this agent and relevant keywords |
| `model` | string | Yes | AI model: `sonnet`, `opus`, or `haiku` |
| `color` | string | No | Terminal display color |
| `tools` | list | Yes | Tools agent can access |
| `mcp_servers` | list | No | MCP servers for additional capabilities |

**Model Selection**:
- `sonnet`: Balanced performance and cost (recommended default)
- `opus`: Maximum capability for complex tasks
- `haiku`: Fast and cost-effective for simple, repetitive tasks

**Tool Access** (principle of least privilege):
- Only grant tools the agent actually needs
- Common tools: `Read`, `Write`, `Edit`, `Bash`, `Grep`, `Glob`
- Specialized tools: `MultiEdit`, `TodoWrite`, `WebFetch`, `WebSearch`

#### Step 3: Define Agent Instructions

After the frontmatter, write clear instructions for what the agent should do:

```markdown
## Purpose

You are a [Role/Specialty], hyperspecialized in [specific capabilities].

**Critical Responsibility**:
- Primary responsibility 1
- Primary responsibility 2
- Key constraint or safety requirement

## Instructions

When invoked, you must follow these steps:

1. **Load Context**
   - Retrieve necessary information from memory
   - Understand task requirements
   - Identify dependencies

2. **Analyze Requirements**
   - Determine approach based on task context
   - Identify tools needed
   - Plan execution steps

3. **Execute Task**
   - Perform primary work (be specific here)
   - Use tools appropriately
   - Handle errors gracefully

4. **Verify Results**
   - Validate output meets requirements
   - Run tests or checks as appropriate
   - Ensure quality standards met

5. **Store Results**
   - Save relevant information to memory
   - Update task status
   - Provide completion summary
```

**Instructions Best Practices**:
- Be specific and prescriptive (not vague)
- Number steps clearly
- Include code examples where relevant
- Specify error handling requirements
- Define success criteria

#### Step 4: Add Best Practices Section

Document patterns and guidelines specific to your agent's domain:

```markdown
**Best Practices:**

**[Category 1]:**
- Guideline 1 with reasoning
- Guideline 2 with example
- Guideline 3 with warning

**[Category 2]:**
- Pattern to follow
- Anti-pattern to avoid
- Edge case handling

**Critical Rules:**
- NEVER do X (with reason)
- ALWAYS do Y (with reason)
- Use Z when condition applies
```

#### Step 5: Define Deliverable Format

Specify the JSON output format for orchestration:

```markdown
**Deliverable Output Format:**

```json
{
  "execution_status": {
    "status": "SUCCESS",
    "agent_name": "your-agent-name"
  },
  "deliverables": {
    "files_created": ["path/to/file1", "path/to/file2"],
    "specific_metric": "value"
  },
  "orchestration_context": {
    "next_recommended_action": "What should happen next",
    "requires_review": true
  }
}
```
```

#### Step 6: Test Your Agent

Test the agent by creating a task that invokes it:

```bash
# Create test task
abathur task enqueue \
  --summary "Test custom agent" \
  --description "Verify agent works correctly" \
  --agent-type your-agent-name

# Monitor task execution
abathur task list

# View task details
abathur task get <task-id>
```

**Verification Checklist**:
- [ ] Agent receives and understands task correctly
- [ ] Agent has access to required tools
- [ ] Agent produces expected output
- [ ] Agent handles errors gracefully
- [ ] Agent returns proper JSON format

### Approach 2: Extend Existing Agent

When you need slight variations on existing agent behavior:

1. **Copy existing agent file** as starting point:
   ```bash
   cp .claude/agents/workers/rust-testing-specialist.md \
      .claude/agents/workers/python-testing-specialist.md
   ```

2. **Modify frontmatter** with new name and description

3. **Adapt instructions** to new domain while keeping structure

4. **Update tool requirements** if different

5. **Test thoroughly** to ensure changes work as expected

!!! warning "Maintain Naming Consistency"
    Ensure the `name` field in frontmatter matches the filename exactly (without `.md` extension).

## Common Agent Patterns

### Pattern 1: Code Analysis Agent

Perfect for domain-specific code review or analysis tasks.

**Example: Go Code Reviewer**

```yaml
---
name: go-code-reviewer
description: "Use proactively for reviewing Go code following best practices. Keywords: go, golang, code review, best practices, linting"
model: sonnet
color: Cyan
tools:
  - Read
  - Grep
  - Glob
mcp_servers:
  - abathur-memory
---

## Purpose

You are a Go Code Reviewer, hyperspecialized in analyzing Go code for correctness, performance, and adherence to Go best practices.

## Instructions

1. **Analyze Codebase Structure**
   - Use Glob to find all .go files
   - Read package structure and dependencies
   - Identify public APIs and interfaces

2. **Review Code Quality**
   - Check for common Go anti-patterns
   - Verify error handling follows Go conventions
   - Review concurrency patterns (goroutines, channels)
   - Validate interface usage

3. **Generate Review Report**
   - List issues by severity (critical, warning, suggestion)
   - Provide specific line references
   - Suggest improvements with examples
```

### Pattern 2: Documentation Writer Agent

Specialized agents for creating domain-specific documentation.

**Example: API Documentation Generator**

```yaml
---
name: api-documentation-generator
description: "Use proactively for generating API documentation from code. Keywords: API docs, OpenAPI, REST, documentation generation"
model: sonnet
color: Green
tools:
  - Read
  - Write
  - Grep
  - Glob
---

## Purpose

You are an API Documentation Generator, specialized in creating comprehensive API documentation from source code.

## Instructions

1. **Extract API Endpoints**
   - Scan codebase for route definitions
   - Identify HTTP methods and paths
   - Extract request/response schemas

2. **Generate Documentation**
   - Create OpenAPI 3.0 specification
   - Document all endpoints with examples
   - Include authentication requirements
   - Add response codes and error cases

3. **Write Human-Readable Guides**
   - Create getting started guide
   - Document common use cases
   - Provide curl examples
```

### Pattern 3: Testing Agent

Agents specialized in writing tests for specific frameworks.

**Example: React Component Tester**

```yaml
---
name: react-component-tester
description: "Use proactively for writing React component tests with Jest and Testing Library. Keywords: react, testing, jest, testing-library, components"
model: sonnet
color: Yellow
tools:
  - Read
  - Write
  - Edit
  - Bash
---

## Purpose

You are a React Testing Specialist, hyperspecialized in writing comprehensive tests for React components using Jest and React Testing Library.

## Instructions

1. **Analyze Component**
   - Read component source code
   - Identify props, state, and behavior
   - Determine user interactions to test
   - Identify edge cases

2. **Write Tests**
   - Create test file: ComponentName.test.tsx
   - Write render tests
   - Write interaction tests
   - Write accessibility tests
   - Test error states

3. **Verify Coverage**
   - Run jest with coverage
   - Ensure >80% coverage
   - Add tests for uncovered branches
```

## Best Practices

### Agent Specialization

**Hyperspecialization Principle**:
- Agents should do ONE thing exceptionally well
- Narrow scope = better performance and reliability
- Combine multiple specialized agents for complex workflows

**Good Specialization** (specific domain):
- `rust-sqlx-database-specialist` - Rust + SQLx + database patterns
- `python-fastapi-api-specialist` - Python + FastAPI + REST APIs
- `react-testing-library-specialist` - React + Testing Library

**Poor Specialization** (too broad):
- `general-purpose-coder` - No clear domain
- `fullstack-developer` - Too many responsibilities
- `problem-solver` - Vague, undefined scope

### Tool Access Control

**Principle of Least Privilege**:
- Grant only tools agent actually uses
- Reduces potential for unintended actions
- Improves agent focus and performance

**Tool Selection Guide**:

| Task Type | Required Tools | Optional Tools |
|-----------|---------------|----------------|
| Code analysis | Read, Grep, Glob | - |
| Code modification | Read, Edit | Write (for new files) |
| Testing | Read, Write, Bash | Grep, Glob |
| Documentation | Read, Write | WebFetch, WebSearch |
| Configuration | Read, Edit | Bash |

### Instruction Clarity

**Be Specific and Prescriptive**:

```markdown
❌ Poor: "Analyze the code and make improvements"
✅ Good: "Read all .rs files, run clippy, fix warnings, run cargo test"

❌ Poor: "Write tests"
✅ Good: "Write unit tests in #[cfg(test)] module, integration tests in tests/ directory, achieve >80% coverage"

❌ Poor: "Handle errors appropriately"
✅ Good: "Return Result<T, Error> for all fallible operations, add context with .context() at layer boundaries"
```

**Use Examples**:
- Include code snippets showing expected patterns
- Show both correct and incorrect approaches
- Provide templates for common structures

### Memory and State Management

Agents should use memory system for state:

```markdown
## Instructions

1. **Load Previous Context**
   ```python
   # Retrieve parent task context
   parent_context = memory_get({
       "namespace": f"task:{parent_task_id}:context",
       "key": "architecture"
   })
   ```

2. **Execute Work**
   [Agent performs its task]

3. **Store Results**
   ```python
   # Store results for downstream agents
   memory_add({
       "namespace": f"task:{task_id}:results",
       "key": "output",
       "value": {"key": "value"},
       "memory_type": "episodic",
       "created_by": "agent-name"
   })
   ```
```

### Error Handling

Specify how agents should handle errors:

```markdown
**Error Handling:**

**Recoverable Errors**:
- Retry with exponential backoff
- Log error with context
- Continue with degraded functionality

**Critical Errors**:
- Stop execution immediately
- Return detailed error in JSON
- Preserve partial results in memory
- Set status to "FAILED"

**Validation Errors**:
- Validate inputs before processing
- Return clear error messages
- Suggest corrections when possible
```

## Troubleshooting

### Problem: Agent Not Found

**Cause**: Agent name mismatch between filename and frontmatter

**Solution**:
```bash
# Check filename
ls .claude/agents/workers/your-agent-name.md

# Verify frontmatter name field matches
grep "^name:" .claude/agents/workers/your-agent-name.md
```

Ensure both match exactly (case-sensitive).

### Problem: Agent Lacks Tool Access

**Cause**: Required tool not listed in frontmatter

**Solution**:
Edit agent file and add tool to `tools` list:

```yaml
tools:
  - Read
  - Write
  - Bash  # Add missing tool
```

### Problem: Task Assigned to Wrong Agent

**Cause**: Agent description or keywords don't match use case

**Solution**:
1. Explicitly specify agent in task:
   ```bash
   abathur task enqueue --agent-type your-agent-name --summary "Task"
   ```

2. Improve agent description with clearer keywords:
   ```yaml
   description: "Use proactively for X. Keywords: very, specific, keywords, that, match, usecase"
   ```

### Problem: Agent Produces Inconsistent Results

**Cause**: Instructions too vague or model inappropriate

**Solution**:
1. Make instructions more specific and prescriptive
2. Add examples of expected behavior
3. Consider using `opus` model for complex reasoning
4. Add validation steps to instructions

### Problem: Agent Times Out

**Cause**: Task too complex or agent doing too much

**Solution**:
1. Break agent into multiple specialized agents
2. Use `haiku` for simple subtasks
3. Add progress checkpoints in instructions
4. Parallelize independent work with Task tool

## Advanced Topics

### Agent Composition

Combine multiple agents for complex workflows:

**Meta-Agent Pattern**:
```yaml
---
name: fullstack-feature-orchestrator
description: "Orchestrates multiple specialists for fullstack features"
model: sonnet
tools: [Task, TodoWrite]
---

## Instructions

1. **Decompose Feature**
   - Break into backend, frontend, testing, docs tasks

2. **Spawn Specialist Agents**
   - Launch backend-api-specialist
   - Launch frontend-component-specialist
   - Launch testing-specialist
   - Launch documentation-writer

3. **Coordinate Execution**
   - Monitor agent progress
   - Handle dependencies
   - Aggregate results
```

### MCP Server Integration

Add custom MCP servers for specialized data access:

```yaml
mcp_servers:
  - abathur-memory
  - abathur-task-queue
  - custom-gitlab-api  # Custom MCP for GitLab integration
  - custom-jira-api    # Custom MCP for Jira integration
```

Agents can then use MCP tools in instructions:
```markdown
3. **Fetch Issue Details**
   - Use mcp__custom-jira-api__get_issue
   - Extract requirements and acceptance criteria
```

### Agent Performance Tuning

**Model Selection for Performance**:

```yaml
# Fast, low-cost for simple tasks
model: haiku  # <2s response, $0.25/M tokens

# Balanced for most tasks
model: sonnet  # 3-5s response, $3/M tokens

# Maximum capability for complex tasks
model: opus  # 8-15s response, $15/M tokens
```

**Optimize Tool Usage**:
- Use `Grep` instead of `Read` for searching
- Use `Glob` instead of `Bash ls/find` for file discovery
- Batch multiple `Read` operations in parallel
- Use `MultiEdit` for multi-file changes

**Reduce Token Usage**:
- Keep instructions concise but clear
- Limit context loading to necessary data
- Use memory selectively (not entire task history)
- Summarize large outputs before storing

### Agent Version Management

Track agent versions for reproducibility:

```yaml
---
name: rust-testing-specialist
version: "2.1.0"  # Semantic versioning
description: "..."
changelog:
  - "2.1.0: Added property testing with proptest"
  - "2.0.0: Complete rewrite for async testing"
  - "1.0.0: Initial release"
---
```

Reference specific versions in tasks:
```bash
abathur task enqueue \
  --agent-type rust-testing-specialist \
  --agent-version "2.1.0" \
  --summary "Write tests"
```

## Related Documentation

- [Agent Reference](../reference/agents.md) - Complete list of built-in agents
- [Understanding Agent Orchestration](../explanation/agent-orchestration.md) - How agents work together
- [Tutorial: Your First Task](../tutorials/first-task.md) - Learn to use existing agents
- [Configuration Reference](../reference/configuration.md) - Configure agent behavior
