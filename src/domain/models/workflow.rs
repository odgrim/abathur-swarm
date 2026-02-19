//! Workflow and phase domain models for the phase orchestrator.
//!
//! A WorkflowDefinition is an immutable DAG of PhaseDefinitions that describes
//! how to execute a goal phase-by-phase. A WorkflowInstance is the mutable
//! runtime state tracking execution progress.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

use super::overmind::TaskDefinition;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    /// Overall workflow timeout in seconds.
    pub timeout_secs: u64,
    /// Whether to stop on first phase failure.
    pub fail_fast: bool,
    /// Maximum retries per phase.
    pub max_phase_retries: u32,
    /// Token budget for the entire workflow (0 = unlimited).
    pub token_budget: u64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 3600,
            fail_fast: false,
            max_phase_retries: 2,
            token_budget: 0,
        }
    }
}

// ============================================================================
// Phase Types & Verification
// ============================================================================

/// The type of work a phase performs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhaseType {
    /// Run tasks via DagExecutor (the common case).
    Execute,
    /// Run a verification gate.
    Verify,
    /// Invoke overmind to decompose further at runtime.
    Decompose,
    /// Invoke overmind to choose the next path.
    Decision,
    /// Dynamically create N parallel phases from decomposition.
    FanOut,
    /// Merge results from fan-out.
    Aggregate,
    /// Loop until verification passes.
    Iterative { max_iterations: u32 },
    /// Recursively execute a nested WorkflowDefinition.
    SubWorkflow,
}

/// Verification configuration for a phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseVerification {
    /// What to verify.
    pub description: String,
    /// Whether this verification blocks progress.
    pub is_blocking: bool,
}

// ============================================================================
// Workflow Definition (Immutable Blueprint)
// ============================================================================

/// A task definition within a phase, referencing the overmind's TaskDefinition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseTaskDefinition {
    /// The task definition from the overmind.
    pub task_def: TaskDefinition,
}

/// Definition of a single phase within a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseDefinition {
    /// Unique phase identifier.
    pub id: Uuid,
    /// Human-readable phase name.
    pub name: String,
    /// What type of phase this is.
    pub phase_type: PhaseType,
    /// Task definitions to execute in this phase.
    pub task_definitions: Vec<PhaseTaskDefinition>,
    /// Optional verification gate for this phase.
    pub verification: Option<PhaseVerification>,
    /// Nested workflow for SubWorkflow phases.
    pub sub_workflow: Option<Box<WorkflowDefinition>>,
}

/// An immutable DAG of phases that describes how to execute a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Unique workflow definition identifier.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// The goal this workflow serves.
    pub goal_id: Uuid,
    /// Phases in this workflow.
    pub phases: Vec<PhaseDefinition>,
    /// DAG edges: phase_id -> successor phase_ids.
    pub edges: HashMap<Uuid, Vec<Uuid>>,
    /// Configuration for this workflow.
    pub config: WorkflowConfig,
}

