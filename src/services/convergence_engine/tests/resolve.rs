//! Tests for `convergence_engine::resolve`.

use std::sync::Arc;

use crate::domain::models::Memory;
use crate::domain::models::MemoryTier;
use crate::domain::models::convergence::*;
use crate::domain::models::task::Complexity;
use crate::domain::ports::MemoryRepository;

use super::super::ConvergenceEngine;
use super::super::test_support::*;
use super::{test_engine, test_trajectory};

// -----------------------------------------------------------------------
// finalize tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_finalize_converged_stores_success_memory() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let mut trajectory = test_trajectory();
    let bandit = StrategyBandit::with_default_priors();
    let outcome = ConvergenceOutcome::Converged {
        trajectory_id: trajectory.id.to_string(),
        final_observation_sequence: 5,
    };

    engine
        .finalize(&mut trajectory, &outcome, &bandit)
        .await
        .unwrap();

    assert_eq!(trajectory.phase, ConvergencePhase::Converged);
    let memories = mem_repo.memories.lock().unwrap();
    // Should have success memory + bandit state
    assert!(
        memories.len() >= 2,
        "Expected at least 2 memories, got {}",
        memories.len()
    );
    assert!(
        memories
            .iter()
            .any(|m| m.metadata.tags.contains(&"success".to_string()))
    );
    assert!(
        memories
            .iter()
            .any(|m| m.metadata.tags.contains(&"strategy-bandit".to_string()))
    );
}

#[tokio::test]
async fn test_finalize_exhausted_stores_failure_memory() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let mut trajectory = test_trajectory();
    let bandit = StrategyBandit::with_default_priors();
    let outcome = ConvergenceOutcome::Exhausted {
        trajectory_id: trajectory.id.to_string(),
        best_observation_sequence: Some(3),
    };

    engine
        .finalize(&mut trajectory, &outcome, &bandit)
        .await
        .unwrap();

    assert_eq!(trajectory.phase, ConvergencePhase::Exhausted);
    let memories = mem_repo.memories.lock().unwrap();
    assert!(
        memories
            .iter()
            .any(|m| m.metadata.tags.contains(&"failure".to_string()))
    );
}

#[tokio::test]
async fn test_finalize_trapped_stores_failure_memory() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let mut trajectory = test_trajectory();
    let bandit = StrategyBandit::with_default_priors();
    let outcome = ConvergenceOutcome::Trapped {
        trajectory_id: trajectory.id.to_string(),
        attractor_type: AttractorType::LimitCycle {
            period: 2,
            cycle_signatures: vec![],
        },
    };

    engine
        .finalize(&mut trajectory, &outcome, &bandit)
        .await
        .unwrap();

    assert_eq!(trajectory.phase, ConvergencePhase::Trapped);
    let memories = mem_repo.memories.lock().unwrap();
    assert!(
        memories
            .iter()
            .any(|m| m.metadata.tags.contains(&"failure".to_string()))
    );
}

// -----------------------------------------------------------------------
// request_extension tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_request_extension_granted_for_fixed_point() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Set up as approaching fixed point
    trajectory.attractor_state = AttractorState {
        classification: AttractorType::FixedPoint {
            estimated_remaining_iterations: 2,
            estimated_remaining_tokens: 30_000,
        },
        confidence: 0.8,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.1, 0.08],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    let result = engine.request_extension(&mut trajectory).await.unwrap();
    assert!(result, "Extension should be granted for fixed point");
    assert_eq!(trajectory.budget.extensions_requested, 1);
    assert_eq!(trajectory.budget.extensions_granted, 1);
}

#[tokio::test]
async fn test_request_extension_denied_for_non_fixed_point() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    // Trajectory in limit cycle
    trajectory.attractor_state = AttractorState {
        classification: AttractorType::LimitCycle {
            period: 2,
            cycle_signatures: vec![],
        },
        confidence: 0.85,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    let result = engine.request_extension(&mut trajectory).await.unwrap();
    assert!(!result, "Extension should be denied for limit cycle");
    assert_eq!(trajectory.budget.extensions_requested, 1);
    assert_eq!(trajectory.budget.extensions_granted, 0);
}

#[tokio::test]
async fn test_request_extension_granted_for_indeterminate_improving() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    trajectory.attractor_state = AttractorState {
        classification: AttractorType::Indeterminate {
            tendency: ConvergenceTendency::Improving,
        },
        confidence: 0.4,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.05],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    let result = engine.request_extension(&mut trajectory).await.unwrap();
    assert!(
        result,
        "Extension should be granted for Indeterminate+Improving"
    );
    assert_eq!(trajectory.budget.extensions_granted, 1);
}

#[tokio::test]
async fn test_request_extension_denied_for_indeterminate_declining() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    trajectory.attractor_state = AttractorState {
        classification: AttractorType::Indeterminate {
            tendency: ConvergenceTendency::Declining,
        },
        confidence: 0.4,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![-0.02],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    let result = engine.request_extension(&mut trajectory).await.unwrap();
    assert!(
        !result,
        "Extension should be denied for Indeterminate+Declining"
    );
    assert_eq!(trajectory.budget.extensions_granted, 0);
}

