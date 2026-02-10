//! Convergence engine types — loop control, outcomes, configuration, and decomposition.

use super::*;

// ---------------------------------------------------------------------------
// LoopControl
// ---------------------------------------------------------------------------

/// Controls the convergence loop flow.
///
/// Returned by the engine's `check_loop_control` method after each observation.
/// Most variants are terminal (end the loop), except `Continue` (keep iterating),
/// `RequestExtension` (attempt a budget extension then continue), and `Decompose`
/// (transition to coordination phase).
#[derive(Debug, Clone)]
pub enum LoopControl {
    /// Keep iterating -- no termination condition met.
    Continue,
    /// The trajectory has reached the acceptance threshold and passed verification.
    Converged,
    /// The convergence budget has been fully consumed without converging.
    Exhausted,
    /// All escape strategies for a limit cycle have been exhausted.
    Trapped,
    /// Budget is nearly exhausted but the trajectory is converging; request more resources.
    RequestExtension,
    /// The task should be decomposed into subtasks.
    Decompose,
}

// ---------------------------------------------------------------------------
// ConvergenceOutcome
// ---------------------------------------------------------------------------

/// The final outcome of a convergence run.
///
/// Every trajectory terminates in one of these outcomes. The engine maps
/// terminal loop control signals to outcomes and emits appropriate events.
#[derive(Debug, Clone)]
pub enum ConvergenceOutcome {
    /// The trajectory converged to a satisfactory result.
    Converged {
        /// The trajectory that converged.
        trajectory_id: String,
        /// The sequence number of the final observation.
        final_observation_sequence: u32,
    },
    /// The budget was exhausted without converging.
    Exhausted {
        /// The trajectory that was exhausted.
        trajectory_id: String,
        /// The sequence number of the best observation, if any.
        best_observation_sequence: Option<u32>,
    },
    /// The trajectory is trapped in a limit cycle with no escape strategies remaining.
    Trapped {
        /// The trajectory that is trapped.
        trajectory_id: String,
        /// The type of attractor the trajectory is trapped in.
        attractor_type: AttractorType,
    },
    /// The task was decomposed into subtasks, each with its own trajectory.
    Decomposed {
        /// The parent trajectory that initiated decomposition.
        parent_trajectory_id: String,
        /// The trajectory IDs of the child subtask trajectories.
        child_trajectory_ids: Vec<String>,
    },
    /// A budget extension was requested but denied.
    BudgetDenied {
        /// The trajectory whose extension was denied.
        trajectory_id: String,
    },
}

// ---------------------------------------------------------------------------
// ConvergenceEngineConfig
// ---------------------------------------------------------------------------

/// Configuration for the convergence engine.
///
/// Controls default behavior, parallelism limits, and feature flags for the
/// convergence system. Assembled at engine initialization time.
#[derive(Debug, Clone)]
pub struct ConvergenceEngineConfig {
    /// The default convergence policy used when no priority hints are provided.
    pub default_policy: ConvergencePolicy,
    /// Maximum number of trajectories that may run in parallel.
    pub max_parallel_trajectories: usize,
    /// Whether the engine should proactively decompose tasks with narrow basins
    /// before entering the iteration loop.
    pub enable_proactive_decomposition: bool,
    /// Whether convergence memory (success/failure patterns) is enabled.
    pub memory_enabled: bool,
    /// Whether the engine emits events to the event bus.
    pub event_emission_enabled: bool,
}

// ---------------------------------------------------------------------------
// DecompositionDecision
// ---------------------------------------------------------------------------

/// Decision about whether/how to decompose a task.
///
/// Returned by the proactive decomposition check during the DECIDE phase.
/// `NoDecomposition` means the task should be attempted monolithically;
/// `Decompose` provides the subtask breakdown.
#[derive(Debug, Clone)]
pub enum DecompositionDecision {
    /// The task should not be decomposed.
    NoDecomposition,
    /// The task should be decomposed into the provided subtasks.
    Decompose {
        /// The subtasks to decompose into.
        subtasks: Vec<TaskDecomposition>,
    },
}

// ---------------------------------------------------------------------------
// TaskDecomposition
// ---------------------------------------------------------------------------

/// A single subtask from decomposition.
///
/// Each subtask receives a fraction of the parent's budget, its own
/// specification snapshot, and may declare dependencies on other subtasks.
#[derive(Debug, Clone)]
pub struct TaskDecomposition {
    /// Unique identifier for this subtask.
    pub subtask_id: String,
    /// Human-readable description of the subtask.
    pub description: String,
    /// The specification for this subtask, derived from the parent specification.
    pub specification: SpecificationSnapshot,
    /// Fraction of the parent budget allocated to this subtask (0.0 to 1.0).
    pub budget_fraction: f64,
    /// IDs of subtasks that must complete before this one can start.
    pub dependencies: Vec<String>,
}
