use super::*;

fn make_execution(template_name: &str, version: u32, outcome: TaskOutcome) -> TaskExecution {
    TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: template_name.to_string(),
        template_version: version,
        outcome,
        executed_at: Utc::now(),
        turns_used: 10,
        tokens_used: 1000,
        downstream_tasks: vec![],
    }
}

#[tokio::test]
async fn test_record_execution() {
    let evolution = EvolutionLoop::with_default_config();

    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;

    let stats = evolution.get_stats("test-agent").await.unwrap();
    assert_eq!(stats.total_tasks, 3);
    assert_eq!(stats.successful_tasks, 2);
    assert_eq!(stats.failed_tasks, 1);
    assert!((stats.success_rate - 0.666).abs() < 0.01);
}

#[tokio::test]
async fn test_low_success_rate_trigger() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 2,
        refinement_threshold: 0.60,
        major_refinement_threshold: 0.40,
        major_refinement_min_tasks: 3, // Lower threshold for test
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // 1 success, 3 failures = 25% success rate
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;

    let events = evolution.evaluate().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].trigger, EvolutionTrigger::VeryLowSuccessRate);
}

#[tokio::test]
async fn test_goal_violation_trigger() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::GoalViolation))
        .await;

    let events = evolution.evaluate().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].trigger, EvolutionTrigger::GoalViolations);
}

#[tokio::test]
async fn test_version_change_detection() {
    let evolution = EvolutionLoop::with_default_config();

    // Record executions for version 1
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
        .await;

    // Change to version 2
    evolution
        .record_execution(make_execution("test-agent", 2, TaskOutcome::Success))
        .await;

    let stats = evolution.get_stats("test-agent").await.unwrap();
    assert_eq!(stats.template_version, 2);
    assert_eq!(stats.total_tasks, 1); // Reset for new version
}

#[tokio::test]
async fn test_refinement_queue() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 2,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // 50% success rate (below 80%)
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;

    evolution.evaluate().await;

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].template_name, "test-agent");
}

#[tokio::test]
async fn test_refinement_lifecycle() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    let pending = evolution.get_pending_refinements().await;
    let request_id = pending[0].id;

    // Start refinement
    assert!(evolution.start_refinement(request_id).await);

    // Complete refinement
    evolution.complete_refinement(request_id, true).await;

    // Should no longer be pending
    let pending = evolution.get_pending_refinements().await;
    assert!(pending.is_empty());
}

#[tokio::test]
async fn test_refinement_lifecycle_failed_path() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Record a failure to trigger refinement
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(pending.len(), 1);
    let request_id = pending[0].id;

    // Start refinement
    assert!(evolution.start_refinement(request_id).await);

    // Complete refinement with failure (success = false)
    evolution.complete_refinement(request_id, false).await;

    // Should no longer be pending (Failed is a terminal state)
    let pending = evolution.get_pending_refinements().await;
    assert!(
        pending.is_empty(),
        "Failed refinement should not appear in pending list"
    );

    // A new evaluation should be able to create a new refinement request
    // since the previous one reached a terminal state (Failed)
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(
        pending.len(),
        1,
        "New refinement should be created after previous one failed"
    );
}

#[tokio::test]
async fn test_events_history() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    evolution
        .record_execution(make_execution("agent-a", 1, TaskOutcome::GoalViolation))
        .await;
    evolution
        .record_execution(make_execution("agent-b", 1, TaskOutcome::GoalViolation))
        .await;

    evolution.evaluate().await;

    let events = evolution.get_events(None).await;
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn test_refinement_deduplication_pending() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Record a failure to trigger refinement
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;

    // First evaluate creates a refinement request
    let events1 = evolution.evaluate().await;
    assert_eq!(events1.len(), 1);
    assert!(matches!(
        events1[0].action_taken,
        EvolutionAction::FlaggedForRefinement { .. }
    ));

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(pending.len(), 1);

    // Record another failure
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;

    // Second evaluate should NOT create a duplicate refinement
    let events2 = evolution.evaluate().await;
    assert_eq!(events2.len(), 1);
    assert!(matches!(
        events2[0].action_taken,
        EvolutionAction::NoAction { .. }
    ));

    // Still only one pending refinement
    let pending = evolution.get_pending_refinements().await;
    assert_eq!(pending.len(), 1);
}

#[tokio::test]
async fn test_refinement_deduplication_in_progress() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Record a failure and trigger refinement
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    let pending = evolution.get_pending_refinements().await;
    let request_id = pending[0].id;

    // Mark as in progress
    assert!(evolution.start_refinement(request_id).await);

    // Record another failure
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;

    // Evaluate should not create a duplicate (InProgress blocks too)
    let events = evolution.evaluate().await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].action_taken,
        EvolutionAction::NoAction { .. }
    ));
}

#[tokio::test]
async fn test_refinement_allowed_after_completion() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // First cycle: failure -> refinement -> complete
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    let pending = evolution.get_pending_refinements().await;
    let request_id = pending[0].id;
    assert!(evolution.start_refinement(request_id).await);
    evolution.complete_refinement(request_id, true).await;

    // No longer active
    assert!(!evolution.has_active_refinement("test-agent").await);

    // Another failure — should be allowed to create a new refinement
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    let events = evolution.evaluate().await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].action_taken,
        EvolutionAction::FlaggedForRefinement { .. }
    ));

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(pending.len(), 1);
}

#[tokio::test]
async fn test_has_active_refinement() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Initially no active refinements
    assert!(!evolution.has_active_refinement("test-agent").await);

    // Create a refinement
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    // Now has active refinement (Pending)
    assert!(evolution.has_active_refinement("test-agent").await);
    // Different template has none
    assert!(!evolution.has_active_refinement("other-agent").await);

    // Start it — still active (InProgress)
    let pending = evolution.get_pending_refinements().await;
    evolution.start_refinement(pending[0].id).await;
    assert!(evolution.has_active_refinement("test-agent").await);

    // Complete it — no longer active
    evolution.complete_refinement(pending[0].id, true).await;
    assert!(!evolution.has_active_refinement("test-agent").await);
}