#[tokio::test]
async fn test_request_extension_granted_for_plateau() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    trajectory.attractor_state = AttractorState {
        classification: AttractorType::Plateau {
            stall_duration: 5,
            plateau_level: 0.6,
        },
        confidence: 0.7,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![0.0, 0.0, 0.01],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    let result = engine.request_extension(&mut trajectory).await.unwrap();
    assert!(result, "Extension should be granted for Plateau");
    assert_eq!(trajectory.budget.extensions_granted, 1);
}

#[tokio::test]
async fn test_request_extension_denied_for_divergent() {
    let engine = test_engine();
    let mut trajectory = test_trajectory();

    trajectory.attractor_state = AttractorState {
        classification: AttractorType::Divergent {
            divergence_rate: 0.15,
            probable_cause: DivergenceCause::AccumulatedRegression,
        },
        confidence: 0.9,
        detected_at: None,
        evidence: AttractorEvidence {
            recent_deltas: vec![-0.1, -0.12],
            recent_signatures: vec![],
            rationale: String::new(),
        },
    };

    let result = engine.request_extension(&mut trajectory).await.unwrap();
    assert!(!result, "Extension should be denied for Divergent");
    assert_eq!(trajectory.budget.extensions_granted, 0);
}

// -----------------------------------------------------------------------
// initialize_bandit tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_initialize_bandit_returns_defaults_without_memory() {
    let engine = test_engine();
    let trajectory = test_trajectory();

    let bandit = engine.initialize_bandit(&trajectory).await;

    // Should have default priors
    assert!(bandit.context_arms.contains_key("fixed_point"));
    assert!(bandit.context_arms.contains_key("limit_cycle"));
}

#[tokio::test]
async fn test_initialize_bandit_restores_from_memory() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let trajectory = test_trajectory();

    // Pre-populate memory with a bandit state
    let mut original_bandit = StrategyBandit::with_default_priors();
    original_bandit.nudge("fixed_point", "focused_repair", 5.0);

    let content = serde_json::to_string(&original_bandit).unwrap();
    let memory = Memory::semantic(format!("strategy-bandit-{}", trajectory.task_id), content)
        .with_namespace("convergence")
        .with_task(trajectory.task_id)
        .with_tag("strategy-bandit");
    mem_repo.store(&memory).await.unwrap();

    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo,
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let bandit = engine.initialize_bandit(&trajectory).await;

    // Should have the nudged value
    let dist = &bandit.context_arms["fixed_point"]["focused_repair"];
    assert!(
        (dist.alpha - 6.0).abs() < f64::EPSILON,
        "Expected alpha=6.0 (1.0 + 5.0), got {}",
        dist.alpha
    );
}

#[tokio::test]
async fn test_initialize_bandit_defaults_when_memory_disabled() {
    let mut config = test_config();
    config.memory_enabled = false;

    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        Arc::new(MockMemoryRepo::new()),
        Arc::new(MockOverseerMeasurer::new()),
        config,
    );

    let trajectory = test_trajectory();
    let bandit = engine.initialize_bandit(&trajectory).await;

    // Should still have default priors
    assert!(bandit.context_arms.contains_key("fixed_point"));
}

// -----------------------------------------------------------------------
// store_success_memory / store_failure_memory tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_store_success_memory_content() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let mut trajectory = test_trajectory();
    trajectory.strategy_log.push(StrategyEntry::new(
        StrategyKind::RetryWithFeedback,
        0,
        10_000,
        false,
    ));

    engine.store_success_memory(&trajectory, 0).await.unwrap();

    let memories = mem_repo.memories.lock().unwrap();
    assert_eq!(memories.len(), 1);
    let memory = &memories[0];
    assert!(memory.content.contains("SUCCESS"));
    assert!(memory.content.contains("retry_with_feedback"));
    assert_eq!(memory.namespace, "convergence");
    assert!(memory.metadata.tags.contains(&"convergence".to_string()));
    assert!(memory.metadata.tags.contains(&"success".to_string()));
    assert_eq!(memory.tier, MemoryTier::Semantic);
}

#[tokio::test]
async fn test_store_failure_memory_content() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let trajectory = test_trajectory();
    engine
        .store_failure_memory(&trajectory, "trapped")
        .await
        .unwrap();

    let memories = mem_repo.memories.lock().unwrap();
    assert_eq!(memories.len(), 1);
    let memory = &memories[0];
    assert!(memory.content.contains("FAILURE"));
    assert!(memory.content.contains("trapped"));
    assert_eq!(memory.namespace, "convergence");
    assert!(memory.metadata.tags.contains(&"failure".to_string()));
    assert_eq!(memory.tier, MemoryTier::Episodic);
}

#[tokio::test]
async fn test_store_failure_memory_increments_version_on_retry() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let trajectory = test_trajectory();

    // First failure stores at version 1
    engine
        .store_failure_memory(&trajectory, "exhausted")
        .await
        .unwrap();
    // Second failure (retry) should store at version 2
    engine
        .store_failure_memory(&trajectory, "trapped")
        .await
        .unwrap();

    let memories = mem_repo.memories.lock().unwrap();
    assert_eq!(memories.len(), 2);
    assert_eq!(memories[0].version, 1);
    assert_eq!(memories[1].version, 2);
    assert!(memories[0].content.contains("exhausted"));
    assert!(memories[1].content.contains("trapped"));
}

