//! Workflow builder service.
//!
//! Converts a `GoalDecompositionDecision` from the Overmind into a
//! `WorkflowDefinition` that the phase orchestrator can execute.
//!
//! Maps `DecompositionStrategy` to workflow shapes:
//! - `Sequential` -- one phase per dependency layer (topological sort of tasks)
//! - `Parallel` -- single Execute phase with all tasks
//! - `Hybrid` -- cluster tasks by dependency groups into phases
//! - `ResearchFirst` -- Research -> Plan -> Implement -> Review phases
//! - `Incremental` -- iterative phases with verification gates

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::domain::models::overmind::{
    DecompositionStrategy, GoalDecompositionDecision, TaskDefinition, VerificationPoint,
};
use crate::domain::models::workflow::{
    PhaseDefinition, PhaseTaskDefinition, PhaseType, PhaseVerification, WorkflowConfig,
    WorkflowDefinition,
};
use crate::domain::models::workflow_template::{OutputDelivery, PhaseDependency, WorkflowTemplate};

/// Build a `WorkflowDefinition` from a goal decomposition decision.
pub fn build_workflow_from_decomposition(
    goal_id: Uuid,
    goal_name: &str,
    decision: &GoalDecompositionDecision,
    config: WorkflowConfig,
) -> WorkflowDefinition {
    let mut workflow = WorkflowDefinition::new(
        format!("Workflow for: {}", goal_name),
        goal_id,
    );
    workflow.config = config;

    match decision.strategy {
        DecompositionStrategy::Sequential => {
            build_sequential(&mut workflow, &decision.tasks, &decision.verification_points);
        }
        DecompositionStrategy::Parallel => {
            build_parallel(&mut workflow, &decision.tasks, &decision.verification_points);
        }
        DecompositionStrategy::Hybrid => {
            build_hybrid(&mut workflow, &decision.tasks, &decision.verification_points);
        }
        DecompositionStrategy::ResearchFirst => {
            build_research_first(&mut workflow, &decision.tasks, &decision.verification_points);
        }
        DecompositionStrategy::Incremental => {
            build_incremental(&mut workflow, &decision.tasks, &decision.verification_points);
        }
    }

    workflow
}

/// Build a `WorkflowDefinition` from a `WorkflowTemplate`.
///
/// Maps each `WorkflowPhase` → `PhaseDefinition` with `PhaseType::Execute`,
/// wiring edges according to each phase's `PhaseDependency`.
///
/// - `Root` phases get no predecessor edges (they start immediately).
/// - `Sequential` phases get a single edge from the immediately preceding phase.
/// - `AllPrevious` phases get edges from every preceding phase.
///
/// When `template.output_delivery == OutputDelivery::PullRequest`, a final
/// `PhaseType::Verify` gate is appended after the last phase to signal PR
/// readiness. This is a pure mapping — no LLM is invoked.
pub fn build_workflow_from_template(
    goal_id: Uuid,
    goal_title: &str,
    template: &WorkflowTemplate,
    config: WorkflowConfig,
) -> WorkflowDefinition {
    use crate::domain::models::TaskPriority;

    let mut workflow = WorkflowDefinition::new(
        format!("Workflow for: {}", goal_title),
        goal_id,
    );
    workflow.config = config;

    let mut phase_ids: Vec<Uuid> = Vec::new();

    for phase in &template.phases {
        let task_def = TaskDefinition {
            title: phase.name.clone(),
            description: format!("{}\n\nRole: {}", phase.description, phase.role),
            agent_type: None,
            priority: TaskPriority::Normal,
            depends_on: Vec::new(),
            needs_worktree: !phase.read_only,
            estimated_complexity: 3,
            acceptance_criteria: Vec::new(),
        };

        let phase_def = PhaseDefinition {
            id: Uuid::new_v4(),
            name: phase.name.clone(),
            phase_type: PhaseType::Execute,
            task_definitions: vec![PhaseTaskDefinition { task_def }],
            verification: None,
            sub_workflow: None,
        };

        let phase_id = phase_def.id;
        workflow.add_phase(phase_def);

        match phase.dependency {
            PhaseDependency::Root => {}
            PhaseDependency::Sequential => {
                if let Some(&prev_id) = phase_ids.last() {
                    workflow.add_edge(prev_id, phase_id);
                }
            }
            PhaseDependency::AllPrevious => {
                for &prev_id in &phase_ids {
                    workflow.add_edge(prev_id, phase_id);
                }
            }
        }

        phase_ids.push(phase_id);
    }

    // Append a PR-gate Verify phase for pull-request delivery workflows.
    if template.output_delivery == OutputDelivery::PullRequest && !phase_ids.is_empty() {
        let verify_phase = PhaseDefinition {
            id: Uuid::new_v4(),
            name: "pr-gate".to_string(),
            phase_type: PhaseType::Verify,
            task_definitions: Vec::new(),
            verification: Some(PhaseVerification {
                description: "All phases complete; ready for pull request creation.".to_string(),
                is_blocking: true,
            }),
            sub_workflow: None,
        };
        let verify_id = verify_phase.id;
        workflow.add_phase(verify_phase);
        if let Some(&last_id) = phase_ids.last() {
            workflow.add_edge(last_id, verify_id);
        }
    }

    workflow
}

