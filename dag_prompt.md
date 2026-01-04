# DAG-Based Workflow Execution for Prompt Chains

## Problem Statement

The current prompt chaining implementation in abathur-swarm uses strictly sequential execution where each step must complete before the next begins. The design docs (Chapters 1 and 3) specify support for parallel execution and graph-based workflows, but this is not yet implemented.

We need to evolve from sequential chains to a proper DAG (Directed Acyclic Graph) execution model that enables:
- Parallel execution of independent steps
- Convergence/join points where multiple branches merge
- Explicit dependency declarations between steps
- Efficient resource utilization via concurrent I/O

## Current State

### What Exists
- Sequential prompt chain execution (`src/domain/models/prompt_chain.rs`)
- Fan-out decomposition (one step spawning multiple child tasks)
- Variable substitution between steps
- Structured output validation (JSON schema)
- Chain execution service with retry logic
- YAML-based chain templates

### What's Missing
- `depends_on` field for explicit step dependencies
- Parallel step execution when dependencies allow
- Fan-in/join semantics to collect results from parallel branches
- `max_parallelism` configuration (specified in design but not implemented)
- Topological ordering of step execution
- Result aggregation at convergence points

## Requirements

### Functional Requirements

1. **Dependency Declaration**
   - Each `ChainStep` must support a `depends_on: Vec<String>` field listing step IDs it requires
   - Steps with no dependencies (or only completed dependencies) can execute in parallel
   - Circular dependency detection at chain validation time

2. **Parallel Execution**
   - Steps with satisfied dependencies execute concurrently up to `max_parallelism` limit
   - Respect existing semaphore-based concurrency control in swarm orchestrator
   - Maintain per-step execution isolation (separate contexts, worktrees if needed)

3. **Convergence/Join Points**
   - A step depending on multiple parent steps receives all parent outputs
   - Aggregated outputs available via templating: `{step_a.output}`, `{step_b.output}`
   - Support for synthesis prompts that combine parallel branch results

4. **Backward Compatibility**
   - Existing sequential chains (no `depends_on` specified) must continue working
   - Default behavior: step N implicitly depends on step N-1

5. **Execution Model**
   - Build topological order at chain start
   - Track step completion state
   - Fire ready steps as dependencies complete
   - Handle partial failures (some branches fail, others succeed)

### Non-Functional Requirements

1. **Performance**
   - Parallel I/O for LLM calls to reduce total latency
   - No polling; use event-driven step triggering

2. **Observability**
   - Log which steps are executing in parallel
   - Track per-step timing for bottleneck identification
   - Expose DAG structure in chain execution metadata

3. **Reliability**
   - Idempotent step execution (existing pattern)
   - Recovery from partial DAG execution failures
   - Persist DAG execution state for resumption

## Design Considerations

### Option A: Extend Current Task Model
- Each chain step becomes a Task with explicit `blocked_by` relationships
- Leverage existing `AwaitingChildren` state for join semantics
- Swarm orchestrator already handles task dependencies

### Option B: In-Process DAG Executor
- New executor that manages DAG state within a single chain execution
- Uses tokio for concurrent step execution
- Simpler but doesn't distribute across agents

### Option C: Hybrid Approach
- DAG logic in chain service determines which steps are ready
- Steps still spawn as Tasks for execution
- Chain service coordinates join points

## Reference: Design Doc Patterns

From Chapter 1 (Prompt Chaining):
> "Complex operations frequently combine parallel processing for independent data gathering with prompt chaining for the dependent steps of synthesis and refinement."

From Chapter 3 (Parallelization):
> "Parallel pathways execute independently before their results can be aggregated at a subsequent convergence point in the graph."

Example workflow from docs:
```
1. Search for Source A AND Search for Source B simultaneously
2. Once both complete, Summarize A AND Summarize B simultaneously
3. Synthesize final answer (sequential, waits for parallel steps)
```

## Example DAG Chain Definition

```yaml
name: research_and_synthesize
steps:
  - id: research_topic_a
    prompt: "Research {topic_a} and provide key findings..."
    output_format: json

  - id: research_topic_b
    prompt: "Research {topic_b} and provide key findings..."
    output_format: json

  - id: research_topic_c
    prompt: "Research {topic_c} and provide key findings..."
    output_format: json

  - id: synthesize
    depends_on: [research_topic_a, research_topic_b, research_topic_c]
    prompt: |
      Synthesize findings from multiple research branches:

      Topic A findings: {research_topic_a.output}
      Topic B findings: {research_topic_b.output}
      Topic C findings: {research_topic_c.output}

      Provide a unified analysis...
    output_format: markdown

max_parallelism: 3
```

## Constraints

- Must work with existing SQLite-based task queue
- Must integrate with existing agent executor and swarm orchestrator
- Rust async/tokio patterns required
- No external workflow engines (keep self-contained)

## Success Criteria

1. Chain with 3 independent steps completes in ~1x single-step time (not 3x)
2. Existing sequential chains pass all tests unchanged
3. DAG visualization available in execution metadata
4. Recovery works for partially-completed DAG executions
