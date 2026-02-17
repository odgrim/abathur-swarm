//! Centralized event construction helpers.
//!
//! Eliminates the repetitive boilerplate of building `UnifiedEvent` structs
//! (EventId::new(), SequenceNumber(0), Utc::now(), etc.) scattered across services.
//! All event construction should go through these helpers.

use chrono::Utc;
use uuid::Uuid;

use super::event_bus::{
    EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};

/// Build a `UnifiedEvent` with standard defaults.
///
/// Sequence is set to 0 (assigned by EventBus on publish).
/// Timestamp is set to `Utc::now()`.
/// Correlation ID and source_process_id are None.
pub fn make_event(
    severity: EventSeverity,
    category: EventCategory,
    goal_id: Option<Uuid>,
    task_id: Option<Uuid>,
    payload: EventPayload,
) -> UnifiedEvent {
    UnifiedEvent {
        id: EventId::new(),
        sequence: SequenceNumber(0),
        timestamp: Utc::now(),
        severity,
        category,
        goal_id,
        task_id,
        correlation_id: None,
        source_process_id: None,
        payload,
    }
}

/// Build a task-category event.
pub fn task_event(
    severity: EventSeverity,
    goal_id: Option<Uuid>,
    task_id: Uuid,
    payload: EventPayload,
) -> UnifiedEvent {
    make_event(severity, EventCategory::Task, goal_id, Some(task_id), payload)
}

/// Build a goal-category event.
pub fn goal_event(
    severity: EventSeverity,
    goal_id: Uuid,
    payload: EventPayload,
) -> UnifiedEvent {
    make_event(severity, EventCategory::Goal, Some(goal_id), None, payload)
}

/// Build a memory-category event.
pub fn memory_event(severity: EventSeverity, payload: EventPayload) -> UnifiedEvent {
    make_event(severity, EventCategory::Memory, None, None, payload)
}

/// Build an orchestrator-category event.
pub fn orchestrator_event(severity: EventSeverity, payload: EventPayload) -> UnifiedEvent {
    make_event(severity, EventCategory::Orchestrator, None, None, payload)
}

/// Build an agent-category event.
pub fn agent_event(
    severity: EventSeverity,
    task_id: Option<Uuid>,
    payload: EventPayload,
) -> UnifiedEvent {
    make_event(severity, EventCategory::Agent, None, task_id, payload)
}

/// Build a workflow-category event.
pub fn workflow_event(
    severity: EventSeverity,
    goal_id: Option<Uuid>,
    payload: EventPayload,
) -> UnifiedEvent {
    make_event(severity, EventCategory::Workflow, goal_id, None, payload)
}
