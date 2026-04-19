//! Tests for `convergence_engine::prepare`.

use uuid::Uuid;

use crate::domain::models::convergence::*;

use super::test_engine;

// -----------------------------------------------------------------------
// prepare tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_prepare_creates_trajectory() {
    let engine = test_engine();
    let submission = TaskSubmission::new(
            "Implement user authentication with bcrypt password hashing and JWT tokens for session management".to_string(),
        );

    let (trajectory, infra) = engine.prepare(&submission, Uuid::new_v4()).await.unwrap();

    assert_eq!(trajectory.phase, ConvergencePhase::Preparing);
    assert!(trajectory.observations.is_empty());
    assert!(trajectory.strategy_log.is_empty());
    assert!(!trajectory.specification.effective.content.is_empty());
    // Infrastructure should be initialized from the submission
    assert!(infra.acceptance_tests.is_empty()); // No discovered tests
}

#[tokio::test]
async fn test_prepare_applies_priority_hint() {
    let engine = test_engine();
    let mut submission = TaskSubmission::new(
            "Implement a simple health check endpoint for the API with a detailed specification that covers all the necessary aspects of monitoring".to_string(),
        );
    submission.priority_hint = Some(PriorityHint::Fast);

    let (trajectory, _) = engine.prepare(&submission, Uuid::new_v4()).await.unwrap();

    assert!(
        (trajectory.policy.acceptance_threshold - 0.85).abs() < f64::EPSILON,
        "Fast hint should set threshold to 0.85"
    );
    assert!(trajectory.policy.skip_expensive_overseers);
}

#[tokio::test]
async fn test_prepare_folds_constraints_into_spec() {
    let engine = test_engine();
    let mut submission = TaskSubmission::new(
            "Build a REST API for user management with proper authentication and authorization controls".to_string(),
        );
    submission.constraints = vec![
        "Must use bcrypt for passwords".to_string(),
        "Must validate all inputs".to_string(),
    ];

    let (trajectory, _) = engine.prepare(&submission, Uuid::new_v4()).await.unwrap();

    assert_eq!(trajectory.specification.amendments.len(), 2);
    assert!(
        trajectory
            .specification
            .effective
            .content
            .contains("Must use bcrypt")
    );
}