/// Verify that direct-mode successes (with real turns/tokens) populate EvolutionLoop stats.
///
/// Direct-mode executions record actual turns_used and tokens_used because the substrate
/// runs a single-shot invocation and returns them. This test mirrors the
/// goal_processing.rs direct-mode success path (line ~1213).
#[tokio::test]
async fn test_direct_mode_success_populates_stats() {
    let evolution = EvolutionLoop::with_default_config();

    // Simulate direct-mode execution: turns and tokens are set from the real substrate response
    let exec = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "direct-agent".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Success,
        executed_at: Utc::now(),
        turns_used: 12,    // real turn count from substrate
        tokens_used: 5000, // real token count from substrate
        downstream_tasks: vec![],
    };
    evolution.record_execution(exec).await;

    let stats = evolution.get_stats("direct-agent").await.unwrap();
    assert_eq!(
        stats.total_tasks, 1,
        "direct-mode success must increment total_tasks"
    );
    assert_eq!(
        stats.successful_tasks, 1,
        "direct-mode success must increment successful_tasks"
    );
    assert_eq!(stats.failed_tasks, 0);
    assert!(
        (stats.success_rate - 1.0).abs() < 0.001,
        "single success = 100% rate"
    );
    assert!(
        (stats.avg_turns - 12.0).abs() < 0.001,
        "avg_turns must reflect real direct-mode turn count"
    );
    assert!(
        (stats.avg_tokens - 5000.0).abs() < 1.0,
        "avg_tokens must reflect real direct-mode token count"
    );
}

/// Verify that direct-mode failures populate EvolutionLoop stats.
///
/// Both error paths in goal_processing.rs (session error + substrate error) record
/// TaskOutcome::Failure with real turn/token counts when available.
#[tokio::test]
async fn test_direct_mode_failure_populates_stats() {
    let evolution = EvolutionLoop::with_default_config();

    // Simulate a direct-mode failure (session ended in error)
    let exec = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "direct-agent".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Failure,
        executed_at: Utc::now(),
        turns_used: 8,
        tokens_used: 3200,
        downstream_tasks: vec![],
    };
    evolution.record_execution(exec).await;

    let stats = evolution.get_stats("direct-agent").await.unwrap();
    assert_eq!(
        stats.total_tasks, 1,
        "direct-mode failure must increment total_tasks"
    );
    assert_eq!(
        stats.failed_tasks, 1,
        "direct-mode failure must increment failed_tasks"
    );
    assert_eq!(stats.successful_tasks, 0);
    assert!(
        (stats.success_rate - 0.0).abs() < 0.001,
        "single failure = 0% rate"
    );
}

/// Verify that EvolutionLoop.stats accumulates across both direct-mode and
/// convergent-mode executions for the same template.
///
/// Convergent-mode records turns_used=0 and tokens_used=0 (because iteration counts
/// and tokens are aggregated inside the convergence loop, not per-execution).
/// Both paths call record_execution(), so they should aggregate correctly.
#[tokio::test]
async fn test_stats_populated_across_both_modes() {
    let evolution = EvolutionLoop::with_default_config();

    // Convergent-mode execution: turns=0, tokens=0 (aggregated inside convergence loop)
    let convergent_exec = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "shared-agent".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Success,
        executed_at: Utc::now(),
        turns_used: 0,  // convergent path: tracks iterations, not turns
        tokens_used: 0, // convergent path: tokens aggregated inside convergence loop
        downstream_tasks: vec![],
    };
    evolution.record_execution(convergent_exec).await;

    // Direct-mode execution: turns and tokens set from real substrate response
    let direct_exec = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "shared-agent".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Success,
        executed_at: Utc::now(),
        turns_used: 15,
        tokens_used: 6000,
        downstream_tasks: vec![],
    };
    evolution.record_execution(direct_exec).await;

    let stats = evolution.get_stats("shared-agent").await.unwrap();
    assert_eq!(
        stats.total_tasks, 2,
        "both direct and convergent executions must count toward total"
    );
    assert_eq!(stats.successful_tasks, 2, "both successes must be counted");
    assert_eq!(stats.failed_tasks, 0);
    assert!(
        (stats.success_rate - 1.0).abs() < 0.001,
        "two successes = 100% rate"
    );
}

/// Verify that the statistical significance threshold (min_tasks_for_evaluation=5)
/// prevents premature RefinementRequest creation, even when direct-mode failures are recorded.
///
/// This is the core constraint from the evolution feedback loop goal:
/// "Refinement triggers must not fire on fewer than the configured minimum tasks."
#[tokio::test]
async fn test_statistical_significance_threshold_respected() {
    // Use the default config (min_tasks_for_evaluation = 5)
    let evolution = EvolutionLoop::with_default_config();

    // Record 4 failures (below min_tasks threshold of 5)
    for _ in 0..4 {
        evolution
            .record_execution(make_execution("agent-under-test", 1, TaskOutcome::Failure))
            .await;
    }

    // evaluate() must NOT create a RefinementRequest — sample too small
    let events = evolution.evaluate().await;
    let refinement_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.action_taken, EvolutionAction::FlaggedForRefinement { .. }))
        .collect();
    assert!(
        refinement_events.is_empty(),
        "evaluate() must not trigger refinement with only 4 tasks (below min_tasks=5); \
         got {:?}",
        events.iter().map(|e| &e.action_taken).collect::<Vec<_>>()
    );

    let pending = evolution.get_pending_refinements().await;
    assert!(
        pending.is_empty(),
        "no RefinementRequest must be created before min_tasks threshold is reached"
    );

    // Record a 5th failure (now meets the threshold)
    evolution
        .record_execution(make_execution("agent-under-test", 1, TaskOutcome::Failure))
        .await;

    let events = evolution.evaluate().await;
    let flagged = events
        .iter()
        .any(|e| matches!(e.action_taken, EvolutionAction::FlaggedForRefinement { .. }));
    assert!(
        flagged,
        "evaluate() must trigger refinement once min_tasks=5 is reached with 0% success rate"
    );

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(
        pending.len(),
        1,
        "exactly one RefinementRequest must be created after threshold is met"
    );
}