impl WorkflowDefinition {
    /// Create a new workflow definition.
    pub fn new(name: impl Into<String>, goal_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            goal_id,
            phases: Vec::new(),
            edges: HashMap::new(),
            config: WorkflowConfig::default(),
        }
    }

    /// Add a phase to the workflow.
    pub fn add_phase(&mut self, phase: PhaseDefinition) {
        self.phases.push(phase);
    }

    /// Add a dependency edge: `from` must complete before `to` can start.
    pub fn add_edge(&mut self, from: Uuid, to: Uuid) {
        self.edges.entry(from).or_default().push(to);
    }

    /// Get predecessor phases for a given phase (reverse lookup of edges).
    pub fn predecessors(&self, phase_id: Uuid) -> Vec<Uuid> {
        self.edges
            .iter()
            .filter(|(_, successors)| successors.contains(&phase_id))
            .map(|(pred, _)| *pred)
            .collect()
    }

    /// Get successor phases for a given phase.
    pub fn successors(&self, phase_id: Uuid) -> Vec<Uuid> {
        self.edges.get(&phase_id).cloned().unwrap_or_default()
    }

    /// Get root phases (no predecessors).
    pub fn root_phases(&self) -> Vec<Uuid> {
        let all_successors: HashSet<Uuid> = self.edges.values().flatten().copied().collect();
        self.phases
            .iter()
            .map(|p| p.id)
            .filter(|id| !all_successors.contains(id))
            .collect()
    }

    /// Validate that the phase DAG has no cycles using Kahn's algorithm.
    pub fn validate_dag(&self) -> Result<(), String> {
        let phase_ids: HashSet<Uuid> = self.phases.iter().map(|p| p.id).collect();

        // Check all edge references are valid
        for (from, tos) in &self.edges {
            if !phase_ids.contains(from) {
                return Err(format!("Edge source phase {} not found in phases", from));
            }
            for to in tos {
                if !phase_ids.contains(to) {
                    return Err(format!("Edge target phase {} not found in phases", to));
                }
            }
        }

        // Kahn's algorithm for cycle detection
        let mut in_degree: HashMap<Uuid, usize> = phase_ids.iter().map(|id| (*id, 0)).collect();
        for successors in self.edges.values() {
            for succ in successors {
                *in_degree.entry(*succ).or_default() += 1;
            }
        }

        let mut queue: VecDeque<Uuid> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut visited = 0usize;
        while let Some(node) = queue.pop_front() {
            visited += 1;
            if let Some(successors) = self.edges.get(&node) {
                for succ in successors {
                    if let Some(deg) = in_degree.get_mut(succ) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(*succ);
                        }
                    }
                }
            }
        }

        if visited != phase_ids.len() {
            Err("Cycle detected in workflow phase DAG".to_string())
        } else {
            Ok(())
        }
    }

    /// Compute the set of phases that are ready to run given completed phases.
    pub fn ready_phases(&self, completed: &HashSet<Uuid>) -> Vec<Uuid> {
        self.phases
            .iter()
            .map(|p| p.id)
            .filter(|id| !completed.contains(id))
            .filter(|id| {
                let preds = self.predecessors(*id);
                preds.iter().all(|pred| completed.contains(pred))
            })
            .collect()
    }
}

// ============================================================================
// Workflow & Phase Status
// ============================================================================

/// Status of a workflow instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    /// Created but not yet started.
    Pending,
    /// Currently executing phases.
    Running,
    /// All phases completed successfully.
    Completed,
    /// One or more phases failed and recovery exhausted.
    Failed,
    /// Workflow was canceled.
    Canceled,
}

impl WorkflowStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Canceled => write!(f, "canceled"),
        }
    }
}

/// Status of a phase instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    /// Waiting for predecessors.
    Pending,
    /// All predecessors completed, ready to start.
    Ready,
    /// Currently executing.
    Running,
    /// Running verification gate.
    Verifying,
    /// Completed successfully.
    Completed,
    /// Failed after all retries.
    Failed,
    /// Skipped (e.g., non-blocking verification).
    Skipped,
    /// Waiting for an overmind decision.
    AwaitingDecision,
}

impl PhaseStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }
}

impl std::fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Ready => write!(f, "ready"),
            Self::Running => write!(f, "running"),
            Self::Verifying => write!(f, "verifying"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
            Self::AwaitingDecision => write!(f, "awaiting_decision"),
        }
    }
}

// ============================================================================
// Workflow Instance (Mutable Runtime State)
// ============================================================================

/// Runtime state of a single phase within a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseInstance {
    /// The phase definition ID this instance tracks.
    pub phase_id: Uuid,
    /// Current status.
    pub status: PhaseStatus,
    /// Task IDs created for this phase (populated when phase starts).
    pub task_ids: Vec<Uuid>,
    /// Number of times this phase has been retried.
    pub retry_count: u32,
    /// Result of the verification gate (if any).
    pub verification_result: Option<bool>,
    /// Iteration count for Iterative phases.
    pub iteration_count: u32,
    /// When this phase started.
    pub started_at: Option<DateTime<Utc>>,
    /// When this phase completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message if failed.
    pub error: Option<String>,
}

