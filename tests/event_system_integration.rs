//! Integration tests for the event-driven architecture.
//!
//! Tests verify:
//! 1. Service mutations emit events to the EventBus and persist them to the EventStore
//! 2. EventReactor dispatches events to matching handlers
//! 3. TriggerRuleEngine evaluates rules and fires actions
//! 4. Replay missed events from the store catches up handlers

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use abathur::adapters::sqlite::{
    create_migrated_test_pool, SqliteEventRepository, SqliteGoalRepository,
    SqliteMemoryRepository, SqliteTaskRepository,
};
use abathur::domain::models::{GoalPriority, TaskPriority, TaskSource};
use abathur::services::command_bus::{
    CommandBus, CommandEnvelope, CommandResult, CommandSource, DomainCommand,
    GoalCommandHandler, MemoryCommandHandler, TaskCommand, TaskCommandHandler,
};
use abathur::services::event_bus::{
    EventBus, EventBusConfig, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber,
    UnifiedEvent,
};
use abathur::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, EventReactor, HandlerContext, HandlerId,
    HandlerMetadata, HandlerPriority, Reaction, ReactorConfig,
};
use abathur::services::event_store::{EventQuery, EventStore, InMemoryEventStore};
use abathur::services::trigger_rules::{
    SerializableEventFilter, TriggerAction, TriggerCondition, TriggerEventPayload, TriggerRule,
    TriggerRuleEngine,
};
use abathur::services::{GoalService, MemoryService, TaskService};

// ---------------------------------------------------------------------------
// Test helper: counting handler
// ---------------------------------------------------------------------------

struct CountingHandler {
    name: String,
    counter: Arc<AtomicU32>,
    filter: EventFilter,
}

impl CountingHandler {
    fn new(name: &str, filter: EventFilter) -> (Self, Arc<AtomicU32>) {
        let counter = Arc::new(AtomicU32::new(0));
        (
            Self {
                name: name.to_string(),
                counter: counter.clone(),
                filter,
            },
            counter,
        )
    }
}

#[async_trait]
impl EventHandler for CountingHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: self.name.clone(),
            filter: EventFilter {
                categories: self.filter.categories.clone(),
                min_severity: self.filter.min_severity,
                goal_id: self.filter.goal_id,
                task_id: self.filter.task_id,
                payload_types: self.filter.payload_types.clone(),
                custom_predicate: None,
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(Reaction::None)
    }
}

/// Small delay to let a spawned reactor task subscribe to the broadcast channel.
async fn wait_for_reactor_startup() {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

// ---------------------------------------------------------------------------
// Test 1: TaskService mutation emits event to EventBus and persists to store
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_task_mutation_emits_and_persists_event() {
    let pool = create_migrated_test_pool().await.expect("test pool");
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let event_store: Arc<dyn EventStore> = Arc::new(SqliteEventRepository::new(pool.clone()));

    let event_bus = Arc::new(
        EventBus::new(EventBusConfig {
            persist_events: true,
            ..Default::default()
        })
        .with_store(event_store.clone()),
    );

    let service = TaskService::new(task_repo, event_bus.clone());

    let task = service
        .submit_task(
            Some("Integration test task".to_string()),
            "Test description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
        )
        .await
        .expect("submit task");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = event_store
        .query(EventQuery::new().task_id(task.id).ascending())
        .await
        .expect("query events");

    assert!(!events.is_empty(), "Expected at least one event for task");

    let first = &events[0];
    match &first.payload {
        EventPayload::TaskSubmitted { task_id, task_title, .. } => {
            assert_eq!(*task_id, task.id);
            assert_eq!(task_title, "Integration test task");
        }
        other => panic!("Expected TaskSubmitted, got {:?}", other),
    }

    let ready_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(&e.payload, EventPayload::TaskReady { .. }))
        .collect();
    assert!(
        !ready_events.is_empty(),
        "Expected TaskReady event for task with no dependencies"
    );
}

