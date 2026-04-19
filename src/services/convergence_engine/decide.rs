//! Convergence engine -- 9.1/9.2/9.3 decide phase.
//!
//! Proactive decomposition, subtask coordination, integration trajectory.

use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::*;
use crate::domain::models::task::Complexity;
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

use super::{ConvergenceDomainEvent, ConvergenceEngine, OverseerMeasurer};

impl<T: TrajectoryRepository, M: MemoryRepository, O: OverseerMeasurer> ConvergenceEngine<T, M, O> {
    // -----------------------------------------------------------------------
    // 9.2 decompose_and_coordinate -- Decomposition flow
    // -----------------------------------------------------------------------

    /// Decompose a task into subtasks and coordinate their convergence (spec 9.2, 9.3).
    ///
    /// Full decomposition flow:
    /// 1. Propose decomposition into subtasks.
    /// 2. Reserve 25% of the parent budget for the integration trajectory.
    /// 3. Allocate remaining 75% across child subtasks.
    /// 4. Converge each child through the full engine (`self.converge()`).
    /// 5. If any child fails, return Exhausted immediately.
    /// 6. After all children converge, run a mandatory integration trajectory
    ///    using the reserved budget.
    /// 7. Return the final outcome.
    pub async fn decompose_and_coordinate(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<ConvergenceOutcome> {
        // 1. Propose decomposition
        let decomposition = self.propose_decomposition(trajectory);

        // 2. Reserve 25% of parent budget for integration (spec 9.3)
        let integration_budget = trajectory.budget.scale(0.25);

        // 3. Allocate remaining 75% across child subtasks
        let child_budgets = allocate_decomposed_budget(&trajectory.budget, &decomposition);

        self.emit_event(ConvergenceEvent::DecompositionTriggered {
            parent_trajectory_id: trajectory.id.to_string(),
            child_count: decomposition.len(),
        });

        // 4. Converge each child through the full engine
        let mut child_ids = Vec::new();
        let empty_infra = ConvergenceInfrastructure::default();

        for (subtask, budget) in decomposition.iter().zip(child_budgets.iter()) {
            let spec = SpecificationEvolution::new(subtask.specification.clone());
            let child = Trajectory::new(
                trajectory.task_id,
                trajectory.goal_id,
                spec,
                budget.clone(),
                trajectory.policy.clone(),
            );
            child_ids.push(child.id.to_string());

            // Run full convergence for this child.
            // Box::pin is required because converge -> decompose_and_coordinate -> converge
            // forms a recursive async call chain.
            let child_outcome = Box::pin(self.converge(child, &empty_infra)).await?;

            // 5. If any child fails, return Exhausted immediately
            if !matches!(&child_outcome, ConvergenceOutcome::Converged { .. }) {
                self.event_sink
                    .emit(ConvergenceDomainEvent::DecompositionChildFailed {
                        parent_trajectory_id: trajectory.id.to_string(),
                        child_subtask: subtask.subtask_id.to_string(),
                    })
                    .await;
                return Ok(ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: trajectory.best_observation().map(|o| o.sequence),
                });
            }
        }

        // 6. All children converged -- run mandatory integration trajectory (spec 9.3).
        let integration_outcome = self
            .run_integration_trajectory(trajectory, &child_ids, integration_budget)
            .await?;

        match &integration_outcome {
            ConvergenceOutcome::Converged { .. } => {
                // 7. Integration succeeded -- return Decomposed with all child IDs
                Ok(ConvergenceOutcome::Decomposed {
                    parent_trajectory_id: trajectory.id.to_string(),
                    child_trajectory_ids: child_ids,
                })
            }
            _ => {
                // Integration failed
                self.event_sink
                    .emit(ConvergenceDomainEvent::DecompositionIntegrationFailed {
                        parent_trajectory_id: trajectory.id.to_string(),
                    })
                    .await;
                Ok(ConvergenceOutcome::Exhausted {
                    trajectory_id: trajectory.id.to_string(),
                    best_observation_sequence: trajectory.best_observation().map(|o| o.sequence),
                })
            }
        }
    }

