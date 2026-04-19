//! Convergence engine -- 6.2 prepare phase.
//!
//! Basin estimation, budget allocation, policy assembly, trajectory
//! initialization from a `TaskSubmission`.

use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::*;
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

use super::{ConvergenceEngine, OverseerMeasurer};

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> ConvergenceEngine<T, M, O> {
    // -----------------------------------------------------------------------
    // 6.2 prepare -- Prepare convergence infrastructure from a TaskSubmission
    // -----------------------------------------------------------------------

    /// Prepare convergence infrastructure from a task submission (spec 6.2).
    ///
    /// This is the SETUP + PREPARE phase combined:
    /// 1. Estimate basin width from the submission's specification signals.
    /// 2. Allocate a convergence budget based on inferred complexity.
    /// 3. Apply basin width adjustments to budget and policy.
    /// 4. Apply priority hint adjustments (if any).
    /// 5. Build convergence infrastructure from discovered project assets.
    /// 6. Fold submission constraints and anti-patterns into the specification
    ///    as amendments.
    /// 7. Create the trajectory.
    /// 8. Initialize the strategy bandit from memory (if enabled).
    ///
    /// Returns the prepared `Trajectory` and the convergence `ConvergenceInfrastructure`.
    pub async fn prepare(
        &self,
        submission: &TaskSubmission,
        task_id: Uuid,
    ) -> DomainResult<(Trajectory, ConvergenceInfrastructure)> {
        // 1. Estimate basin width
        let infra = &submission.discovered_infrastructure;
        let basin = estimate_basin_width(
            &submission.description,
            !infra.acceptance_tests.is_empty(),
            !infra.examples.is_empty(),
            !infra.invariants.is_empty(),
            !infra.anti_examples.is_empty(),
            !infra.context_files.is_empty(),
        );

        // 2. Allocate budget from complexity
        let mut budget = allocate_budget(submission.inferred_complexity);

        // 3. Assemble default policy
        let mut policy = self.config.default_policy.clone();

        // 4. Apply basin width adjustments
        apply_basin_width(&basin, &mut budget, &mut policy);

        // 5. Apply priority hint overlay (if any)
        if let Some(hint) = submission.priority_hint {
            hint.apply(&mut policy, &mut budget);
            policy.priority_hint = Some(hint);
        }

        // 6. Build convergence infrastructure
        let mut convergence_infra =
            ConvergenceInfrastructure::from_discovered(&submission.discovered_infrastructure);
        convergence_infra.merge_user_references(&submission.references);
        convergence_infra.add_invariants(&submission.constraints);
        convergence_infra.add_anti_patterns(&submission.anti_patterns);

        // 7. Build the specification and create trajectory
        let spec_snapshot = SpecificationSnapshot::new(submission.description.clone());
        let mut spec_evolution = SpecificationEvolution::new(spec_snapshot);

        // Fold constraints into specification as amendments
        for constraint in &submission.constraints {
            spec_evolution.add_amendment(SpecificationAmendment::new(
                AmendmentSource::SubmissionConstraint,
                constraint.clone(),
                "User-provided constraint from task submission",
            ));
        }

        let mut trajectory =
            Trajectory::new(task_id, submission.goal_id, spec_evolution, budget, policy);
        trajectory.complexity = Some(submission.inferred_complexity);

        // 8. Persist the trajectory
        self.trajectory_store.save(&trajectory).await?;

        Ok((trajectory, convergence_infra))
    }
}