#[tokio::test]
async fn test_regression_detection_triggers_on_rate_drop() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 3,
        regression_min_tasks: 3,
        regression_threshold: 0.15,
        regression_detection_window_hours: 24,
        auto_revert_enabled: false,
        // Set low so LowSuccessRate/VeryLowSuccessRate don't fire before regression
        refinement_threshold: 0.01,
        major_refinement_threshold: 0.01,
        major_refinement_min_tasks: 100,
        stale_refinement_timeout_hours: 48,
    };
    let evolution = EvolutionLoop::new(config);

    // v1: 5 successes → 100% rate
    for _ in 0..5 {
        evolution
            .record_execution(make_execution("regress-agent", 1, TaskOutcome::Success))
            .await;
    }
    // Evaluate to establish baseline (no trigger expected since rate is high)
    let events = evolution.evaluate().await;
    assert!(events.is_empty(), "v1 at 100% should not trigger anything");

    // Switch to v2: 1 success + 3 failures → 25% rate (drop of 75%)
    evolution
        .record_execution(make_execution("regress-agent", 2, TaskOutcome::Success))
        .await;
    for _ in 0..3 {
        evolution
            .record_execution(make_execution("regress-agent", 2, TaskOutcome::Failure))
            .await;
    }

    let events = evolution.evaluate().await;
    let regression_event = events
        .iter()
        .find(|e| e.trigger == EvolutionTrigger::Regression);
    assert!(
        regression_event.is_some(),
        "Should detect regression after version change with rate drop (75%) >= threshold (15%); events: {:?}",
        events.iter().map(|e| &e.trigger).collect::<Vec<_>>()
    );
    // With auto_revert_enabled=false, action should be FlaggedForRefinement with Immediate severity
    if let Some(ev) = regression_event {
        assert!(
            matches!(
                ev.action_taken,
                EvolutionAction::FlaggedForRefinement {
                    severity: RefinementSeverity::Immediate
                }
            ),
            "Regression without auto-revert should flag for immediate refinement; got {:?}",
            ev.action_taken,
        );
    }
}

#[tokio::test]
async fn test_auto_revert_when_regression_detected() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 3,
        regression_min_tasks: 3,
        regression_threshold: 0.15,
        regression_detection_window_hours: 24,
        auto_revert_enabled: true,
        refinement_threshold: 0.01,
        major_refinement_threshold: 0.01,
        major_refinement_min_tasks: 100,
        stale_refinement_timeout_hours: 48,
    };
    let evolution = EvolutionLoop::new(config);

    // v1: 5 successes → 100% rate
    for _ in 0..5 {
        evolution
            .record_execution(make_execution("revert-agent", 1, TaskOutcome::Success))
            .await;
    }
    evolution.evaluate().await;

    // Switch to v2: 1 success + 2 failures → 33% rate (drop of 67% from 100%)
    evolution
        .record_execution(make_execution("revert-agent", 2, TaskOutcome::Success))
        .await;
    for _ in 0..2 {
        evolution
            .record_execution(make_execution("revert-agent", 2, TaskOutcome::Failure))
            .await;
    }

    let events = evolution.evaluate().await;
    let revert_event = events
        .iter()
        .find(|e| matches!(e.action_taken, EvolutionAction::Reverted { .. }));
    assert!(
        revert_event.is_some(),
        "Should auto-revert when regression detected and auto_revert_enabled=true; events: {:?}",
        events
            .iter()
            .map(|e| (&e.trigger, &e.action_taken))
            .collect::<Vec<_>>()
    );
    if let Some(ev) = revert_event {
        match &ev.action_taken {
            EvolutionAction::Reverted {
                from_version,
                to_version,
            } => {
                assert_eq!(*from_version, 2, "Should revert FROM version 2");
                assert_eq!(*to_version, 1, "Should revert TO version 1");
            }
            other => panic!("Expected Reverted action, got {:?}", other),
        }
    }
}

#[tokio::test]
async fn test_auto_revert_only_applies_to_regression_trigger() {
    // auto_revert_enabled=true, but the trigger is LowSuccessRate (not Regression).
    // The action must be FlaggedForRefinement, NOT Reverted.
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 3,
        regression_min_tasks: 3,
        regression_threshold: 0.15,
        regression_detection_window_hours: 24,
        auto_revert_enabled: true,
        refinement_threshold: 0.70,
        major_refinement_threshold: 0.01,
        major_refinement_min_tasks: 100,
        stale_refinement_timeout_hours: 48,
    };
    let evolution = EvolutionLoop::new(config);

    // All executions on the same version → no regression possible.
    // 1 success + 2 failures on v1 → 33% success rate, below refinement_threshold (0.70).
    evolution
        .record_execution(make_execution("guard-agent", 1, TaskOutcome::Success))
        .await;
    for _ in 0..2 {
        evolution
            .record_execution(make_execution("guard-agent", 1, TaskOutcome::Failure))
            .await;
    }

    let events = evolution.evaluate().await;

    // There should be an event for this agent, and it must NOT be Reverted.
    let agent_events: Vec<_> = events
        .iter()
        .filter(|e| e.template_name == "guard-agent")
        .collect();
    assert!(
        !agent_events.is_empty(),
        "Expected at least one evolution event for guard-agent; got none"
    );
    for ev in &agent_events {
        assert!(
            !matches!(ev.action_taken, EvolutionAction::Reverted { .. }),
            "LowSuccessRate trigger should NOT produce Reverted action; got {:?}",
            ev.action_taken
        );
        assert!(
            matches!(
                ev.action_taken,
                EvolutionAction::FlaggedForRefinement { .. }
            ),
            "LowSuccessRate trigger should produce FlaggedForRefinement; got {:?}",
            ev.action_taken
        );
    }
}

