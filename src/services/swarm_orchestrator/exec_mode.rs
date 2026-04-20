//! Execution-mode resolver.
//!
//! Decides whether a task runs in Direct or Convergent mode based on the
//! stored mode, agent capability, and the swarm-wide convergence toggle.
//!
//! Extracted from `goal_processing::spawn_task_agent` per spec T10
//! (`specs/T10-spawn-task-agent-extraction.md`).
#![allow(dead_code)]

use crate::domain::models::ExecutionMode;

use super::agent_prep::AgentMetadata;

/// Resolves the effective execution mode for a task spawn.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionModeResolverService {
    convergence_enabled: bool,
}

impl ExecutionModeResolverService {
    pub fn new(convergence_enabled: bool) -> Self {
        Self {
            convergence_enabled,
        }
    }

    /// Returns `(effective_mode, is_convergent_final)`.
    ///
    /// A stored Direct mode is upgraded to Convergent when convergence is
    /// enabled globally AND the agent is write-capable AND the agent is not a
    /// read-only role. `is_convergent_final` is `true` only when the effective
    /// mode is Convergent, convergence is enabled, and the role is not
    /// read-only — matching the original gating in `spawn_task_agent`.
    pub fn resolve_mode(
        &self,
        stored_mode: ExecutionMode,
        agent: &AgentMetadata,
    ) -> (ExecutionMode, bool) {
        let effective = if stored_mode.is_direct()
            && self.convergence_enabled
            && !agent.is_read_only_role
            && agent.can_write
        {
            ExecutionMode::Convergent {
                parallel_samples: None,
            }
        } else {
            stored_mode
        };

        let is_convergent =
            effective.is_convergent() && self.convergence_enabled && !agent.is_read_only_role;

        (effective, is_convergent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::AgentTier;

    fn meta(can_write: bool, is_read_only_role: bool) -> AgentMetadata {
        AgentMetadata {
            version: 1,
            capabilities: vec![],
            cli_tools: vec![],
            can_write,
            is_read_only: false,
            max_turns: 0,
            preferred_model: None,
            tier: AgentTier::Worker,
            is_read_only_role,
        }
    }

    #[test]
    fn test_mode_direct_unchanged_for_read_only_agent() {
        let svc = ExecutionModeResolverService::new(true);
        let (effective, is_convergent) = svc.resolve_mode(ExecutionMode::Direct, &meta(false, true));
        assert!(matches!(effective, ExecutionMode::Direct));
        assert!(!is_convergent);
    }

    #[test]
    fn test_mode_upgrades_direct_to_convergent_for_write_capable() {
        let svc = ExecutionModeResolverService::new(true);
        let (effective, is_convergent) =
            svc.resolve_mode(ExecutionMode::Direct, &meta(true, false));
        assert!(matches!(effective, ExecutionMode::Convergent { .. }));
        assert!(is_convergent);
    }

    #[test]
    fn test_mode_respects_convergence_disabled_flag() {
        let svc = ExecutionModeResolverService::new(false);
        let (effective, is_convergent) =
            svc.resolve_mode(ExecutionMode::Direct, &meta(true, false));
        assert!(matches!(effective, ExecutionMode::Direct));
        assert!(!is_convergent);

        // Even an explicit Convergent stored mode is NOT considered "is_convergent"
        // when the global flag is off — it would fall back to direct in spawn.
        let (effective, is_convergent) = svc.resolve_mode(
            ExecutionMode::Convergent {
                parallel_samples: None,
            },
            &meta(true, false),
        );
        assert!(matches!(effective, ExecutionMode::Convergent { .. }));
        assert!(!is_convergent);
    }
}
