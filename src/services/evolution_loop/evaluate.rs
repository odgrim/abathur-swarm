//! Template evaluation: success-rate stats, refinement triggers, and
//! regression detection.

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::domain::models::AgentStatus;

use super::{
    EvolutionAction, EvolutionEvent, EvolutionLoop, EvolutionTrigger, RefinementRequest,
    RefinementSeverity, RefinementStatus, TaskOutcome, TemplateStats,
};

impl EvolutionLoop {
    /// Evaluate templates and trigger evolution if needed.
    pub async fn evaluate(&self) -> Vec<EvolutionEvent> {
        let stale_expired = self.expire_stale_refinements().await;
        let mut stale_events: Vec<EvolutionEvent> = stale_expired
            .into_iter()
            .map(
                |(template_name, template_version, request_id)| EvolutionEvent {
                    id: Uuid::new_v4(),
                    template_name: template_name.clone(),
                    template_version,
                    trigger: EvolutionTrigger::StaleTimeout,
                    stats_at_trigger: TemplateStats::new(template_name, template_version),
                    action_taken: EvolutionAction::StaleExpired { request_id },
                    occurred_at: Utc::now(),
                },
            )
            .collect();
        let mut new_requests: Vec<RefinementRequest> = Vec::new();
        // Collect revert instructions: (template_name, to_version) to execute
        // outside the write lock since agent_repo calls are async.
        let mut revert_instructions: Vec<(String, u32)> = Vec::new();

        let events = {
            let mut state = self.state.write().await;
            let mut events = Vec::new();

            let template_names: Vec<String> = state.stats.keys().cloned().collect();

            for template_name in template_names {
                let stats = match state.stats.get(&template_name) {
                    Some(s) => s.clone(),
                    None => continue,
                };

                // Skip if not enough tasks
                if stats.total_tasks < self.config.min_tasks_for_evaluation {
                    continue;
                }

                let mut trigger = None;
                let mut severity = RefinementSeverity::Minor;

                // Check for goal violations (immediate review)
                if stats.goal_violations > 0 {
                    trigger = Some(EvolutionTrigger::GoalViolations);
                    severity = RefinementSeverity::Immediate;
                }
                // Check for very low success rate
                else if stats.total_tasks >= self.config.major_refinement_min_tasks
                    && stats.success_rate < self.config.major_refinement_threshold
                {
                    trigger = Some(EvolutionTrigger::VeryLowSuccessRate);
                    severity = RefinementSeverity::Major;
                }
                // Check for low success rate
                else if stats.success_rate < self.config.refinement_threshold {
                    trigger = Some(EvolutionTrigger::LowSuccessRate);
                    severity = RefinementSeverity::Minor;
                }

                // Check for regression
                if trigger.is_none()
                    && let Some((new_version, change_time)) =
                        state.version_change_times.get(&template_name)
                    && stats.template_version == *new_version
                {
                    let window = Duration::hours(self.config.regression_detection_window_hours);
                    let in_window = Utc::now() - *change_time < window;

                    if in_window
                        && stats.total_tasks >= self.config.regression_min_tasks
                        && let Some(prev_stats) = state.previous_version_stats.get(&template_name)
                    {
                        let rate_drop = prev_stats.success_rate - stats.success_rate;
                        if rate_drop >= self.config.regression_threshold {
                            trigger = Some(EvolutionTrigger::Regression);
                            severity = RefinementSeverity::Immediate;
                        }
                    }
                }

                if let Some(trig) = trigger {
                    let action = if trig == EvolutionTrigger::Regression
                        && self.config.auto_revert_enabled
                    {
                        // Auto-revert
                        if let Some(prev_stats) = state.previous_version_stats.get(&template_name) {
                            let to_version = prev_stats.template_version;
                            revert_instructions.push((template_name.clone(), to_version));
                            EvolutionAction::Reverted {
                                from_version: stats.template_version,
                                to_version,
                            }
                        } else {
                            EvolutionAction::FlaggedForRefinement { severity }
                        }
                    } else {
                        // Deduplication: skip if a Pending or InProgress refinement
                        // already exists for this template
                        let has_active = state.refinement_queue.iter().any(|r| {
                            r.template_name == template_name
                                && matches!(
                                    r.status,
                                    RefinementStatus::Pending | RefinementStatus::InProgress
                                )
                        });

                        if has_active {
                            EvolutionAction::NoAction {
                                reason: format!(
                                    "Refinement already pending/in-progress for '{}'",
                                    template_name,
                                ),
                            }
                        } else {
                            // Create refinement request
                            let failed_task_ids = state
                                .executions
                                .get(&template_name)
                                .map(|execs| {
                                    execs
                                        .iter()
                                        .filter(|e| e.outcome != TaskOutcome::Success)
                                        .map(|e| e.task_id)
                                        .collect()
                                })
                                .unwrap_or_default();

                            let request = RefinementRequest::new(
                                template_name.clone(),
                                stats.template_version,
                                severity,
                                trig,
                                stats.clone(),
                                failed_task_ids,
                            );
                            // Collect for persistence outside the write lock
                            new_requests.push(request.clone());
                            state.refinement_queue.push(request);

                            EvolutionAction::FlaggedForRefinement { severity }
                        }
                    };

                    let event = EvolutionEvent {
                        id: Uuid::new_v4(),
                        template_name: template_name.clone(),
                        template_version: stats.template_version,
                        trigger: trig,
                        stats_at_trigger: stats.clone(),
                        action_taken: action,
                        occurred_at: Utc::now(),
                    };

                    state.events.push(event.clone());
                    events.push(event);
                }
            }

            events
        }; // write lock dropped here

        // Persist new requests outside the write lock (non-fatal on failure)
        if let Some(ref repo) = self.refinement_repo {
            for request in &new_requests {
                if let Err(e) = repo.create(request).await {
                    tracing::warn!(
                        "Failed to persist refinement request {} to DB: {}",
                        request.id,
                        e
                    );
                }
            }
        }

        // Actually restore previous template versions for auto-reverts.
        // This must happen outside the write lock because agent_repo calls are async.
        if !revert_instructions.is_empty() {
            if let Some(ref agent_repo) = self.agent_repo {
                for (template_name, to_version) in &revert_instructions {
                    match agent_repo
                        .get_template_version(template_name, *to_version)
                        .await
                    {
                        Ok(Some(mut prev_template)) => {
                            prev_template.status = AgentStatus::Active;
                            prev_template.updated_at = Utc::now();
                            if let Err(e) = agent_repo.update_template(&prev_template).await {
                                tracing::error!(
                                    "Auto-revert failed: could not update template '{}' v{} to active: {}",
                                    template_name,
                                    to_version,
                                    e,
                                );
                            } else {
                                tracing::info!(
                                    "Auto-revert: restored template '{}' v{} as active",
                                    template_name,
                                    to_version,
                                );
                            }
                        }
                        Ok(None) => {
                            tracing::error!(
                                "Auto-revert failed: template '{}' v{} not found in repository",
                                template_name,
                                to_version,
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "Auto-revert failed: could not fetch template '{}' v{}: {}",
                                template_name,
                                to_version,
                                e,
                            );
                        }
                    }
                }
            } else {
                tracing::warn!(
                    "Auto-revert: {} template(s) flagged for revert but no agent repository configured — \
                     revert event emitted but template not actually restored",
                    revert_instructions.len(),
                );
            }
        }

        stale_events.extend(events);
        stale_events
    }