#[tokio::test]
async fn test_regression_window_expiry() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 3,
        regression_min_tasks: 3,
        regression_threshold: 0.15,
        regression_detection_window_hours: 1, // 1-hour window (will be expired)
        auto_revert_enabled: true,
        refinement_threshold: 0.01,
        major_refinement_threshold: 0.01,
        major_refinement_min_tasks: 100,
        stale_refinement_timeout_hours: 48,
    };
    let evolution = EvolutionLoop::new(config);

    // v1: 5 successes
    for _ in 0..5 {
        evolution
            .record_execution(make_execution("window-agent", 1, TaskOutcome::Success))
            .await;
    }
    evolution.evaluate().await;

    // Switch to v2 (this sets version_change_time to now)
    evolution
        .record_execution(make_execution("window-agent", 2, TaskOutcome::Failure))
        .await;

    // Manually backdate the version change time to >1 hour ago
    {
        let mut state = evolution.state.write().await;
        if let Some(entry) = state.version_change_times.get_mut("window-agent") {
            entry.1 = Utc::now() - Duration::hours(2);
        }
    }

    // Record more failures to meet regression_min_tasks
    for _ in 0..2 {
        evolution
            .record_execution(make_execution("window-agent", 2, TaskOutcome::Failure))
            .await;
    }

    let events = evolution.evaluate().await;
    let has_regression = events
        .iter()
        .any(|e| e.trigger == EvolutionTrigger::Regression);
    assert!(
        !has_regression,
        "Should NOT detect regression outside the detection window; events: {:?}",
        events.iter().map(|e| &e.trigger).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_expire_stale_refinements() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        stale_refinement_timeout_hours: 2,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Manually insert two Pending refinement requests with different ages
    let old_request = RefinementRequest {
        id: Uuid::new_v4(),
        template_name: "stale-agent".to_string(),
        template_version: 1,
        severity: RefinementSeverity::Minor,
        trigger: EvolutionTrigger::LowSuccessRate,
        stats: TemplateStats::new("stale-agent".to_string(), 1),
        failed_task_ids: vec![],
        created_at: Utc::now() - Duration::hours(3), // 3h old → should expire
        status: RefinementStatus::Pending,
    };
    let fresh_request = RefinementRequest {
        id: Uuid::new_v4(),
        template_name: "fresh-agent".to_string(),
        template_version: 1,
        severity: RefinementSeverity::Minor,
        trigger: EvolutionTrigger::LowSuccessRate,
        stats: TemplateStats::new("fresh-agent".to_string(), 1),
        failed_task_ids: vec![],
        created_at: Utc::now() - Duration::hours(1), // 1h old → should NOT expire
        status: RefinementStatus::Pending,
    };

    {
        let mut state = evolution.state.write().await;
        state.refinement_queue.push(old_request.clone());
        state.refinement_queue.push(fresh_request.clone());
    }

    let expired = evolution.expire_stale_refinements().await;
    assert_eq!(
        expired.len(),
        1,
        "only the 3h-old request should be expired"
    );
    assert_eq!(expired[0].0, "stale-agent");
    assert_eq!(expired[0].1, 1);
    assert_eq!(expired[0].2, old_request.id);

    let state = evolution.state.read().await;
    let old = state
        .refinement_queue
        .iter()
        .find(|r| r.id == old_request.id)
        .unwrap();
    assert_eq!(old.status, RefinementStatus::Failed);

    let fresh = state
        .refinement_queue
        .iter()
        .find(|r| r.id == fresh_request.id)
        .unwrap();
    assert_eq!(fresh.status, RefinementStatus::Pending);
}

#[tokio::test]
async fn test_expire_stale_refinements_disabled_when_zero() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        stale_refinement_timeout_hours: 0, // disabled
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Insert a very old Pending request
    let ancient_request = RefinementRequest {
        id: Uuid::new_v4(),
        template_name: "ancient-agent".to_string(),
        template_version: 1,
        severity: RefinementSeverity::Minor,
        trigger: EvolutionTrigger::LowSuccessRate,
        stats: TemplateStats::new("ancient-agent".to_string(), 1),
        failed_task_ids: vec![],
        created_at: Utc::now() - Duration::hours(1000), // 1000h old
        status: RefinementStatus::Pending,
    };

    {
        let mut state = evolution.state.write().await;
        state.refinement_queue.push(ancient_request.clone());
    }

    let expired = evolution.expire_stale_refinements().await;
    assert_eq!(expired.len(), 0, "expiry disabled when timeout=0");

    let state = evolution.state.read().await;
    let req = state
        .refinement_queue
        .iter()
        .find(|r| r.id == ancient_request.id)
        .unwrap();
    assert_eq!(
        req.status,
        RefinementStatus::Pending,
        "request must remain Pending when expiry is disabled"
    );
}

#[tokio::test]
async fn test_evaluate_emits_events_for_stale_expirations() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        stale_refinement_timeout_hours: 2,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Insert a stale Pending refinement request
    let stale_request = RefinementRequest {
        id: Uuid::new_v4(),
        template_name: "stale-agent".to_string(),
        template_version: 3,
        severity: RefinementSeverity::Minor,
        trigger: EvolutionTrigger::LowSuccessRate,
        stats: TemplateStats::new("stale-agent".to_string(), 3),
        failed_task_ids: vec![],
        created_at: Utc::now() - Duration::hours(5),
        status: RefinementStatus::Pending,
    };

    {
        let mut state = evolution.state.write().await;
        state.refinement_queue.push(stale_request.clone());
    }

    let events = evolution.evaluate().await;

    // Should have exactly one StaleExpired event
    let stale_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.action_taken, EvolutionAction::StaleExpired { .. }))
        .collect();
    assert_eq!(stale_events.len(), 1, "should emit one StaleExpired event");

    let event = stale_events[0];
    assert_eq!(event.template_name, "stale-agent");
    assert_eq!(event.template_version, 3);
    assert_eq!(event.trigger, EvolutionTrigger::StaleTimeout);
    if let EvolutionAction::StaleExpired { request_id } = &event.action_taken {
        assert_eq!(*request_id, stale_request.id);
    } else {
        panic!("Expected StaleExpired action");
    }
}