/// Sequential: one phase per dependency layer (topological sort of tasks).
fn build_sequential(
    workflow: &mut WorkflowDefinition,
    tasks: &[TaskDefinition],
    verification_points: &[VerificationPoint],
) {
    let layers = topological_layers(tasks);

    let mut prev_phase_id: Option<Uuid> = None;
    for (i, layer) in layers.iter().enumerate() {
        let phase = make_execute_phase(
            &format!("Phase {} - Execute", i + 1),
            layer,
        );
        let phase_id = phase.id;
        workflow.add_phase(phase);

        if let Some(prev) = prev_phase_id {
            workflow.add_edge(prev, phase_id);
        }

        // Insert verification phases after this layer if any verification points apply
        let layer_titles: HashSet<&str> = layer.iter().map(|t| t.title.as_str()).collect();
        let applicable_vps: Vec<&VerificationPoint> = verification_points
            .iter()
            .filter(|vp| vp.after_tasks.iter().all(|t| layer_titles.contains(t.as_str())))
            .collect();

        if !applicable_vps.is_empty() {
            let verify_phase = make_verify_phase(
                &format!("Phase {} - Verify", i + 1),
                &applicable_vps,
            );
            let verify_id = verify_phase.id;
            workflow.add_phase(verify_phase);
            workflow.add_edge(phase_id, verify_id);
            prev_phase_id = Some(verify_id);
        } else {
            prev_phase_id = Some(phase_id);
        }
    }
}

/// Parallel: single Execute phase with all tasks.
fn build_parallel(
    workflow: &mut WorkflowDefinition,
    tasks: &[TaskDefinition],
    verification_points: &[VerificationPoint],
) {
    let phase = make_execute_phase("Execute - All Tasks", tasks);
    let phase_id = phase.id;
    workflow.add_phase(phase);

    if !verification_points.is_empty() {
        let verify_phase = make_verify_phase("Verify - All", &verification_points.iter().collect::<Vec<_>>());
        let verify_id = verify_phase.id;
        workflow.add_phase(verify_phase);
        workflow.add_edge(phase_id, verify_id);
    }
}

/// Hybrid: cluster tasks by dependency groups into phases.
fn build_hybrid(
    workflow: &mut WorkflowDefinition,
    tasks: &[TaskDefinition],
    verification_points: &[VerificationPoint],
) {
    // Build adjacency from depends_on titles
    let title_to_idx: HashMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.title.as_str(), i))
        .collect();

    // Compute dependency layers using topological sort
    let layers = topological_layers(tasks);

    if layers.len() <= 1 {
        // All tasks are independent or single layer -- treat as parallel
        build_parallel(workflow, tasks, verification_points);
        return;
    }

    // Each layer becomes a phase; layers are chained sequentially
    let mut prev_phase_id: Option<Uuid> = None;
    for (i, layer) in layers.iter().enumerate() {
        let phase = make_execute_phase(
            &format!("Layer {} ({} tasks)", i + 1, layer.len()),
            layer,
        );
        let phase_id = phase.id;
        workflow.add_phase(phase);

        if let Some(prev) = prev_phase_id {
            workflow.add_edge(prev, phase_id);
        }
        prev_phase_id = Some(phase_id);
    }

    // Add verification at the end
    let _ = title_to_idx; // used implicitly by topological_layers
    if !verification_points.is_empty() {
        let verify_phase = make_verify_phase("Final Verification", &verification_points.iter().collect::<Vec<_>>());
        let verify_id = verify_phase.id;
        workflow.add_phase(verify_phase);
        if let Some(prev) = prev_phase_id {
            workflow.add_edge(prev, verify_id);
        }
    }
}