    /// Run the mandatory integration trajectory after all children converge (spec 9.3).
    ///
    /// The integration trajectory verifies that the combined child outputs form
    /// a coherent whole. It receives 25% of the parent budget and a specification
    /// that references all child trajectory IDs.
    pub(super) async fn run_integration_trajectory(
        &self,
        parent_trajectory: &Trajectory,
        child_ids: &[String],
        integration_budget: ConvergenceBudget,
    ) -> DomainResult<ConvergenceOutcome> {
        let integration_description = format!(
            "Integration of decomposed subtasks for: {}. Child trajectories: [{}]",
            parent_trajectory.specification.effective.content,
            child_ids.join(", "),
        );

        let integration_spec =
            SpecificationEvolution::new(SpecificationSnapshot::new(integration_description));

        let integration_trajectory = Trajectory::new(
            parent_trajectory.task_id,
            parent_trajectory.goal_id,
            integration_spec,
            integration_budget,
            parent_trajectory.policy.clone(),
        );

        let empty_infra = ConvergenceInfrastructure::default();
        Box::pin(self.converge(integration_trajectory, &empty_infra)).await
    }

    // -----------------------------------------------------------------------
    // 9.1 maybe_decompose_proactively -- Proactive decomposition check
    // -----------------------------------------------------------------------

    /// Check whether the task should be proactively decomposed (spec 9.1).
    ///
    /// This runs during the DECIDE phase before entering the iteration loop.
    /// A task is a candidate for proactive decomposition when:
    /// - The basin is narrow (many starting points do not converge).
    /// - The estimated convergence cost exceeds the allocated budget.
    /// - The task complexity is Complex.
    ///
    /// Returns `Some(outcome)` if decomposition was triggered, `None` to proceed.
    pub async fn maybe_decompose_proactively(
        &self,
        trajectory: &mut Trajectory,
    ) -> DomainResult<Option<ConvergenceOutcome>> {
        let basin = estimate_basin_width(
            &trajectory.specification.effective.content,
            !trajectory
                .specification
                .effective
                .success_criteria
                .is_empty(),
            false,
            !trajectory.specification.effective.constraints.is_empty(),
            !trajectory.specification.effective.anti_patterns.is_empty(),
            false,
        );

        // Only consider proactive decomposition for narrow basins
        if basin.classification != BasinClassification::Narrow {
            return Ok(None);
        }

        // Estimate convergence cost
        let estimate = estimate_convergence_heuristic(Complexity::Complex, &basin);

        // If the estimated cost exceeds the budget, recommend decomposition
        if estimate.expected_tokens > trajectory.budget.max_tokens
            || estimate.convergence_probability < 0.4
        {
            self.emit_event(ConvergenceEvent::DecompositionRecommended {
                task_id: trajectory.task_id.to_string(),
                subtask_count: 0, // Unknown until decomposition is proposed
                savings_estimate: 1.0 - estimate.convergence_probability,
            });

            let outcome = self.decompose_and_coordinate(trajectory).await?;
            return Ok(Some(outcome));
        }

        Ok(None)
    }

    /// Propose a task decomposition.
    ///
    /// Without an LLM, creates a simple 2-way split based on the
    /// specification content.
    pub(super) fn propose_decomposition(&self, trajectory: &Trajectory) -> Vec<TaskDecomposition> {
        let spec = &trajectory.specification.effective;
        vec![
            TaskDecomposition {
                subtask_id: Uuid::new_v4().to_string(),
                description: format!("Part 1 of: {}", spec.content),
                specification: SpecificationSnapshot::new(format!("Part 1 of: {}", spec.content)),
                budget_fraction: 0.5,
                dependencies: vec![],
            },
            TaskDecomposition {
                subtask_id: Uuid::new_v4().to_string(),
                description: format!("Part 2 of: {}", spec.content),
                specification: SpecificationSnapshot::new(format!("Part 2 of: {}", spec.content)),
                budget_fraction: 0.5,
                dependencies: vec![],
            },
        ]
    }
}