/// A mock agent repository that records update_template calls for verification.
struct MockAgentRepo {
    templates: tokio::sync::Mutex<HashMap<(String, u32), crate::domain::models::AgentTemplate>>,
    updated: tokio::sync::Mutex<Vec<crate::domain::models::AgentTemplate>>,
}

impl MockAgentRepo {
    fn new() -> Self {
        Self {
            templates: tokio::sync::Mutex::new(HashMap::new()),
            updated: tokio::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl crate::domain::ports::AgentRepository for MockAgentRepo {
    async fn create_template(
        &self,
        _template: &crate::domain::models::AgentTemplate,
    ) -> crate::domain::errors::DomainResult<()> {
        Ok(())
    }
    async fn get_template(
        &self,
        _id: Uuid,
    ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentTemplate>>
    {
        Ok(None)
    }
    async fn get_template_by_name(
        &self,
        _name: &str,
    ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentTemplate>>
    {
        Ok(None)
    }
    async fn get_template_version(
        &self,
        name: &str,
        version: u32,
    ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentTemplate>>
    {
        let templates = self.templates.lock().await;
        Ok(templates.get(&(name.to_string(), version)).cloned())
    }
    async fn update_template(
        &self,
        template: &crate::domain::models::AgentTemplate,
    ) -> crate::domain::errors::DomainResult<()> {
        let mut updated = self.updated.lock().await;
        updated.push(template.clone());
        Ok(())
    }
    async fn delete_template(&self, _id: Uuid) -> crate::domain::errors::DomainResult<()> {
        Ok(())
    }
    async fn list_templates(
        &self,
        _filter: crate::domain::ports::AgentFilter,
    ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentTemplate>>
    {
        Ok(vec![])
    }
    async fn list_by_tier(
        &self,
        _tier: crate::domain::models::AgentTier,
    ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentTemplate>>
    {
        Ok(vec![])
    }
    async fn get_active_templates(
        &self,
    ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentTemplate>>
    {
        Ok(vec![])
    }
    async fn create_instance(
        &self,
        _instance: &crate::domain::models::AgentInstance,
    ) -> crate::domain::errors::DomainResult<()> {
        Ok(())
    }
    async fn get_instance(
        &self,
        _id: Uuid,
    ) -> crate::domain::errors::DomainResult<Option<crate::domain::models::AgentInstance>>
    {
        Ok(None)
    }
    async fn update_instance(
        &self,
        _instance: &crate::domain::models::AgentInstance,
    ) -> crate::domain::errors::DomainResult<()> {
        Ok(())
    }
    async fn delete_instance(&self, _id: Uuid) -> crate::domain::errors::DomainResult<()> {
        Ok(())
    }
    async fn list_instances_by_status(
        &self,
        _status: crate::domain::models::InstanceStatus,
    ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentInstance>>
    {
        Ok(vec![])
    }
    async fn get_running_instances(
        &self,
        _template_name: &str,
    ) -> crate::domain::errors::DomainResult<Vec<crate::domain::models::AgentInstance>>
    {
        Ok(vec![])
    }
    async fn count_running_by_template(
        &self,
    ) -> crate::domain::errors::DomainResult<HashMap<String, u32>> {
        Ok(HashMap::new())
    }
}

fn make_template(name: &str, version: u32) -> crate::domain::models::AgentTemplate {
    use crate::domain::models::{AgentCard, AgentStatus, AgentTier};
    crate::domain::models::AgentTemplate {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: format!("{} v{}", name, version),
        tier: AgentTier::Worker,
        version,
        system_prompt: "test prompt".to_string(),
        tools: vec![],
        constraints: vec![],
        agent_card: AgentCard::default(),
        max_turns: 10,
        read_only: false,
        preferred_model: None,
        status: AgentStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn test_auto_revert_actually_restores_template() {
    let v1_template = make_template("revert-real", 1);

    let mock_repo = Arc::new(MockAgentRepo::new());
    {
        let mut templates = mock_repo.templates.lock().await;
        templates.insert(
            (v1_template.name.clone(), v1_template.version),
            v1_template.clone(),
        );
    }

    let config = EvolutionConfig {
        min_tasks_for_evaluation: 3,
        regression_min_tasks: 3,
        regression_threshold: 0.15,
        regression_detection_window_hours: 24,
        auto_revert_enabled: true,
        refinement_threshold: 0.01,
        major_refinement_threshold: 0.01,
        major_refinement_min_tasks: 100,
        stale_refinement_timeout_hours: 48,
    };
    let evolution = EvolutionLoop::new(config).with_agent_repo(mock_repo.clone());

    // v1: 5 successes → 100% rate
    for _ in 0..5 {
        evolution
            .record_execution(make_execution("revert-real", 1, TaskOutcome::Success))
            .await;
    }
    evolution.evaluate().await;

    // Switch to v2: 1 success + 2 failures → 33% rate (drop of 67%)
    evolution
        .record_execution(make_execution("revert-real", 2, TaskOutcome::Success))
        .await;
    for _ in 0..2 {
        evolution
            .record_execution(make_execution("revert-real", 2, TaskOutcome::Failure))
            .await;
    }

    let events = evolution.evaluate().await;

    // Verify the Reverted event was emitted
    let revert_event = events
        .iter()
        .find(|e| matches!(e.action_taken, EvolutionAction::Reverted { .. }));
    assert!(revert_event.is_some(), "Should emit Reverted event");

    // Verify the agent repo was actually called to restore v1
    let updated = mock_repo.updated.lock().await;
    assert_eq!(
        updated.len(),
        1,
        "Should have called update_template exactly once"
    );
    assert_eq!(updated[0].name, "revert-real");
    assert_eq!(updated[0].version, 1, "Should restore version 1");
    assert_eq!(
        updated[0].status,
        crate::domain::models::AgentStatus::Active,
        "Restored template should be marked Active"
    );
}

#[tokio::test]
async fn test_auto_revert_graceful_when_no_repo() {
    // No agent_repo configured — should still emit the Reverted event
    // without panicking
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 3,
        regression_min_tasks: 3,
        regression_threshold: 0.15,
        regression_detection_window_hours: 24,
        auto_revert_enabled: true,
        refinement_threshold: 0.01,
        major_refinement_threshold: 0.01,
        major_refinement_min_tasks: 100,
        stale_refinement_timeout_hours: 48,
    };
    let evolution = EvolutionLoop::new(config);
    // Deliberately NOT calling with_agent_repo

    // v1: 5 successes
    for _ in 0..5 {
        evolution
            .record_execution(make_execution("no-repo-agent", 1, TaskOutcome::Success))
            .await;
    }
    evolution.evaluate().await;

    // Switch to v2 with regression
    evolution
        .record_execution(make_execution("no-repo-agent", 2, TaskOutcome::Success))
        .await;
    for _ in 0..2 {
        evolution
            .record_execution(make_execution("no-repo-agent", 2, TaskOutcome::Failure))
            .await;
    }

    // Should not panic — just emit the event without repo restoration
    let events = evolution.evaluate().await;
    let revert_event = events
        .iter()
        .find(|e| matches!(e.action_taken, EvolutionAction::Reverted { .. }));
    assert!(
        revert_event.is_some(),
        "Should still emit Reverted event even without agent_repo"
    );
}

#[tokio::test]
async fn test_get_all_stats_returns_all_templates() {
    let evolution = EvolutionLoop::with_default_config();

    evolution
        .record_execution(make_execution("agent-a", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("agent-b", 1, TaskOutcome::Failure))
        .await;
    evolution
        .record_execution(make_execution("agent-c", 2, TaskOutcome::Success))
        .await;

    let all_stats = evolution.get_all_stats().await;
    assert_eq!(all_stats.len(), 3);

    let names: Vec<String> = all_stats.iter().map(|s| s.template_name.clone()).collect();
    assert!(names.contains(&"agent-a".to_string()));
    assert!(names.contains(&"agent-b".to_string()));
    assert!(names.contains(&"agent-c".to_string()));
}

#[tokio::test]
async fn test_get_all_stats_empty_when_no_executions() {
    let evolution = EvolutionLoop::with_default_config();
    let all_stats = evolution.get_all_stats().await;
    assert!(all_stats.is_empty());
}

#[tokio::test]
async fn test_get_events_returns_reverse_chronological() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Record a single agent's goal violation and evaluate
    evolution
        .record_execution(make_execution("agent-first", 1, TaskOutcome::GoalViolation))
        .await;
    evolution.evaluate().await;

    let events = evolution.get_events(None).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].template_name, "agent-first");

    // Now record a second agent and evaluate again
    evolution
        .record_execution(make_execution(
            "agent-second",
            1,
            TaskOutcome::GoalViolation,
        ))
        .await;
    evolution.evaluate().await;

    let all_events = evolution.get_events(None).await;
    // Multiple events from both evaluations — verify reverse chronological order
    assert!(all_events.len() >= 2);

    // Most recent event should be last appended (reversed = first returned)
    // The last evaluate produced events for agent-second (and possibly agent-first again)
    // Just verify ordering: each event's occurred_at should be >= the next
    for window in all_events.windows(2) {
        assert!(
            window[0].occurred_at >= window[1].occurred_at,
            "Events should be in reverse chronological order"
        );
    }

    // Test with limit — should return only the most recent event(s)
    let limited = evolution.get_events(Some(1)).await;
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].occurred_at, all_events[0].occurred_at);
}

#[tokio::test]
async fn test_get_templates_needing_attention_sorted_by_severity() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        major_refinement_threshold: 0.40,
        major_refinement_min_tasks: 1,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Agent with minor issue (below refinement_threshold but above major)
    evolution
        .record_execution(make_execution("minor-agent", 1, TaskOutcome::Success))
        .await;
    evolution
        .record_execution(make_execution("minor-agent", 1, TaskOutcome::Failure))
        .await;

    // Agent with goal violation (immediate severity)
    evolution
        .record_execution(make_execution(
            "immediate-agent",
            1,
            TaskOutcome::GoalViolation,
        ))
        .await;

    evolution.evaluate().await;

    let attention = evolution.get_templates_needing_attention().await;
    assert!(attention.len() >= 2);

    // Immediate should come before Minor
    let immediate_pos = attention
        .iter()
        .position(|(name, _)| name == "immediate-agent");
    let minor_pos = attention.iter().position(|(name, _)| name == "minor-agent");

    if let (Some(imm), Some(min)) = (immediate_pos, minor_pos) {
        assert!(imm < min, "Immediate severity should sort before Minor");
    }
}

#[tokio::test]
async fn test_get_templates_needing_attention_excludes_non_pending() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Create a refinement
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    // Verify it shows up
    let attention = evolution.get_templates_needing_attention().await;
    assert_eq!(attention.len(), 1);

    // Start and complete the refinement
    let pending = evolution.get_pending_refinements().await;
    let request_id = pending[0].id;
    evolution.start_refinement(request_id).await;
    evolution.complete_refinement(request_id, true).await;

    // Should no longer need attention (Completed is not Pending)
    let attention = evolution.get_templates_needing_attention().await;
    assert!(
        attention.is_empty(),
        "Completed refinements should not appear in needing-attention list"
    );
}

#[tokio::test]
async fn test_clear_resets_all_state() {
    let config = EvolutionConfig {
        min_tasks_for_evaluation: 1,
        refinement_threshold: 0.80,
        ..Default::default()
    };
    let evolution = EvolutionLoop::new(config);

    // Populate state
    evolution
        .record_execution(make_execution("test-agent", 1, TaskOutcome::Failure))
        .await;
    evolution.evaluate().await;

    // Verify state is populated
    assert!(!evolution.get_all_stats().await.is_empty());
    assert!(!evolution.get_events(None).await.is_empty());
    assert!(!evolution.get_pending_refinements().await.is_empty());

    // Clear
    evolution.clear().await;

    // Verify all state is reset
    assert!(evolution.get_all_stats().await.is_empty());
    assert!(evolution.get_events(None).await.is_empty());
    assert!(evolution.get_pending_refinements().await.is_empty());
}

// ── MockRefinementRepo for persistence tests ──

/// A mock `RefinementRepository` backed by in-memory `Arc<Mutex<Vec<...>>>` collections.
struct MockRefinementRepo {
    requests: std::sync::Mutex<Vec<RefinementRequest>>,
    stats: std::sync::Mutex<Vec<TemplateStats>>,
    version_changes: std::sync::Mutex<Vec<VersionChangeRecord>>,
}

impl MockRefinementRepo {
    fn new() -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            stats: std::sync::Mutex::new(Vec::new()),
            version_changes: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl RefinementRepository for MockRefinementRepo {
    async fn create(
        &self,
        request: &RefinementRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(())
    }

    async fn get_pending(
        &self,
    ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.status == RefinementStatus::Pending)
            .cloned()
            .collect())
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: RefinementStatus,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut reqs = self.requests.lock().unwrap();
        if let Some(r) = reqs.iter_mut().find(|r| r.id == id) {
            r.status = status;
        }
        Ok(())
    }

    async fn reset_in_progress_to_pending(
        &self,
    ) -> Result<Vec<RefinementRequest>, Box<dyn std::error::Error + Send + Sync>> {
        let mut reqs = self.requests.lock().unwrap();
        let mut recovered = Vec::new();
        for r in reqs.iter_mut() {
            if r.status == RefinementStatus::InProgress {
                r.status = RefinementStatus::Pending;
                recovered.push(r.clone());
            }
        }
        Ok(recovered)
    }

    async fn load_all_stats(
        &self,
    ) -> Result<Vec<TemplateStats>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.stats.lock().unwrap().clone())
    }

    async fn load_version_changes(
        &self,
    ) -> Result<Vec<VersionChangeRecord>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.version_changes.lock().unwrap().clone())
    }
}