/// ResearchFirst: Research -> Plan -> Implement -> Review phases.
fn build_research_first(
    workflow: &mut WorkflowDefinition,
    tasks: &[TaskDefinition],
    verification_points: &[VerificationPoint],
) {
    // Classify tasks into research, planning, implementation, review
    let (research, plan, implement, review) = classify_tasks(tasks);

    let mut prev_phase_id: Option<Uuid> = None;

    let phase_groups: Vec<(&str, Vec<&TaskDefinition>)> = vec![
        ("Research", research),
        ("Plan", plan),
        ("Implement", implement),
        ("Review", review),
    ];

    for (name, group) in phase_groups {
        if group.is_empty() {
            continue;
        }

        let owned: Vec<TaskDefinition> = group.into_iter().cloned().collect();
        let phase = make_execute_phase(name, &owned);
        let phase_id = phase.id;
        workflow.add_phase(phase);

        if let Some(prev) = prev_phase_id {
            workflow.add_edge(prev, phase_id);
        }
        prev_phase_id = Some(phase_id);

        // Insert verification after each non-empty phase if applicable
        let phase_titles: HashSet<&str> = owned.iter().map(|t| t.title.as_str()).collect();
        let applicable: Vec<&VerificationPoint> = verification_points
            .iter()
            .filter(|vp| vp.after_tasks.iter().all(|t| phase_titles.contains(t.as_str())))
            .collect();
        if !applicable.is_empty() {
            let verify_phase = make_verify_phase(
                &format!("{} Verification", name),
                &applicable,
            );
            let verify_id = verify_phase.id;
            workflow.add_phase(verify_phase);
            workflow.add_edge(phase_id, verify_id);
            prev_phase_id = Some(verify_id);
        }
    }
}