#[tokio::test]
async fn test_store_success_memory_increments_version_on_retry() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let trajectory = test_trajectory();

    engine.store_success_memory(&trajectory, 0).await.unwrap();
    engine.store_success_memory(&trajectory, 1).await.unwrap();

    let memories = mem_repo.memories.lock().unwrap();
    assert_eq!(memories.len(), 2);
    assert_eq!(memories[0].version, 1);
    assert_eq!(memories[1].version, 2);
}

// -----------------------------------------------------------------------
// persist_bandit_state tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_persist_bandit_state_serializes_correctly() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let trajectory = test_trajectory();
    let mut bandit = StrategyBandit::with_default_priors();
    bandit.nudge("fixed_point", "focused_repair", 3.0);

    engine
        .persist_bandit_state(&bandit, &trajectory)
        .await
        .unwrap();

    let memories = mem_repo.memories.lock().unwrap();
    assert_eq!(memories.len(), 1);
    let memory = &memories[0];
    assert!(
        memory
            .metadata
            .tags
            .contains(&"strategy-bandit".to_string())
    );

    // Verify deserializable
    let restored: StrategyBandit = serde_json::from_str(&memory.content).unwrap();
    let dist = &restored.context_arms["fixed_point"]["focused_repair"];
    assert!((dist.alpha - 4.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_persist_bandit_state_updates_in_place_on_retry() {
    let mem_repo = Arc::new(MockMemoryRepo::new());
    let engine = ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        mem_repo.clone(),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    );

    let trajectory = test_trajectory();

    // First persist — inserts
    let mut bandit = StrategyBandit::with_default_priors();
    bandit.nudge("fixed_point", "focused_repair", 3.0);
    engine
        .persist_bandit_state(&bandit, &trajectory)
        .await
        .unwrap();

    // Second persist — should update in place, not insert a new row
    bandit.nudge("fixed_point", "focused_repair", 5.0);
    engine
        .persist_bandit_state(&bandit, &trajectory)
        .await
        .unwrap();

    let memories = mem_repo.memories.lock().unwrap();
    assert_eq!(memories.len(), 1, "should update in place, not duplicate");

    // Verify the content was updated to the second bandit state
    let restored: StrategyBandit = serde_json::from_str(&memories[0].content).unwrap();
    let dist = &restored.context_arms["fixed_point"]["focused_repair"];
    // Initial alpha=1, +3 nudge, +5 nudge = 9.0
    assert!((dist.alpha - 9.0).abs() < f64::EPSILON);
}
// -----------------------------------------------------------------------
// budget calibration alert tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_finalize_emits_calibration_alert_when_p95_exceeds_budget() {
    let engine = test_engine();

    // Simple tier has max_tokens = 150_000. Overshoot threshold is 20%.
    // So max_allowed = 180_000. We need P95 > 180_000.
    // With 20 samples all at 200_000, P95 = 200_000 which exceeds 180_000.
    let bandit = StrategyBandit::with_default_priors();

    for _ in 0..20 {
        let mut trajectory = test_trajectory();
        trajectory.complexity = Some(Complexity::Simple);
        trajectory.budget.tokens_used = 200_000;

        let outcome = ConvergenceOutcome::Converged {
            trajectory_id: trajectory.id.to_string(),
            final_observation_sequence: 1,
        };

        engine
            .finalize(&mut trajectory, &outcome, &bandit)
            .await
            .unwrap();
    }

    let alerts = engine.calibration_alerts();
    assert!(
        !alerts.is_empty(),
        "Expected calibration alert for Simple tier exceeding budget"
    );
    assert!(alerts.iter().any(|a| a.tier == Complexity::Simple));
    let alert = alerts
        .iter()
        .find(|a| a.tier == Complexity::Simple)
        .unwrap();
    assert!(alert.overshoot_pct > 20.0);
}

#[tokio::test]
async fn test_finalize_no_calibration_alert_when_within_budget() {
    let engine = test_engine();

    // Simple tier has max_tokens = 150_000. Overshoot threshold is 20%.
    // max_allowed = 180_000. All samples at 100_000 => P95 = 100_000 < 180_000.
    let bandit = StrategyBandit::with_default_priors();

    for _ in 0..20 {
        let mut trajectory = test_trajectory();
        trajectory.complexity = Some(Complexity::Simple);
        trajectory.budget.tokens_used = 100_000;

        let outcome = ConvergenceOutcome::Converged {
            trajectory_id: trajectory.id.to_string(),
            final_observation_sequence: 1,
        };

        engine
            .finalize(&mut trajectory, &outcome, &bandit)
            .await
            .unwrap();
    }

    let alerts = engine.calibration_alerts();
    assert!(
        alerts.is_empty(),
        "Expected no calibration alerts when within budget, got {:?}",
        alerts
    );
}