#[tokio::test]
async fn test_load_persisted_state_restores_stats() {
    let repo = Arc::new(MockRefinementRepo::new());
    {
        let mut stats = repo.stats.lock().unwrap();
        let mut s = TemplateStats::new("persisted-agent".to_string(), 3);
        s.total_tasks = 10;
        s.successful_tasks = 7;
        s.success_rate = 0.7;
        stats.push(s);
    }

    let evolution = EvolutionLoop::new(EvolutionConfig::default())
        .with_repo(repo as Arc<dyn RefinementRepository>);
    evolution.load_persisted_state().await;

    let all = evolution.get_all_stats().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].template_name, "persisted-agent");
    assert_eq!(all[0].total_tasks, 10);
    assert_eq!(all[0].template_version, 3);
}

#[tokio::test]
async fn test_load_persisted_state_or_insert_semantics() {
    // In-memory stats should take precedence over repo stats.
    let repo = Arc::new(MockRefinementRepo::new());
    {
        let mut stats = repo.stats.lock().unwrap();
        let mut s = TemplateStats::new("agent-x".to_string(), 1);
        s.total_tasks = 50;
        s.successful_tasks = 25;
        s.success_rate = 0.5;
        stats.push(s);
    }

    let evolution = EvolutionLoop::new(EvolutionConfig::default())
        .with_repo(repo as Arc<dyn RefinementRepository>);

    // Record an execution first (populates in-memory stats for "agent-x")
    evolution
        .record_execution(make_execution("agent-x", 1, TaskOutcome::Success))
        .await;

    // Now load persisted state — should NOT overwrite the in-memory entry
    evolution.load_persisted_state().await;

    let stats = evolution.get_stats("agent-x").await.unwrap();
    assert_eq!(
        stats.total_tasks, 1,
        "in-memory stats (1 task) must take precedence over repo stats (50 tasks)"
    );
}

