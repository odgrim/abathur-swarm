//! Refinement request lifecycle: queue inspection and status transitions.

use uuid::Uuid;

use super::{EvolutionLoop, RefinementRequest, RefinementStatus};

impl EvolutionLoop {
    /// Get pending refinement requests.
    pub async fn get_pending_refinements(&self) -> Vec<RefinementRequest> {
        let state = self.state.read().await;
        state
            .refinement_queue
            .iter()
            .filter(|r| r.status == RefinementStatus::Pending)
            .cloned()
            .collect()
    }

    /// Check if a template has an active (Pending or InProgress) refinement request.
    pub async fn has_active_refinement(&self, template_name: &str) -> bool {
        let state = self.state.read().await;
        state.refinement_queue.iter().any(|r| {
            r.template_name == template_name
                && matches!(
                    r.status,
                    RefinementStatus::Pending | RefinementStatus::InProgress
                )
        })
    }

    /// Mark a refinement request as in progress.
    pub async fn start_refinement(&self, request_id: Uuid) -> bool {
        let found = {
            let mut state = self.state.write().await;
            let mut found = false;
            for request in &mut state.refinement_queue {
                if request.id == request_id && request.status == RefinementStatus::Pending {
                    request.status = RefinementStatus::InProgress;
                    found = true;
                    break;
                }
            }
            found
        }; // write lock dropped here

        if found
            && let Some(ref repo) = self.refinement_repo
            && let Err(e) = repo
                .update_status(request_id, RefinementStatus::InProgress)
                .await
        {
            tracing::warn!(
                "Failed to persist InProgress status for refinement {}: {}",
                request_id,
                e
            );
        }

        found
    }

    /// Mark a refinement request as completed.
    pub async fn complete_refinement(&self, request_id: Uuid, success: bool) {
        let new_status = if success {
            RefinementStatus::Completed
        } else {
            RefinementStatus::Failed
        };

        {
            let mut state = self.state.write().await;
            for request in &mut state.refinement_queue {
                if request.id == request_id {
                    request.status = new_status;
                    break;
                }
            }
        } // write lock dropped here

        if let Some(ref repo) = self.refinement_repo
            && let Err(e) = repo.update_status(request_id, new_status).await
        {
            tracing::warn!(
                "Failed to persist {} status for refinement {}: {}",
                if success { "Completed" } else { "Failed" },
                request_id,
                e
            );
        }
    }
}
