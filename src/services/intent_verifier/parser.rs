//! Parsing of verifier-agent responses into domain verification results.
//!
//! The verifier agent returns a structured text block (see
//! `INTENT_VERIFIER_SYSTEM_PROMPT` in `prompt.rs`). This module converts that
//! text back into an [`IntentVerificationResult`], including gaps, constraint
//! evaluations, reprompt guidance, and auto-escalation decisions.

use crate::domain::errors::DomainResult;
use crate::domain::models::{
    ConstraintConformance, ConstraintEvaluation, GapCategory, GapSeverity, HumanEscalation,
    IntentGap, IntentSatisfaction, IntentVerificationResult, NewTaskGuidance, OriginalIntent,
    RepromptApproach, RepromptGuidance, RepromptStrategySelector, SubstrateSession, Task,
};

/// Parse the verifier agent's structured text response into a domain result.
///
/// This is stateless: it depends only on the response text and the
/// verification context (intent, tasks, iteration). It does not read from
/// config or any service state.
pub(super) fn parse_verification_response(
    session: &SubstrateSession,
    intent: &OriginalIntent,
    completed_tasks: &[Task],
    iteration: u32,
) -> DomainResult<IntentVerificationResult> {
    // Get the final response text from the session result
    let response_text = session.result.clone().unwrap_or_default();

    // Parse the structured response
    let mut result = IntentVerificationResult::new(intent.id, IntentSatisfaction::Indeterminate)
        .with_iteration(iteration);

    // Add evaluated tasks
    for task in completed_tasks {
        result = result.with_task(task.id);
    }

    // Parse SATISFACTION
    if let Some(sat_line) = response_text
        .lines()
        .find(|l| l.starts_with("SATISFACTION:"))
    {
        let sat_value = sat_line
            .trim_start_matches("SATISFACTION:")
            .trim()
            .to_lowercase();
        result.satisfaction = match sat_value.as_str() {
            "satisfied" => IntentSatisfaction::Satisfied,
            "partial" => IntentSatisfaction::Partial,
            "unsatisfied" => IntentSatisfaction::Unsatisfied,
            _ => IntentSatisfaction::Indeterminate,
        };
    }

    // Parse CONFIDENCE
    if let Some(conf_line) = response_text.lines().find(|l| l.starts_with("CONFIDENCE:")) {
        let conf_str = conf_line.trim_start_matches("CONFIDENCE:").trim();
        if let Ok(conf) = conf_str.parse::<f64>() {
            result = result.with_confidence(conf);
        }
    }

    // Parse NEEDS_HUMAN and HUMAN_REASON for escalation
    let needs_human = response_text
        .lines()
        .find(|l| l.starts_with("NEEDS_HUMAN:"))
        .map(|l| l.trim_start_matches("NEEDS_HUMAN:").trim().to_lowercase() == "yes")
        .unwrap_or(false);

    if needs_human {
        let human_reason = response_text
            .lines()
            .find(|l| l.starts_with("HUMAN_REASON:"))
            .map(|l| l.trim_start_matches("HUMAN_REASON:").trim().to_string())
            .unwrap_or_else(|| "Human judgment required".to_string());

        result = result.with_escalation(HumanEscalation::new(human_reason));
    }

    // Parse SUMMARY
    if let Some(sum_line) = response_text.lines().find(|l| l.starts_with("SUMMARY:")) {
        let summary = sum_line.trim_start_matches("SUMMARY:").trim();
        result = result.with_summary(summary);
    }

    // Parse GAPS (format: description | severity | action | category)
    let mut in_gaps = false;
    for line in response_text.lines() {
        if line.starts_with("GAPS:") {
            in_gaps = true;
            continue;
        }
        if in_gaps {
            if line.starts_with("IMPLICIT_GAPS:")
                || line.starts_with("CONSTRAINT_CONFORMANCE:")
                || line.starts_with("FOCUS_AREAS:")
                || line.starts_with("NEW_TASKS:")
                || line.is_empty()
            {
                in_gaps = false;
                continue;
            }
            if line.starts_with("- ")
                && let Some(gap) = parse_gap_line(line, false)
            {
                result = result.with_gap(gap);
            }
        }
    }

    // Parse IMPLICIT_GAPS (format: description | severity | rationale)
    let mut in_implicit = false;
    for line in response_text.lines() {
        if line.starts_with("IMPLICIT_GAPS:") {
            in_implicit = true;
            continue;
        }
        if in_implicit {
            if line.starts_with("CONSTRAINT_CONFORMANCE:")
                || line.starts_with("FOCUS_AREAS:")
                || line.starts_with("NEW_TASKS:")
                || line.is_empty()
            {
                in_implicit = false;
                continue;
            }
            if line.starts_with("- ")
                && let Some(gap) = parse_gap_line(line, true)
            {
                result = result.with_implicit_gap(gap);
            }
        }
    }

    // Parse CONSTRAINT_CONFORMANCE (format: constraint text | status | explanation)
    let mut in_constraints = false;
    for line in response_text.lines() {
        if line.starts_with("CONSTRAINT_CONFORMANCE:") {
            in_constraints = true;
            continue;
        }
        if in_constraints {
            if line.starts_with("FOCUS_AREAS:")
                || line.starts_with("NEW_TASKS:")
                || line.starts_with("REPROMPT_STRATEGY:")
                || line.is_empty()
            {
                in_constraints = false;
                continue;
            }
            if line.starts_with("- ") {
                let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
                if parts.len() >= 2 {
                    let constraint = parts[0].trim().to_string();
                    let status = parts[1]
                        .trim()
                        .parse::<ConstraintConformance>()
                        .unwrap_or(ConstraintConformance::Deviating);
                    let explanation = if parts.len() > 2 {
                        parts[2].trim().to_string()
                    } else {
                        String::new()
                    };

                    result = result.with_constraint_evaluation(ConstraintEvaluation {
                        constraint: constraint.clone(),
                        status,
                        explanation,
                    });

                    // For violating constraints, also create a corresponding IntentGap
                    if status == ConstraintConformance::Violating {
                        let severity = if constraint.starts_with("[MUST]") {
                            GapSeverity::Critical
                        } else if constraint.starts_with("[WITHIN]")
                            || constraint.starts_with("[CONSTRAINT]")
                        {
                            GapSeverity::Major
                        } else {
                            GapSeverity::Moderate
                        };
                        result = result.with_gap(
                            IntentGap::new(
                                format!("Constraint violation: {}", constraint),
                                severity,
                            )
                            .with_category(GapCategory::ConstraintViolation),
                        );
                    }
                }
            }
        }
    }

    // Parse REPROMPT_STRATEGY and STRATEGY_RATIONALE
    let strategy = response_text
        .lines()
        .find(|l| l.starts_with("REPROMPT_STRATEGY:"))
        .and_then(|l| {
            let s = l.trim_start_matches("REPROMPT_STRATEGY:").trim();
            s.parse::<RepromptApproach>().ok()
        });

    let _strategy_rationale = response_text
        .lines()
        .find(|l| l.starts_with("STRATEGY_RATIONALE:"))
        .map(|l| {
            l.trim_start_matches("STRATEGY_RATIONALE:")
                .trim()
                .to_string()
        });

    // Build reprompt guidance if not satisfied
    if result.satisfaction != IntentSatisfaction::Satisfied {
        // Use the strategy from the agent if provided, otherwise compute based on gaps
        let approach =
            strategy.unwrap_or_else(|| RepromptStrategySelector::select_strategy(&result));

        let mut guidance = RepromptGuidance::new(approach);

        // Parse FOCUS_AREAS
        let mut in_focus = false;
        for line in response_text.lines() {
            if line.starts_with("FOCUS_AREAS:") {
                in_focus = true;
                continue;
            }
            if in_focus {
                if line.starts_with("NEW_TASKS:")
                    || line.starts_with("REPROMPT_STRATEGY:")
                    || line.is_empty()
                {
                    in_focus = false;
                    continue;
                }
                if line.starts_with("- ") {
                    guidance = guidance.with_focus(line.trim_start_matches("- ").trim());
                }
            }
        }

        // Parse NEW_TASKS (format: title | description | priority | execution_mode)
        let mut in_new_tasks = false;
        for line in response_text.lines() {
            if line.starts_with("NEW_TASKS:") {
                in_new_tasks = true;
                continue;
            }
            if in_new_tasks {
                if line.starts_with("REPROMPT_STRATEGY:")
                    || line.is_empty()
                    || (!line.starts_with("- ") && !line.starts_with("  "))
                {
                    in_new_tasks = false;
                    continue;
                }
                if line.starts_with("- ") {
                    let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
                    if parts.len() >= 2 {
                        let title = parts[0].trim();
                        let description = parts[1].trim();
                        let mut task = NewTaskGuidance::new(title, description);

                        if parts.len() > 2 && parts[2].trim().to_lowercase().as_str() == "high" {
                            task = task.high_priority()
                        }

                        if parts.len() > 3 && parts[3].trim().to_lowercase().as_str() == "blocking"
                        {
                            task = task.blocking()
                        }

                        guidance = guidance.with_new_task(task);
                    }
                }
            }
        }

        // Add context from gaps (both explicit and implicit)
        let all_gaps: Vec<_> = result.all_gaps().collect();
        if !all_gaps.is_empty() {
            let gap_context = all_gaps
                .iter()
                .map(|g| {
                    let implicit_marker = if g.is_implicit { " [IMPLICIT]" } else { "" };
                    format!(
                        "- [{}]{} {}",
                        g.category.as_str(),
                        implicit_marker,
                        g.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            guidance = guidance.with_context(format!("Previous gaps identified:\n{}", gap_context));
        }

        result = result.with_reprompt_guidance(guidance);
    }

    // Check for auto-escalation based on gap patterns
    if result.escalation.is_none()
        && let Some(auto_escalation) = result.should_escalate()
    {
        result = result.with_escalation(auto_escalation);
    }

    Ok(result)
}

/// Parse a single gap line (`- description | severity | action | category`).
///
/// When `is_implicit` is true, the third pipe-separated field is treated as
/// an implicit-requirement rationale rather than a suggested action, and no
/// category field is consumed.
pub(super) fn parse_gap_line(line: &str, is_implicit: bool) -> Option<IntentGap> {
    let parts: Vec<&str> = line.trim_start_matches("- ").split('|').collect();
    if parts.is_empty() {
        return None;
    }

    let description = parts[0].trim().to_string();
    let severity = if parts.len() > 1 {
        match parts[1].trim().to_lowercase().as_str() {
            "minor" => GapSeverity::Minor,
            "moderate" => GapSeverity::Moderate,
            "major" => GapSeverity::Major,
            "critical" => GapSeverity::Critical,
            _ => GapSeverity::Moderate,
        }
    } else {
        GapSeverity::Moderate
    };

    let mut gap = IntentGap::new(description, severity);

    if is_implicit {
        // For implicit gaps: description | severity | rationale
        if parts.len() > 2 {
            gap = gap.as_implicit(parts[2].trim());
        } else {
            gap = gap.as_implicit("Implicit requirement not met");
        }
    } else {
        // For explicit gaps: description | severity | action | category
        if parts.len() > 2 {
            gap = gap.with_action(parts[2].trim());
        }
        if parts.len() > 3
            && let Ok(cat) = parts[3].trim().parse::<GapCategory>()
        {
            gap = gap.with_category(cat);
        }
    }

    Some(gap)
}