#[tokio::test]
async fn test_load_persisted_state_restores_version_changes() {
    let repo = Arc::new(MockRefinementRepo::new());
    {
        let mut changes = repo.version_changes.lock().unwrap();
        let prev_stats = TemplateStats::new("vc-agent".to_string(), 1);
        changes.push(VersionChangeRecord {
            template_name: "vc-agent".to_string(),
            from_version: 1,
            to_version: 2,
            previous_stats: prev_stats,
            changed_at: Utc::now() - Duration::hours(1),
        });
    }

    let evolution = EvolutionLoop::new(EvolutionConfig::default())
        .with_repo(repo as Arc<dyn RefinementRepository>);
    evolution.load_persisted_state().await;

    // Verify that previous_version_stats were restored
    let state = evolution.state.read().await;
    assert!(
        state.previous_version_stats.contains_key("vc-agent"),
        "version change should restore previous_version_stats"
    );
    assert_eq!(
        state.previous_version_stats["vc-agent"].template_version, 1,
        "previous stats should be for version 1"
    );
    assert!(
        state.version_change_times.contains_key("vc-agent"),
        "version change should restore version_change_times"
    );
    assert_eq!(
        state.version_change_times["vc-agent"].0, 2,
        "version_change_times should record to_version=2"
    );
}

#[tokio::test]
async fn test_recover_in_progress_refinements() {
    let repo = Arc::new(MockRefinementRepo::new());
    let request_id = Uuid::new_v4();
    {
        let mut reqs = repo.requests.lock().unwrap();
        reqs.push(RefinementRequest {
            id: request_id,
            template_name: "recover-agent".to_string(),
            template_version: 1,
            severity: RefinementSeverity::Minor,
            trigger: EvolutionTrigger::LowSuccessRate,
            stats: TemplateStats::new("recover-agent".to_string(), 1),
            failed_task_ids: vec![],
            created_at: Utc::now(),
            status: RefinementStatus::InProgress,
        });
    }

    let evolution = EvolutionLoop::new(EvolutionConfig::default())
        .with_repo(repo.clone() as Arc<dyn RefinementRepository>);
    evolution.recover_in_progress_refinements().await;

    // The request should have been reset to Pending in the repo
    {
        let repo_reqs = repo.requests.lock().unwrap();
        assert_eq!(repo_reqs[0].status, RefinementStatus::Pending);
    }

    // And loaded into the in-memory queue
    let pending = evolution.get_pending_refinements().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, request_id);
    assert_eq!(pending[0].status, RefinementStatus::Pending);
}