    /// Get stats for a template.
    pub async fn get_stats(&self, template_name: &str) -> Option<TemplateStats> {
        let state = self.state.read().await;
        state.stats.get(template_name).cloned()
    }

    /// Get all template stats.
    pub async fn get_all_stats(&self) -> Vec<TemplateStats> {
        let state = self.state.read().await;
        state.stats.values().cloned().collect()
    }

    /// Get evolution events for audit.
    pub async fn get_events(&self, limit: Option<usize>) -> Vec<EvolutionEvent> {
        let state = self.state.read().await;
        let events: Vec<_> = state.events.iter().rev().cloned().collect();
        match limit {
            Some(n) => events.into_iter().take(n).collect(),
            None => events,
        }
    }

    /// Get templates needing attention (sorted by urgency).
    pub async fn get_templates_needing_attention(&self) -> Vec<(String, RefinementSeverity)> {
        let state = self.state.read().await;
        let mut result: Vec<_> = state
            .refinement_queue
            .iter()
            .filter(|r| r.status == RefinementStatus::Pending)
            .map(|r| (r.template_name.clone(), r.severity))
            .collect();

        // Sort by severity (Immediate > Major > Minor)
        result.sort_by_key(|(_, s)| match s {
            RefinementSeverity::Immediate => 0,
            RefinementSeverity::Major => 1,
            RefinementSeverity::Minor => 2,
        });

        result
    }
}