// ---------------------------------------------------------------------------
// Test 2: GoalService mutation emits event and persists to store
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_goal_mutation_emits_and_persists_event() {
    let pool = create_migrated_test_pool().await.expect("test pool");
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let event_store: Arc<dyn EventStore> = Arc::new(SqliteEventRepository::new(pool.clone()));

    let event_bus = Arc::new(
        EventBus::new(EventBusConfig {
            persist_events: true,
            ..Default::default()
        })
        .with_store(event_store.clone()),
    );

    let service = GoalService::new(goal_repo, event_bus.clone());

    let goal = service
        .create_goal(
            "Test goal".to_string(),
            "A test goal description".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("create goal");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = event_store
        .query(EventQuery::new().goal_id(goal.id).ascending())
        .await
        .expect("query events");

    assert!(!events.is_empty(), "Expected at least one event for goal");

    match &events[0].payload {
        EventPayload::GoalStarted { goal_id, goal_name } => {
            assert_eq!(*goal_id, goal.id);
            assert_eq!(goal_name, "Test goal");
        }
        other => panic!("Expected GoalStarted, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test 3: EventReactor dispatches to matching handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reactor_dispatches_to_handler() {
    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let reactor = EventReactor::new(event_bus.clone(), ReactorConfig::default());

    let (handler, counter) = CountingHandler::new(
        "task-counter",
        EventFilter::new().payload_types(vec!["TaskSubmitted".to_string()]),
    );
    reactor.register(Arc::new(handler)).await;

    let handle = reactor.start();
    wait_for_reactor_startup().await;

    // Publish a matching TaskSubmitted event
    let event = UnifiedEvent {
        id: EventId::new(),
        sequence: SequenceNumber(0),
        timestamp: chrono::Utc::now(),
        severity: EventSeverity::Info,
        category: EventCategory::Task,
        goal_id: None,
        task_id: Some(Uuid::new_v4()),
        correlation_id: None,
        source_process_id: None,
        payload: EventPayload::TaskSubmitted {
            task_id: Uuid::new_v4(),
            task_title: "test task".to_string(),
            goal_id: Uuid::nil(),
        },
    };
    event_bus.publish(event).await;

    // Publish a non-matching event
    let event2 = UnifiedEvent {
        id: EventId::new(),
        sequence: SequenceNumber(0),
        timestamp: chrono::Utc::now(),
        severity: EventSeverity::Info,
        category: EventCategory::Orchestrator,
        goal_id: None,
        task_id: None,
        correlation_id: None,
        source_process_id: None,
        payload: EventPayload::OrchestratorStarted,
    };
    event_bus.publish(event2).await;

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    reactor.stop();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;

    assert_eq!(counter.load(Ordering::SeqCst), 1, "Handler should have been called exactly once");
}

// ---------------------------------------------------------------------------
// Test 4: Trigger rule matches event and emits action event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_trigger_rule_emits_event_on_match() {
    let pool = create_migrated_test_pool().await.expect("test pool");

    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = Arc::new(TaskService::new(task_repo, event_bus.clone()));
    let goal_service = Arc::new(GoalService::new(goal_repo, event_bus.clone()));
    let memory_service = Arc::new(MemoryService::new_with_event_bus(memory_repo, event_bus.clone()));

    let command_bus = Arc::new(CommandBus::new(
        task_service as Arc<dyn TaskCommandHandler>,
        goal_service as Arc<dyn GoalCommandHandler>,
        memory_service as Arc<dyn MemoryCommandHandler>,
    ));

    let engine = TriggerRuleEngine::new(command_bus).with_event_bus(event_bus.clone());

    // Create a rule: on TaskCompleted, emit a ScheduledEventFired
    let rule = TriggerRule::new(
        "task-complete-trigger",
        SerializableEventFilter {
            categories: vec![],
            min_severity: None,
            payload_types: vec!["TaskCompleted".to_string()],
            goal_id: None,
            task_id: None,
        },
        TriggerAction::EmitEvent {
            payload: TriggerEventPayload::ScheduledEventFired {
                name: "post-completion-hook".to_string(),
            },
            category: EventCategory::Scheduler,
            severity: EventSeverity::Info,
        },
    );
    engine.add_rule(rule).await;

    let reactor = EventReactor::new(event_bus.clone(), ReactorConfig::default());

    let (counter_handler, counter) = CountingHandler::new(
        "post-completion-counter",
        EventFilter::new().payload_types(vec!["ScheduledEventFired".to_string()]),
    );
    reactor.register(Arc::new(engine)).await;
    reactor.register(Arc::new(counter_handler)).await;

    let handle = reactor.start();
    wait_for_reactor_startup().await;

    // Publish a TaskCompleted event
    let event = UnifiedEvent {
        id: EventId::new(),
        sequence: SequenceNumber(0),
        timestamp: chrono::Utc::now(),
        severity: EventSeverity::Info,
        category: EventCategory::Task,
        goal_id: None,
        task_id: Some(Uuid::new_v4()),
        correlation_id: None,
        source_process_id: None,
        payload: EventPayload::TaskCompleted {
            task_id: Uuid::new_v4(),
            tokens_used: 100,
        },
    };
    event_bus.publish(event).await;

    // Wait for the chain: TaskCompleted → trigger engine emits ScheduledEventFired → counter handler
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    reactor.stop();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;

    assert!(
        counter.load(Ordering::SeqCst) >= 1,
        "Counter handler should have received the ScheduledEventFired event emitted by trigger rule (got {})",
        counter.load(Ordering::SeqCst),
    );
}

// ---------------------------------------------------------------------------
// Test 5: Replay missed events from store catches up handlers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_replay_missed_events_catches_up() {
    let store: Arc<dyn EventStore> = Arc::new(InMemoryEventStore::new());

    // Pre-populate the store with events at sequences 1..=5.
    // Replay uses watermark 0 (no prior processing) and skips events where
    // sequence <= watermark, so sequence 0 would be skipped. Start from 1.
    let task_id = Uuid::new_v4();
    for i in 1..=5u64 {
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(i),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskSubmitted {
                task_id,
                task_title: format!("task-{}", i),
                goal_id: Uuid::nil(),
            },
        };
        store.append(&event).await.expect("append event");
    }

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let reactor = EventReactor::new(event_bus.clone(), ReactorConfig::default())
        .with_store(store.clone());

    let (handler, counter) = CountingHandler::new(
        "replay-counter",
        EventFilter::new().payload_types(vec!["TaskSubmitted".to_string()]),
    );
    reactor.register(Arc::new(handler)).await;

    let replayed = reactor.replay_missed_events().await.expect("replay");

    assert_eq!(replayed, 5, "Should have replayed all 5 events");
    assert_eq!(counter.load(Ordering::SeqCst), 5, "Handler should have processed all 5 events");
}