#[tokio::test]
async fn test_load_from_repo_dedup() {
    let repo = Arc::new(MockRefinementRepo::new());
    let shared_id = Uuid::new_v4();
    let request = RefinementRequest {
        id: shared_id,
        template_name: "dedup-agent".to_string(),
        template_version: 1,
        severity: RefinementSeverity::Minor,
        trigger: EvolutionTrigger::LowSuccessRate,
        stats: TemplateStats::new("dedup-agent".to_string(), 1),
        failed_task_ids: vec![],
        created_at: Utc::now(),
        status: RefinementStatus::Pending,
    };
    {
        repo.requests.lock().unwrap().push(request.clone());
    }

    let evolution = EvolutionLoop::new(EvolutionConfig::default())
        .with_repo(repo as Arc<dyn RefinementRepository>);

    // Manually insert the same request into in-memory queue first
    {
        let mut state = evolution.state.write().await;
        state.refinement_queue.push(request);
    }

    // load_from_repo should NOT duplicate
    evolution.load_from_repo().await;

    let pending = evolution.get_pending_refinements().await;
    assert_eq!(
        pending.len(),
        1,
        "duplicate IDs from repo must not create duplicates in queue"
    );
}

#[tokio::test]
async fn test_load_persisted_state_no_repo_is_noop() {
    // No repo attached — should not panic and leave state empty
    let evolution = EvolutionLoop::with_default_config();
    evolution.load_persisted_state().await;

    let all = evolution.get_all_stats().await;
    assert!(all.is_empty(), "no repo means no stats loaded");

    // Also test recover_in_progress_refinements with no repo
    evolution.recover_in_progress_refinements().await;
    let pending = evolution.get_pending_refinements().await;
    assert!(pending.is_empty(), "no repo means no refinements recovered");
}

#[test]
fn test_evolution_config_default_values() {
    let config = EvolutionConfig::default();
    assert_eq!(config.min_tasks_for_evaluation, 5);
    assert!((config.refinement_threshold - 0.60).abs() < f64::EPSILON);
    assert!((config.major_refinement_threshold - 0.40).abs() < f64::EPSILON);
    assert_eq!(config.major_refinement_min_tasks, 10);
    assert_eq!(config.regression_detection_window_hours, 24);
    assert_eq!(config.regression_min_tasks, 3);
    assert!((config.regression_threshold - 0.15).abs() < f64::EPSILON);
    assert!(config.auto_revert_enabled);
    assert_eq!(config.stale_refinement_timeout_hours, 48);
}

#[tokio::test]
async fn test_template_stats_average_computation() {
    let mut stats = TemplateStats::new("avg-test".to_string(), 1);

    // First execution: 10 turns, 1000 tokens
    let exec1 = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "avg-test".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Success,
        executed_at: Utc::now(),
        turns_used: 10,
        tokens_used: 1000,
        downstream_tasks: vec![],
    };
    stats.update(&exec1);
    assert!((stats.avg_turns - 10.0).abs() < f64::EPSILON);
    assert!((stats.avg_tokens - 1000.0).abs() < f64::EPSILON);
    assert_eq!(stats.total_tasks, 1);

    // Second execution: 20 turns, 3000 tokens → averages become 15.0 and 2000.0
    let exec2 = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "avg-test".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Success,
        executed_at: Utc::now(),
        turns_used: 20,
        tokens_used: 3000,
        downstream_tasks: vec![],
    };
    stats.update(&exec2);
    assert!((stats.avg_turns - 15.0).abs() < f64::EPSILON);
    assert!((stats.avg_tokens - 2000.0).abs() < f64::EPSILON);
    assert_eq!(stats.total_tasks, 2);

    // Third execution: 30 turns, 5000 tokens → averages become 20.0 and 3000.0
    let exec3 = TaskExecution {
        task_id: Uuid::new_v4(),
        template_name: "avg-test".to_string(),
        template_version: 1,
        outcome: TaskOutcome::Failure,
        executed_at: Utc::now(),
        turns_used: 30,
        tokens_used: 5000,
        downstream_tasks: vec![],
    };
    stats.update(&exec3);
    assert!((stats.avg_turns - 20.0).abs() < f64::EPSILON);
    assert!((stats.avg_tokens - 3000.0).abs() < f64::EPSILON);
    assert_eq!(stats.total_tasks, 3);
    assert_eq!(stats.successful_tasks, 2);
    assert_eq!(stats.failed_tasks, 1);
}

#[test]
fn test_refinement_request_new_defaults() {
    let stats = TemplateStats::new("req-test".to_string(), 3);
    let failed_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
    let before = Utc::now();

    let request = RefinementRequest::new(
        "req-test".to_string(),
        3,
        RefinementSeverity::Major,
        EvolutionTrigger::LowSuccessRate,
        stats,
        failed_ids.clone(),
    );

    let after = Utc::now();

    assert_eq!(request.template_name, "req-test");
    assert_eq!(request.template_version, 3);
    assert_eq!(request.severity, RefinementSeverity::Major);
    assert_eq!(request.trigger, EvolutionTrigger::LowSuccessRate);
    assert_eq!(request.status, RefinementStatus::Pending);
    assert_eq!(request.failed_task_ids.len(), 2);
    assert_eq!(request.failed_task_ids, failed_ids);
    assert!(request.created_at >= before && request.created_at <= after);
    // id must be a valid non-nil UUID
    assert_ne!(request.id, Uuid::nil());
}

#[tokio::test]
async fn test_complete_refinement_nonexistent_id_is_noop() {
    let evolution = EvolutionLoop::new(EvolutionConfig::default());

    // Completing a non-existent refinement should not panic or create entries
    evolution.complete_refinement(Uuid::new_v4(), true).await;
    evolution.complete_refinement(Uuid::new_v4(), false).await;

    let pending = evolution.get_pending_refinements().await;
    assert!(pending.is_empty(), "No refinement entries should exist");
}
