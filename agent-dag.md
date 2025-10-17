# Abathur Agent Workflow DAG

## Visual Representation

```
                                    ┌─────────────────────────┐
                                    │  User Request/Task      │
                                    └───────────┬─────────────┘
                                                │
                                                ▼
                                    ┌─────────────────────────┐
                                    │  requirements-gatherer  │
                                    │  (Entry Point)          │
                                    │  • Analyzes user needs  │
                                    │  • Stores in memory     │
                                    └───────────┬─────────────┘
                                                │
                                                │ spawns
                                                │ memory: task:{id}:requirements
                                                ▼
                                    ┌─────────────────────────┐
                                    │  technical-architect    │
                                    │  (Design Phase)         │
                                    │  • System architecture  │
                                    │  • Decomposition        │
                                    └───────────┬─────────────┘
                                                │
                            ┌───────────────────┴────────────────────┐
                            │                                        │
                        Single Path                            Multi-Path
                            │                                        │
                            ▼                                        ▼
        ┌──────────────────────────────┐         ┌─────────────────────────────────┐
        │ technical-requirements-      │         │ technical-requirements-         │
        │ specialist                   │         │ specialist (×N)                 │
        │ (Spec Phase)                 │         │ (1 per subproject)              │
        │ • Tech specs                 │         │ • Tech specs                    │
        │ • Suggested agents           │         │ • Suggested agents              │
        └──────────────┬───────────────┘         └────────────┬────────────────────┘
                       │                                       │
                       │ spawns                                │ spawns (each)
                       │ memory: task:{id}:technical_specs     │
                       └───────────────┬───────────────────────┘
                                       │
                                       ▼
                           ┌─────────────────────────┐
                           │  task-planner           │
                           │  (Orchestration)        │
                           │  • Identify agents      │
                           │  • Decompose tasks      │
                           │  • Set dependencies     │
                           └───────┬─────────────────┘
                                   │
                    ┌──────────────┴──────────────┐
                    │                             │
            Missing agents?                   Create tasks
                    │                             │
                    ▼                             ▼
        ┌─────────────────────┐     ┌──────────────────────────┐
        │  agent-creator      │     │ Implementation Tasks     │
        │  (Agent Gen)        │     │ (Multiple)               │
        │  • Create .md files │     │ • Assigned to agents     │
        │  • Register agents  │     │ • With dependencies      │
        └──────────┬──────────┘     │ • In git worktrees       │
                   │                └────────────┬─────────────┘
                   │                             │
                   │ creates agents              │ depends on
                   │ writes to:                  │ agent-creator
                   │ .claude/agents/workers/     │ (if needed)
                   └──────────┬──────────────────┘
                              │
                              ▼
                   ┌────────────────────────┐
                   │ Specialized Agents     │
                   │ Execute Tasks          │
                   │ • Parallel when        │
                   │   independent          │
                   │ • Sequential when      │
                   │   dependent            │
                   └────────────────────────┘

                        (Separate Workflow)
                              │
                   ┌──────────┴──────────┐
                   │ Issue Identified    │
                   │ with Agent Behavior │
                   └──────────┬──────────┘
                              │
                              ▼
                   ┌─────────────────────────┐
                   │ swarm-enhancement-agent │
                   │ (Maintenance)           │
                   │ • Fix agent issues      │
                   │ • Update templates      │
                   │ • Systematic updates    │
                   └─────────────────────────┘
```

## Agent Dependency Graph