/// Incremental: iterative phases with verification gates.
fn build_incremental(
    workflow: &mut WorkflowDefinition,
    tasks: &[TaskDefinition],
    verification_points: &[VerificationPoint],
) {
    let layers = topological_layers(tasks);

    let mut prev_phase_id: Option<Uuid> = None;
    for (i, layer) in layers.iter().enumerate() {
        // Each increment is an iterative phase with a verification gate
        let mut phase = make_execute_phase(
            &format!("Increment {}", i + 1),
            layer,
        );

        // Make iterative phases with verification
        phase.phase_type = PhaseType::Iterative { max_iterations: 3 };

        let layer_titles: HashSet<&str> = layer.iter().map(|t| t.title.as_str()).collect();
        let applicable: Vec<&VerificationPoint> = verification_points
            .iter()
            .filter(|vp| vp.after_tasks.iter().any(|t| layer_titles.contains(t.as_str())))
            .collect();

        if !applicable.is_empty() {
            phase.verification = Some(PhaseVerification {
                description: applicable
                    .iter()
                    .map(|vp| vp.verify.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
                is_blocking: applicable.iter().any(|vp| vp.is_blocking),
            });
        }

        let phase_id = phase.id;
        workflow.add_phase(phase);

        if let Some(prev) = prev_phase_id {
            workflow.add_edge(prev, phase_id);
        }
        prev_phase_id = Some(phase_id);
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Create an Execute phase from task definitions.
fn make_execute_phase(name: &str, tasks: &[TaskDefinition]) -> PhaseDefinition {
    PhaseDefinition {
        id: Uuid::new_v4(),
        name: name.to_string(),
        phase_type: PhaseType::Execute,
        task_definitions: tasks
            .iter()
            .map(|t| PhaseTaskDefinition {
                task_def: t.clone(),
            })
            .collect(),
        verification: None,
        sub_workflow: None,
    }
}

/// Create a Verify phase from verification points.
fn make_verify_phase(name: &str, vps: &[&VerificationPoint]) -> PhaseDefinition {
    let description = vps
        .iter()
        .map(|vp| vp.verify.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let is_blocking = vps.iter().any(|vp| vp.is_blocking);

    PhaseDefinition {
        id: Uuid::new_v4(),
        name: name.to_string(),
        phase_type: PhaseType::Verify,
        task_definitions: Vec::new(),
        verification: Some(PhaseVerification {
            description,
            is_blocking,
        }),
        sub_workflow: None,
    }
}

/// Compute topological layers of tasks based on `depends_on` (title-based).
/// Each layer contains tasks whose dependencies are all in previous layers.
fn topological_layers(tasks: &[TaskDefinition]) -> Vec<Vec<TaskDefinition>> {
    let title_to_idx: HashMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.title.as_str(), i))
        .collect();

    // Compute in-degree for each task
    let mut in_degree = vec![0usize; tasks.len()];
    let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();

    for (i, task) in tasks.iter().enumerate() {
        for dep_title in &task.depends_on {
            if let Some(&dep_idx) = title_to_idx.get(dep_title.as_str()) {
                in_degree[i] += 1;
                dependents.entry(dep_idx).or_default().push(i);
            }
        }
    }

    let mut layers = Vec::new();
    let mut remaining: HashSet<usize> = (0..tasks.len()).collect();

    while !remaining.is_empty() {
        let layer_indices: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|&i| in_degree[i] == 0)
            .collect();

        if layer_indices.is_empty() {
            // Circular dependency or unresolvable deps -- put all remaining in one layer
            let layer: Vec<TaskDefinition> = remaining
                .iter()
                .map(|&i| tasks[i].clone())
                .collect();
            layers.push(layer);
            break;
        }

        let layer: Vec<TaskDefinition> = layer_indices
            .iter()
            .map(|&i| tasks[i].clone())
            .collect();
        layers.push(layer);

        for &idx in &layer_indices {
            remaining.remove(&idx);
            if let Some(deps) = dependents.get(&idx) {
                for &dep_idx in deps {
                    if in_degree[dep_idx] > 0 {
                        in_degree[dep_idx] -= 1;
                    }
                }
            }
        }
    }

    layers
}

/// Classify tasks into research, planning, implementation, and review buckets.
fn classify_tasks(tasks: &[TaskDefinition]) -> (
    Vec<&TaskDefinition>,
    Vec<&TaskDefinition>,
    Vec<&TaskDefinition>,
    Vec<&TaskDefinition>,
) {
    let mut research = Vec::new();
    let mut plan = Vec::new();
    let mut implement = Vec::new();
    let mut review = Vec::new();

    for task in tasks {
        let title_lower = task.title.to_lowercase();
        let agent = task.agent_type.as_deref().unwrap_or("");

        if title_lower.contains("research")
            || title_lower.contains("investigate")
            || title_lower.contains("analyze")
            || title_lower.contains("explore")
            || agent.contains("research")
        {
            research.push(task);
        } else if title_lower.contains("plan")
            || title_lower.contains("design")
            || title_lower.contains("architect")
            || agent.contains("architect")
        {
            plan.push(task);
        } else if title_lower.contains("review")
            || title_lower.contains("test")
            || title_lower.contains("verify")
            || title_lower.contains("validate")
            || agent.contains("reviewer")
        {
            review.push(task);
        } else {
            implement.push(task);
        }
    }

    (research, plan, implement, review)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::overmind::DecisionMetadata;
    use crate::domain::models::TaskPriority;

    fn make_task(title: &str, depends_on: Vec<&str>) -> TaskDefinition {
        TaskDefinition {
            title: title.to_string(),
            description: format!("Description for {}", title),
            agent_type: None,
            priority: TaskPriority::Normal,
            depends_on: depends_on.into_iter().map(String::from).collect(),
            needs_worktree: false,
            estimated_complexity: 2,
            acceptance_criteria: vec![],
        }
    }

    fn make_decision(
        strategy: DecompositionStrategy,
        tasks: Vec<TaskDefinition>,
        verification_points: Vec<VerificationPoint>,
    ) -> GoalDecompositionDecision {
        GoalDecompositionDecision {
            metadata: DecisionMetadata::new(0.9, "test"),
            strategy,
            tasks,
            verification_points,
            execution_hints: vec![],
        }
    }

    #[test]
    fn test_sequential_workflow() {
        let tasks = vec![
            make_task("Task A", vec![]),
            make_task("Task B", vec!["Task A"]),
            make_task("Task C", vec!["Task B"]),
        ];
        let decision = make_decision(DecompositionStrategy::Sequential, tasks, vec![]);
        let wf = build_workflow_from_decomposition(Uuid::new_v4(), "Test", &decision, WorkflowConfig::default());

        assert!(wf.validate_dag().is_ok());
        // Should have 3 phases (one per layer)
        assert_eq!(wf.phases.len(), 3);
        // Root should be exactly 1
        assert_eq!(wf.root_phases().len(), 1);
    }

    #[test]
    fn test_parallel_workflow() {
        let tasks = vec![
            make_task("Task A", vec![]),
            make_task("Task B", vec![]),
            make_task("Task C", vec![]),
        ];
        let decision = make_decision(DecompositionStrategy::Parallel, tasks, vec![]);
        let wf = build_workflow_from_decomposition(Uuid::new_v4(), "Test", &decision, WorkflowConfig::default());

        assert!(wf.validate_dag().is_ok());
        // Single phase with all tasks
        assert_eq!(wf.phases.len(), 1);
        assert_eq!(wf.phases[0].task_definitions.len(), 3);
    }

    #[test]
    fn test_hybrid_workflow() {
        let tasks = vec![
            make_task("Task A", vec![]),
            make_task("Task B", vec![]),
            make_task("Task C", vec!["Task A", "Task B"]),
        ];
        let decision = make_decision(DecompositionStrategy::Hybrid, tasks, vec![]);
        let wf = build_workflow_from_decomposition(Uuid::new_v4(), "Test", &decision, WorkflowConfig::default());

        assert!(wf.validate_dag().is_ok());
        // Layer 1: A, B (parallel); Layer 2: C (depends on both)
        assert_eq!(wf.phases.len(), 2);
    }

    #[test]
    fn test_research_first_workflow() {
        let tasks = vec![
            make_task("Research API options", vec![]),
            make_task("Design system architecture", vec!["Research API options"]),
            make_task("Implement feature X", vec!["Design system architecture"]),
            make_task("Review implementation", vec!["Implement feature X"]),
        ];
        let decision = make_decision(DecompositionStrategy::ResearchFirst, tasks, vec![]);
        let wf = build_workflow_from_decomposition(Uuid::new_v4(), "Test", &decision, WorkflowConfig::default());

        assert!(wf.validate_dag().is_ok());
        // Should have 4 phases: Research, Plan, Implement, Review
        assert_eq!(wf.phases.len(), 4);
    }

    #[test]
    fn test_incremental_workflow() {
        let tasks = vec![
            make_task("Task A", vec![]),
            make_task("Task B", vec!["Task A"]),
        ];
        let vps = vec![VerificationPoint {
            after_tasks: vec!["Task A".to_string()],
            verify: "Check Task A output".to_string(),
            is_blocking: true,
        }];
        let decision = make_decision(DecompositionStrategy::Incremental, tasks, vps);
        let wf = build_workflow_from_decomposition(Uuid::new_v4(), "Test", &decision, WorkflowConfig::default());

        assert!(wf.validate_dag().is_ok());
        // 2 phases (one per layer), first has verification
        assert_eq!(wf.phases.len(), 2);
        assert!(wf.phases[0].verification.is_some());
        assert_eq!(wf.phases[0].phase_type, PhaseType::Iterative { max_iterations: 3 });
    }

    #[test]
    fn test_verification_point_insertion() {
        let tasks = vec![
            make_task("Task A", vec![]),
            make_task("Task B", vec!["Task A"]),
        ];
        let vps = vec![VerificationPoint {
            after_tasks: vec!["Task A".to_string()],
            verify: "Check A is correct".to_string(),
            is_blocking: true,
        }];
        let decision = make_decision(DecompositionStrategy::Sequential, tasks, vps);
        let wf = build_workflow_from_decomposition(Uuid::new_v4(), "Test", &decision, WorkflowConfig::default());

        assert!(wf.validate_dag().is_ok());
        // 3 phases: Execute(A) -> Verify -> Execute(B)
        assert_eq!(wf.phases.len(), 3);
        assert_eq!(wf.phases[1].phase_type, PhaseType::Verify);
    }

    #[test]
    fn test_topological_layers() {
        let tasks = vec![
            make_task("A", vec![]),
            make_task("B", vec![]),
            make_task("C", vec!["A", "B"]),
            make_task("D", vec!["C"]),
        ];
        let layers = topological_layers(&tasks);

        assert_eq!(layers.len(), 3);
        // Layer 0: A, B
        assert_eq!(layers[0].len(), 2);
        // Layer 1: C
        assert_eq!(layers[1].len(), 1);
        assert_eq!(layers[1][0].title, "C");
        // Layer 2: D
        assert_eq!(layers[2].len(), 1);
        assert_eq!(layers[2][0].title, "D");
    }
}