// ---------------------------------------------------------------------------
// Test 6: CommandBus routes TaskCommand through service and emits events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_command_bus_routes_and_emits() {
    let pool = create_migrated_test_pool().await.expect("test pool");
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
    let event_store: Arc<dyn EventStore> = Arc::new(SqliteEventRepository::new(pool.clone()));

    let event_bus = Arc::new(
        EventBus::new(EventBusConfig {
            persist_events: true,
            ..Default::default()
        })
        .with_store(event_store.clone()),
    );

    let task_service = Arc::new(TaskService::new(task_repo, event_bus.clone()));
    let goal_service = Arc::new(GoalService::new(goal_repo, event_bus.clone()));
    let memory_service = Arc::new(MemoryService::new_with_event_bus(memory_repo, event_bus.clone()));

    let command_bus = CommandBus::new(
        task_service as Arc<dyn TaskCommandHandler>,
        goal_service as Arc<dyn GoalCommandHandler>,
        memory_service as Arc<dyn MemoryCommandHandler>,
    );

    let envelope = CommandEnvelope::new(
        CommandSource::Human,
        DomainCommand::Task(TaskCommand::Submit {
            title: Some("CommandBus test".to_string()),
            description: "Testing command bus routing".to_string(),
            parent_id: None,
            priority: TaskPriority::Normal,
            agent_type: None,
            depends_on: vec![],
            context: Box::new(None),
            idempotency_key: None,
            source: TaskSource::Human,
        }),
    );

    let result = command_bus.dispatch(envelope).await.expect("dispatch");
    let task = match result {
        CommandResult::Task(t) => t,
        other => panic!("Expected Task result, got {:?}", other),
    };

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = event_store
        .query(EventQuery::new().task_id(task.id).ascending())
        .await
        .expect("query");

    assert!(!events.is_empty(), "CommandBus routed task should emit events");
    assert!(
        events.iter().any(|e| matches!(&e.payload, EventPayload::TaskSubmitted { .. })),
        "Should contain TaskSubmitted event"
    );
}

// ---------------------------------------------------------------------------
// Test 7: End-to-end: service mutation → persist → reactor handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_e2e_mutation_persist_react() {
    let pool = create_migrated_test_pool().await.expect("test pool");
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let event_store: Arc<dyn EventStore> = Arc::new(SqliteEventRepository::new(pool.clone()));

    let event_bus = Arc::new(
        EventBus::new(EventBusConfig {
            persist_events: true,
            ..Default::default()
        })
        .with_store(event_store.clone()),
    );

    let reactor = EventReactor::new(event_bus.clone(), ReactorConfig::default());
    let (handler, counter) = CountingHandler::new(
        "e2e-task-handler",
        EventFilter::new().categories(vec![EventCategory::Task]),
    );
    reactor.register(Arc::new(handler)).await;
    let handle = reactor.start();
    wait_for_reactor_startup().await;

    let service = TaskService::new(task_repo, event_bus.clone());
    let _task = service
        .submit_task(
            Some("E2E test task".to_string()),
            "End to end test".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
        )
        .await
        .expect("submit task");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    reactor.stop();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;

    let count = counter.load(Ordering::SeqCst);
    assert!(
        count >= 2,
        "Expected at least 2 task events (TaskSubmitted + TaskReady), got {}",
        count
    );

    let stored = event_store
        .query(EventQuery::new().category(EventCategory::Task).ascending())
        .await
        .expect("query store");
    assert!(
        stored.len() >= 2,
        "Expected at least 2 persisted task events, got {}",
        stored.len()
    );
}