```
Legend:
  ──▶  Spawns/Creates (task_enqueue)
  ···▶ Reads from memory
  ═══▶ Writes to memory
  - -▶ Optional/Conditional

┌─────────────────────┐
│ requirements-       │══════════════════════════════════════╗
│ gatherer            │                                      ║
└──────────┬──────────┘                                      ║
           │                                                 ║
           │ spawns                                          ║
           ▼                                                 ║
┌─────────────────────┐                                      ║
│ technical-          │·····································╝
│ architect           │══════════════════════════════════╗
└──────────┬──────────┘                                  ║
           │                                             ║
           │ spawns (1 or N)                             ║
           ▼                                             ║
┌─────────────────────┐                                  ║
│ technical-          │·····································╝
│ requirements-       │══════════════════════════════╗
│ specialist          │                              ║
└──────────┬──────────┘                              ║
           │                                         ║
           │ spawns                                  ║
           ▼                                         ║
┌─────────────────────┐                              ║
│ task-planner        │·····························╝
└──────┬───────┬──────┘
       │       │
       │       └──────┐
       │              │ spawns (conditional)
       │              ▼
       │      ┌─────────────────────┐
       │      │ agent-creator       │═══════════════╗
       │      └──────────┬──────────┘               ║
       │                 │                          ║
       │                 │ creates agents           ║
       │                 ▼                          ║
       │      ┌─────────────────────┐               ║
       │      │ .claude/agents/     │◀══════════════╝
       │      │ workers/            │
       │      └─────────────────────┘
       │                 │
       │                 │ agent files written
       │                 ▼
       │      ┌─────────────────────┐
       │      │ agents:registry     │
       │      │ (memory)            │
       │      └──────────┬──────────┘
       │                 │
       │  spawns         │ prereq for impl tasks
       │                 │
       └────────┬────────┘
                ▼
     ┌─────────────────────┐
     │ Implementation      │
     │ Tasks               │
     │ (specialized agents)│
     └─────────────────────┘


     (Independent)
     ┌─────────────────────┐
     │ swarm-enhancement-  │
     │ agent               │
     │ (invoked separately)│
     └─────────────────────┘
```

## Memory Flow

```
requirements-gatherer
    ║
    ║ writes memory
    ║
    ╚════▶ task:{task_id}:requirements
                  │
                  │ reads
                  ▼
         technical-architect
                  ║
                  ║ writes memory
                  ║
                  ╚════▶ task:{arch_task_id}:architecture
                                │
                                │ reads
                                ▼
                    technical-requirements-specialist
                                ║
                                ║ writes memory
                                ║
                                ╚════▶ task:{spec_task_id}:technical_specs
                                              │
                                              │ reads
                                              ▼
                                         task-planner
                                              ║
                                              ║ checks/reads
                                              ║
                                              ╠════▶ agents:registry
                                              ║           ▲
                                              ║           │ writes
                                              ║           │
                                              ║      agent-creator
                                              ║
                                              ║ spawns tasks with context
                                              ▼
                                    Implementation Tasks
```

## Task Queue Dependencies

```
Task Hierarchy (prerequisite_task_ids):

[T1] requirements-gatherer
  │   prerequisite_task_ids: []
  │
  └──▶ [T2] technical-architect
        │   prerequisite_task_ids: [T1]
        │
        ├──▶ [T3a] technical-requirements-specialist (subproject A)
        │     │   prerequisite_task_ids: [T2]
        │     │
        │     └──▶ [T4a] task-planner (subproject A)
        │           │   prerequisite_task_ids: [T3a]
        │           │
        │           ├──▶ [T5a1] agent-creator (agent X)
        │           │     │   prerequisite_task_ids: [T4a]
        │           │     │
        │           │     └──▶ [T6a1] impl-task-1
        │           │           │   prerequisite_task_ids: [T5a1]
        │           │           │
        │           │           └──▶ [T6a2] impl-task-2
        │           │                 prerequisite_task_ids: [T6a1]
        │           │
        │           └──▶ [T5a2] agent-creator (agent Y)
        │                 │   prerequisite_task_ids: [T4a]
        │                 │
        │                 └──▶ [T6a3] impl-task-3
        │                       prerequisite_task_ids: [T5a2]
        │
        └──▶ [T3b] technical-requirements-specialist (subproject B)
              │   prerequisite_task_ids: [T2]
              │
              └──▶ [T4b] task-planner (subproject B)
                    │   prerequisite_task_ids: [T3b]
                    │
                    └──▶ [T6b1] impl-task-4
                          prerequisite_task_ids: [T4b]
                          (uses existing agents)
```

## Key Characteristics

### Linear Sequential Pipeline
- requirements → architecture → specs → tasks
- Each stage completes before next begins
- Memory-based handoffs between stages

### Conditional Agent Creation
- task-planner checks existing agents
- Creates new agents only when needed
- Agent creation tasks block dependent implementation tasks

### Parallel Execution Opportunities
- Multiple technical-requirements-specialist tasks (when decomposed)
- Multiple agent-creator tasks (independent agent creation)
- Multiple implementation tasks (when dependencies allow)

### Memory-Based Coordination
- No direct agent-to-agent communication
- All context passed through namespaced memory
- Enables async/distributed execution

### Independent Maintenance
- swarm-enhancement-agent operates separately
- Fixes behavioral issues in existing agents
- Updates templates and active agents
