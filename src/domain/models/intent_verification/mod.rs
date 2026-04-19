//! Intent Verification domain model.
//!
//! Captures the concept of verifying that completed work satisfies the
//! original intent, not just the derived checklist. This enables convergence
//! loops where work can be re-evaluated and refined.
//!
//! ## Key Principles
//!
//! 1. **Goals are convergent attractors** - they are never "completed."
//!    Verification happens at the task/wave level, not goal level.
//!
//! 2. **The re-prompt test**: "If someone submitted the exact same prompt again,
//!    would there be additional work done?" If yes, intent is not satisfied.
//!
//! 3. **Semantic drift detection**: If the same gaps keep appearing across
//!    iterations, we're not making progress and should escalate or restructure.
//!
//! ## Verification Hierarchy
//!
//! - Task verification: Single task against its description
//! - Wave verification: Batch of concurrent tasks
//! - Branch verification: Dependency chain sub-objective
//! - Intent alignment: Tasks against the guiding goal's intent (but never "goal completion")
//!
//! ## Module layout
//!
//! This module is split into four submodules:
//! - [`verification`]: verification protocol — gaps, satisfaction, constraints, results
//! - [`guidance`]: re-prompt guidance, new-task guidance, strategy selection, task augmentations
//! - [`convergence_state`]: iteration state, gap fingerprints, embedding-based similarity
//! - [`escalation`]: human escalation types and events
//!
//! Every symbol is re-exported at this module's root so existing paths
//! like `domain::models::intent_verification::Foo` continue to work.

pub mod convergence_state;
pub mod escalation;
pub mod guidance;
pub mod verification;

pub use convergence_state::{
    cosine_similarity, jaccard_similarity, ConvergenceConfig, ConvergenceState,
    EmbeddedGapFingerprint, EmbeddingSimilarityConfig, EnhancedConvergenceState, GapCluster,
    GapFingerprint, IterationContext,
};
pub use escalation::{
    EscalationDecision, EscalationUrgency, HumanEscalation, HumanEscalationEvent,
    HumanEscalationResponse,
};
pub use guidance::{
    build_task_augmentations, DependentTaskAugmentation, NewTaskGuidance, RepromptApproach,
    RepromptGuidance, RepromptStrategySelector, TaskAugmentation, TaskExecutionMode,
    TaskGuidancePriority,
};
pub use verification::{
    BranchVerificationRequest, BranchVerificationResult, ConstraintConformance,
    ConstraintEvaluation, GapCategory, GapSeverity, IntentGap, IntentSatisfaction, IntentSource,
    IntentVerificationResult, OriginalIntent,
};