// ---------------------------------------------------------------------------
// Test 8: SQLite watermark tracking
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_watermark_tracking() {
    let pool = create_migrated_test_pool().await.expect("test pool");
    let store = SqliteEventRepository::new(pool);

    // No watermark initially
    let wm = store.get_watermark("test-handler").await.expect("get wm");
    assert_eq!(wm, None);

    // Set watermark
    store
        .set_watermark("test-handler", SequenceNumber(42))
        .await
        .expect("set wm");

    let wm = store.get_watermark("test-handler").await.expect("get wm");
    assert_eq!(wm, Some(SequenceNumber(42)));

    // Update watermark
    store
        .set_watermark("test-handler", SequenceNumber(100))
        .await
        .expect("set wm");

    let wm = store.get_watermark("test-handler").await.expect("get wm");
    assert_eq!(wm, Some(SequenceNumber(100)));
}

// ---------------------------------------------------------------------------
// Test 9: Trigger rule with CountThreshold condition
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_trigger_count_threshold() {
    let pool = create_migrated_test_pool().await.expect("test pool");
    let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
    let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
    let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));

    let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
    let task_service = Arc::new(TaskService::new(task_repo, event_bus.clone()));
    let goal_service = Arc::new(GoalService::new(goal_repo, event_bus.clone()));
    let memory_service = Arc::new(MemoryService::new_with_event_bus(memory_repo, event_bus.clone()));

    let command_bus = Arc::new(CommandBus::new(
        task_service as Arc<dyn TaskCommandHandler>,
        goal_service as Arc<dyn GoalCommandHandler>,
        memory_service as Arc<dyn MemoryCommandHandler>,
    ));

    let engine = TriggerRuleEngine::new(command_bus).with_event_bus(event_bus.clone());

    // Rule: fire only after 3 TaskFailed events within 60 seconds
    let rule = TriggerRule::new(
        "failure-threshold",
        SerializableEventFilter {
            categories: vec![],
            min_severity: None,
            payload_types: vec!["TaskFailed".to_string()],
            goal_id: None,
            task_id: None,
        },
        TriggerAction::EmitEvent {
            payload: TriggerEventPayload::HumanEscalation {
                reason: "Too many task failures".to_string(),
            },
            category: EventCategory::Escalation,
            severity: EventSeverity::Warning,
        },
    )
    .with_condition(TriggerCondition::CountThreshold {
        count: 3,
        window_secs: 60,
    });
    engine.add_rule(rule).await;

    let reactor = EventReactor::new(event_bus.clone(), ReactorConfig::default());

    let (counter_handler, counter) = CountingHandler::new(
        "escalation-counter",
        EventFilter::new().payload_types(vec!["HumanEscalationNeeded".to_string()]),
    );
    reactor.register(Arc::new(engine)).await;
    reactor.register(Arc::new(counter_handler)).await;

    let handle = reactor.start();
    wait_for_reactor_startup().await;

    // Send 2 failures — should NOT trigger
    for _ in 0..2 {
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(Uuid::new_v4()),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: Uuid::new_v4(),
                error: "test error".to_string(),
                retry_count: 3,
            },
        };
        event_bus.publish(event).await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "Should not trigger below threshold");

    // Send 3rd failure — should trigger
    let event = UnifiedEvent {
        id: EventId::new(),
        sequence: SequenceNumber(0),
        timestamp: chrono::Utc::now(),
        severity: EventSeverity::Error,
        category: EventCategory::Task,
        goal_id: None,
        task_id: Some(Uuid::new_v4()),
        correlation_id: None,
        source_process_id: None,
        payload: EventPayload::TaskFailed {
            task_id: Uuid::new_v4(),
            error: "test error 3".to_string(),
            retry_count: 3,
        },
    };
    event_bus.publish(event).await;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    reactor.stop();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;

    assert!(
        counter.load(Ordering::SeqCst) >= 1,
        "Threshold trigger should fire after 3 failures (got {})",
        counter.load(Ordering::SeqCst),
    );
}