impl PhaseInstance {
    /// Create a new pending phase instance.
    pub fn new(phase_id: Uuid) -> Self {
        Self {
            phase_id,
            status: PhaseStatus::Pending,
            task_ids: Vec::new(),
            retry_count: 0,
            verification_result: None,
            iteration_count: 0,
            started_at: None,
            completed_at: None,
            error: None,
        }
    }
}

/// Mutable runtime state for a workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    /// Unique instance identifier.
    pub id: Uuid,
    /// The workflow definition this is an instance of.
    pub workflow_id: Uuid,
    /// The goal this workflow serves.
    pub goal_id: Uuid,
    /// Current status.
    pub status: WorkflowStatus,
    /// Phase instances keyed by phase definition ID.
    pub phase_instances: HashMap<Uuid, PhaseInstance>,
    /// Total tokens consumed by this workflow.
    pub tokens_consumed: u64,
    /// When this instance was created.
    pub created_at: DateTime<Utc>,
    /// When this instance was last updated.
    pub updated_at: DateTime<Utc>,
    /// When this instance completed (success or failure).
    pub completed_at: Option<DateTime<Utc>>,
}

impl WorkflowInstance {
    /// Create a new workflow instance from a definition.
    pub fn new(definition: &WorkflowDefinition) -> Self {
        let now = Utc::now();
        let phase_instances = definition
            .phases
            .iter()
            .map(|p| (p.id, PhaseInstance::new(p.id)))
            .collect();

        Self {
            id: Uuid::new_v4(),
            workflow_id: definition.id,
            goal_id: definition.goal_id,
            status: WorkflowStatus::Pending,
            phase_instances,
            tokens_consumed: 0,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    /// Get the set of completed phase IDs.
    pub fn completed_phases(&self) -> HashSet<Uuid> {
        self.phase_instances
            .iter()
            .filter(|(_, inst)| inst.status == PhaseStatus::Completed)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get the set of failed phase IDs.
    pub fn failed_phases(&self) -> HashSet<Uuid> {
        self.phase_instances
            .iter()
            .filter(|(_, inst)| inst.status == PhaseStatus::Failed)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Check if all phases are in a terminal state.
    pub fn all_phases_terminal(&self) -> bool {
        self.phase_instances.values().all(|inst| inst.status.is_terminal())
    }

    /// Check if all phases completed successfully.
    pub fn all_phases_completed(&self) -> bool {
        self.phase_instances
            .values()
            .all(|inst| inst.status == PhaseStatus::Completed || inst.status == PhaseStatus::Skipped)
    }

    /// Check if any phase failed.
    pub fn any_phase_failed(&self) -> bool {
        self.phase_instances
            .values()
            .any(|inst| inst.status == PhaseStatus::Failed)
    }

    /// Map a task ID to the phase it belongs to (linear scan).
    pub fn phase_for_task(&self, task_id: Uuid) -> Option<Uuid> {
        self.phase_instances
            .iter()
            .find(|(_, inst)| inst.task_ids.contains(&task_id))
            .map(|(phase_id, _)| *phase_id)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_phase(name: &str, phase_type: PhaseType) -> PhaseDefinition {
        PhaseDefinition {
            id: Uuid::new_v4(),
            name: name.to_string(),
            phase_type,
            task_definitions: Vec::new(),
            verification: None,
            sub_workflow: None,
        }
    }

    #[test]
    fn test_workflow_dag_validation_no_cycles() {
        let mut wf = WorkflowDefinition::new("test", Uuid::new_v4());
        let p1 = make_phase("phase1", PhaseType::Execute);
        let p2 = make_phase("phase2", PhaseType::Execute);
        let p3 = make_phase("phase3", PhaseType::Execute);
        let p1_id = p1.id;
        let p2_id = p2.id;
        let p3_id = p3.id;
        wf.add_phase(p1);
        wf.add_phase(p2);
        wf.add_phase(p3);
        wf.add_edge(p1_id, p2_id);
        wf.add_edge(p2_id, p3_id);
        assert!(wf.validate_dag().is_ok());
    }

    #[test]
    fn test_workflow_dag_validation_with_cycle() {
        let mut wf = WorkflowDefinition::new("test", Uuid::new_v4());
        let p1 = make_phase("phase1", PhaseType::Execute);
        let p2 = make_phase("phase2", PhaseType::Execute);
        let p1_id = p1.id;
        let p2_id = p2.id;
        wf.add_phase(p1);
        wf.add_phase(p2);
        wf.add_edge(p1_id, p2_id);
        wf.add_edge(p2_id, p1_id);
        assert!(wf.validate_dag().is_err());
    }

    #[test]
    fn test_ready_phases_computation() {
        let mut wf = WorkflowDefinition::new("test", Uuid::new_v4());
        let p1 = make_phase("phase1", PhaseType::Execute);
        let p2 = make_phase("phase2", PhaseType::Execute);
        let p3 = make_phase("phase3", PhaseType::Execute);
        let p1_id = p1.id;
        let p2_id = p2.id;
        let p3_id = p3.id;
        wf.add_phase(p1);
        wf.add_phase(p2);
        wf.add_phase(p3);
        wf.add_edge(p1_id, p2_id);
        wf.add_edge(p1_id, p3_id);

        // Initially only p1 is ready (no predecessors)
        let ready = wf.ready_phases(&HashSet::new());
        assert_eq!(ready, vec![p1_id]);

        // After p1 completes, p2 and p3 are ready
        let mut completed = HashSet::new();
        completed.insert(p1_id);
        let mut ready = wf.ready_phases(&completed);
        ready.sort();
        let mut expected = vec![p2_id, p3_id];
        expected.sort();
        assert_eq!(ready, expected);
    }

    #[test]
    fn test_root_phases() {
        let mut wf = WorkflowDefinition::new("test", Uuid::new_v4());
        let p1 = make_phase("phase1", PhaseType::Execute);
        let p2 = make_phase("phase2", PhaseType::Execute);
        let p3 = make_phase("phase3", PhaseType::Execute);
        let p1_id = p1.id;
        let p2_id = p2.id;
        let p3_id = p3.id;
        wf.add_phase(p1);
        wf.add_phase(p2);
        wf.add_phase(p3);
        wf.add_edge(p1_id, p3_id);
        wf.add_edge(p2_id, p3_id);

        let mut roots = wf.root_phases();
        roots.sort();
        let mut expected = vec![p1_id, p2_id];
        expected.sort();
        assert_eq!(roots, expected);
    }

    #[test]
    fn test_workflow_instance_creation() {
        let mut wf = WorkflowDefinition::new("test", Uuid::new_v4());
        let p1 = make_phase("phase1", PhaseType::Execute);
        let p2 = make_phase("phase2", PhaseType::Verify);
        wf.add_phase(p1);
        wf.add_phase(p2);

        let instance = WorkflowInstance::new(&wf);
        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert_eq!(instance.phase_instances.len(), 2);
        assert!(instance.completed_phases().is_empty());
        assert!(!instance.all_phases_terminal());
    }

    #[test]
    fn test_phase_for_task_lookup() {
        let mut wf = WorkflowDefinition::new("test", Uuid::new_v4());
        let p1 = make_phase("phase1", PhaseType::Execute);
        let p1_id = p1.id;
        wf.add_phase(p1);

        let mut instance = WorkflowInstance::new(&wf);
        let task_id = Uuid::new_v4();
        instance.phase_instances.get_mut(&p1_id).unwrap().task_ids.push(task_id);

        assert_eq!(instance.phase_for_task(task_id), Some(p1_id));
        assert_eq!(instance.phase_for_task(Uuid::new_v4()), None);
    }
}
