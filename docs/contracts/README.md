# Inter-Functional Contracts

This directory documents the contracts between services, event handlers, and
workflows in the Abathur swarm system. The goal is to make implicit
expectations explicit so that callers and callees agree on preconditions,
postconditions, error semantics, and concurrency guarantees.

## Documents

| Document | Scope |
|----------|-------|
| [event-bus.md](event-bus.md) | Event bus architecture, event envelope, publish/subscribe semantics, persistence, and safety mechanisms |
| [event-catalog.md](event-catalog.md) | Complete catalog of every `EventPayload` variant: who emits it, who handles it, and what invariants hold |
| [task-lifecycle.md](task-lifecycle.md) | Task state machine, `TaskService` method contracts, optimistic locking, and retry semantics |
| [workflow-engine.md](workflow-engine.md) | Workflow state machine, phase advancement, verification loops, fan-out/aggregation, and gate verdicts |
| [convergence-engine.md](convergence-engine.md) | Convergence loop contracts: trajectory lifecycle, strategy selection, overseer measurement, and budget management |
| [service-dependencies.md](service-dependencies.md) | Cross-service dependency map, shared-state inventory, and concurrency boundaries |
| [error-catalog.md](error-catalog.md) | `DomainError` variants, when each is raised, and how callers should handle them |

## How to Read These Docs

Each contract document follows a consistent structure:

- **Preconditions** — what must be true before calling
- **Postconditions** — what is guaranteed after a successful call
- **Events emitted** — which `EventPayload` variants are published
- **Errors** — which `DomainError` variants can be returned and why
- **Concurrency** — locking strategy, conflict handling, idempotency guarantees
- **Caller responsibilities** — what the caller must do with the result

## Conventions

- "Terminal state" means `Complete`, `Failed` (retries exhausted), or `Canceled`
  for tasks; `Completed`, `Rejected`, or `Failed` for workflows.
- "Idempotent" means calling the operation twice with the same input produces
  the same observable outcome (no duplicate events, no state corruption).
- Version numbers in this directory refer to the code as of the commit that
  introduced these docs. Contracts should be updated when the code changes.
