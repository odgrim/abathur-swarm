# Explanations

This section provides understanding-oriented documentation that clarifies concepts, explores design decisions, and explains the "why" behind Abathur Swarm's architecture and implementation. These articles help you build a deeper, conceptual understanding of how the system works.

Explanations focus on theory, background, and context rather than practical instructions. They discuss trade-offs, alternatives, and the reasoning behind technical choices, helping you understand not just what Abathur does, but why it does it that way.

## Available Explanations

### Core Concepts
- **[Understanding the Task Queue](task-queue.md)**: How tasks are scheduled, prioritized, and executed *(coming soon)*
- **[Agent Orchestration Model](orchestration.md)**: The philosophy and patterns behind agent coordination *(coming soon)*
- **[Memory System Architecture](memory-system.md)**: How semantic, episodic, and procedural memory work together *(coming soon)*
- **[Dependency Management](dependencies.md)**: Why and how task dependencies enable complex workflows *(coming soon)*

### Architecture and Design
- **[System Architecture](architecture.md)**: High-level design and component interactions *(coming soon)*
- **[Clean Architecture Principles](clean-architecture.md)**: How hexagonal architecture shapes the codebase *(coming soon)*
- **[Async Runtime Design](async-runtime.md)**: Tokio-based concurrency patterns and trade-offs *(coming soon)*
- **[Database Layer](database-design.md)**: SQLite with WAL mode for concurrent access *(coming soon)*

### Design Rationale
- **[Why Rust?](why-rust.md)**: Language choice and benefits for agentic systems *(coming soon)*
- **[MCP Integration Strategy](mcp-integration.md)**: Model Context Protocol design decisions *(coming soon)*
- **[Configuration Approach](configuration-design.md)**: Hierarchical YAML with environment overrides *(coming soon)*
- **[Testing Philosophy](testing-philosophy.md)**: Unit, integration, and property testing strategy *(coming soon)*

### Patterns and Best Practices
- **[Task Decomposition Patterns](task-patterns.md)**: Common ways to structure complex workflows *(coming soon)*
- **[Agent Specialization](agent-patterns.md)**: When and how to create specialized agents *(coming soon)*
- **[Error Handling Strategy](error-handling.md)**: Failure modes and recovery patterns *(coming soon)*
- **[Concurrency Patterns](concurrency-patterns.md)**: Managing parallel task execution *(coming soon)*

## Diátaxis Framework Context

**Explanations are understanding-oriented**: They help you build mental models and understand concepts at a deeper level. Unlike tutorials (learning by doing) or how-to guides (solving problems), explanations focus on clarity and comprehension.

**Key Characteristics**:
- **Conceptual depth**: Explores ideas, not procedures
- **Context and background**: Provides historical and theoretical foundations
- **Design rationale**: Explains why things are the way they are
- **Trade-offs and alternatives**: Discusses different approaches and their implications
- **No instructions**: Focuses on understanding, not doing
- **Connections**: Links concepts to broader principles and patterns

**When to Use Explanations**:
- You want to understand the reasoning behind design decisions
- You need context for why a feature works a certain way
- You're curious about alternatives and trade-offs
- You want to deepen your conceptual understanding
- You're evaluating whether Abathur fits your needs
- You want to contribute and need architectural context

## Understanding vs. Doing

**Explanations answer "why"**:
- Why does Abathur use a task queue instead of direct execution?
- Why SQLite with WAL mode rather than PostgreSQL?
- Why specialized agents rather than general-purpose LLMs?

**Other sections answer different questions**:
- **Tutorials** answer "how do I learn": Step-by-step guided practice
- **How-To Guides** answer "how do I solve": Problem-focused recipes
- **Reference** answers "what exactly": Precise technical specifications

## Learning Path

For deepest understanding, read explanations in this order:

1. **Start with fundamentals**: [Task Queue](task-queue.md), [Orchestration Model](orchestration.md)
2. **Understand architecture**: [System Architecture](architecture.md), [Clean Architecture](clean-architecture.md)
3. **Explore design decisions**: [Why Rust?](why-rust.md), [MCP Integration](mcp-integration.md)
4. **Learn patterns**: [Task Patterns](task-patterns.md), [Agent Patterns](agent-patterns.md)

## Using Diagrams

Many explanations include visual diagrams to illustrate concepts:
- **Architecture diagrams**: Show component relationships and data flow
- **Sequence diagrams**: Illustrate interactions over time
- **State diagrams**: Clarify task lifecycle and transitions
- **Flowcharts**: Explain decision logic and processes

These visualizations complement the text and help build intuition about system behavior.

## Related Documentation

**If you need to**... **then consult**:
- Learn by practicing → [Tutorials](../tutorials/index.md)
- Solve specific problems → [How-To Guides](../how-to/index.md)
- Look up syntax or specs → [Reference](../reference/index.md)
- Understand concepts deeply → Explanations (this section)

## Contributing

Help improve conceptual understanding:
- Clarify confusing explanations
- Add diagrams to illustrate concepts
- Expand on design rationale
- Connect concepts across articles
- Explain trade-offs and alternatives

See [Contributing Guide](../contributing/index.md) for details on writing explanation documentation.

---

*Curious about a specific topic? Browse the articles above or start with [System Architecture](architecture.md) for a comprehensive overview.*
