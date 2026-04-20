use super::*;
use crate::services::event_factory::orchestrator_event;
use std::sync::Arc;
use uuid::Uuid;

    #[tokio::test]
    async fn test_event_bus_sequence_assignment() {
        let bus = EventBus::new(EventBusConfig::default());

        assert_eq!(bus.current_sequence().0, 0);

        let mut rx = bus.subscribe();

        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStarted,
        ))
        .await;
        let event1 = rx.recv().await.unwrap();
        assert_eq!(event1.sequence.0, 0);

        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStopped,
        ))
        .await;
        let event2 = rx.recv().await.unwrap();
        assert_eq!(event2.sequence.0, 1);

        assert_eq!(bus.current_sequence().0, 2);
    }

    #[tokio::test]
    async fn test_event_bus_correlation() {
        let bus = EventBus::new(EventBusConfig::default());
        let mut rx = bus.subscribe();

        // Event without correlation
        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStarted,
        ))
        .await;
        let event1 = rx.recv().await.unwrap();
        assert!(event1.correlation_id.is_none());

        // Start correlation
        let corr_id = bus.start_correlation().await;

        // Event with correlation
        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorPaused,
        ))
        .await;
        let event2 = rx.recv().await.unwrap();
        assert_eq!(event2.correlation_id, Some(corr_id));

        // End correlation
        bus.end_correlation().await;

        // Event without correlation again
        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStopped,
        ))
        .await;
        let event3 = rx.recv().await.unwrap();
        assert!(event3.correlation_id.is_none());
    }

    #[tokio::test]
    async fn test_swarm_event_conversion() {
        let event = SwarmEvent::TaskFailed {
            task_id: Uuid::new_v4(),
            error: "test error".to_string(),
            retry_count: 2,
        };

        let unified: UnifiedEvent = event.into();
        assert_eq!(unified.severity, EventSeverity::Error);
        assert_eq!(unified.category, EventCategory::Task);
        assert!(unified.task_id.is_some());
    }

    #[tokio::test]
    async fn test_execution_event_conversion() {
        let event = ExecutionEvent::WaveStarted {
            wave_number: 1,
            task_count: 5,
        };

        let unified: UnifiedEvent = event.into();
        assert_eq!(unified.severity, EventSeverity::Info);
        assert_eq!(unified.category, EventCategory::Execution);
    }

    #[tokio::test]
    async fn test_dropped_count_increments_on_no_receivers() {
        // Create bus with no subscribers — sends will be dropped
        let bus = EventBus::new(EventBusConfig {
            channel_capacity: 16,
            persist_events: false,
        });

        assert_eq!(bus.dropped_count(), 0);

        // Publish without any subscriber — should increment dropped_count
        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStarted,
        ))
        .await;
        assert_eq!(bus.dropped_count(), 1);

        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStopped,
        ))
        .await;
        assert_eq!(bus.dropped_count(), 2);
    }

    #[tokio::test]
    async fn test_task_events_persist_regardless_of_config() {
        use crate::services::event_store::{EventQuery, EventStoreError};

        // Minimal in-memory event store that records whether append was called
        struct TrackingStore {
            appended: std::sync::Mutex<Vec<EventCategory>>,
        }

        impl TrackingStore {
            fn new() -> Self {
                Self {
                    appended: std::sync::Mutex::new(Vec::new()),
                }
            }
        }

        #[async_trait::async_trait]
        impl EventStore for TrackingStore {
            async fn append(&self, event: &UnifiedEvent) -> Result<(), EventStoreError> {
                self.appended.lock().unwrap().push(event.category);
                Ok(())
            }
            async fn query(
                &self,
                _query: EventQuery,
            ) -> Result<Vec<UnifiedEvent>, EventStoreError> {
                Ok(vec![])
            }
            async fn latest_sequence(&self) -> Result<Option<SequenceNumber>, EventStoreError> {
                Ok(None)
            }
            async fn count(&self) -> Result<u64, EventStoreError> {
                Ok(0)
            }
            async fn prune_older_than(
                &self,
                _duration: std::time::Duration,
            ) -> Result<u64, EventStoreError> {
                Ok(0)
            }
        }

        let store = Arc::new(TrackingStore::new());

        // persist_events is FALSE, but Task/Workflow events should still persist
        let bus = EventBus::new(EventBusConfig {
            channel_capacity: 16,
            persist_events: false,
        })
        .with_store(store.clone());

        let _rx = bus.subscribe(); // Need a subscriber so events don't get dropped

        // Publish an Orchestrator event — should NOT be persisted
        bus.publish(orchestrator_event(
            EventSeverity::Info,
            EventPayload::OrchestratorStarted,
        ))
        .await;

        // Publish a Task event — should be persisted
        let task_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(Uuid::new_v4()),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskReady {
                task_id: Uuid::new_v4(),
                task_title: "test".to_string(),
            },
        };
        bus.publish(task_event).await;

        let appended = store.appended.lock().unwrap();
        // Only the Task event should have been persisted
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0], EventCategory::Task);
    }

    #[test]
    fn test_subsystem_error_variant_name() {
        let payload = EventPayload::SubsystemError {
            subsystem: "drain_ready_tasks".into(),
            error: "connection lost".into(),
        };
        assert_eq!(payload.variant_name(), "SubsystemError");
    }

    #[test]
    fn test_subsystem_error_category() {
        let payload = EventPayload::SubsystemError {
            subsystem: "drain_ready_tasks".into(),
            error: "connection lost".into(),
        };
        assert_eq!(
            payload.expected_category(),
            Some(EventCategory::Orchestrator)
        );
    }

    #[tokio::test]
    async fn test_subsystem_error_event_published() {
        let bus = EventBus::new(EventBusConfig::default());
        let mut rx = bus.subscribe();

        let event = crate::services::event_factory::orchestrator_event(
            EventSeverity::Error,
            EventPayload::SubsystemError {
                subsystem: "test_subsystem".into(),
                error: "test error".into(),
            },
        );
        bus.publish(event).await;

        let received = rx.recv().await.unwrap();
        assert_eq!(received.payload.variant_name(), "SubsystemError");
        if let EventPayload::SubsystemError { subsystem, error } = &received.payload {
            assert_eq!(subsystem, "test_subsystem");
            assert_eq!(error, "test error");
        } else {
            panic!("Expected SubsystemError payload");
        }
    }
