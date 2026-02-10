# Attractor-Driven Task Convergence

## Problem Statement

LLMs cannot reliably convert a specification into a complete implementation in one pass. Research demonstrates that implementation is fundamentally an iterative convergence process, not a single-shot transformation. Effectiveness follows exponential decay — capability is typically exhausted within 3-5 attempts. Strategic fresh starts and external feedback dramatically improve outcomes.

Current systems treat iteration as a failure mode — retries, max attempts, timeouts. This spec treats convergence as the *primary mechanism* of task completion, with attractors as the framework for understanding, measuring, and steering it.

Every task execution is a trajectory through solution space. The goal is not "did it pass?" but "where is this trajectory heading?" Attractors are the destinations — some are correct implementations (fixed points we want), some are oscillating failure modes (limit cycles we must escape). The system's job is to detect which attractor a trajectory is approaching and intervene accordingly.

## Convergence Flow

The ConvergenceEngine owns the full lifecycle of a trajectory through five phases:

```
1. SETUP     → Estimate basin width, allocate budget, assemble policy
2. PREPARE   → Generate acceptance tests, detect ambiguity, recall memories
3. DECIDE    → Proactive decomposition check, convergence mode selection
4. ITERATE   → The main convergence loop:
   a. Select strategy (forced or bandit-selected from attractor-eligible set)
   b. Execute strategy → produce artifact
   c. Measure with overseers (external, deterministic — never self-assess)
   d. Compute convergence metrics (delta + level)
   e. Classify attractor (FixedPoint, LimitCycle, Divergent, Plateau, Indeterminate)
   f. Update strategy bandit with outcome
   g. Check loop control:
      → Converged: accept result
      → BudgetExhausted: try extension or partial acceptance, else fail
      → Trapped: all escape strategies exhausted, fail
      → FreshStart: context degraded, reset LLM context with curated carry-forward
      → Continue: loop
5. RESOLVE   → Store convergence memory, persist bandit state, emit terminal event
```

---

## Part 1: Core Model

### 1.1 Trajectory — The Unit of Convergence

A **Trajectory** is the sequence of attempts to satisfy a task. Each attempt produces an **Observation** — a snapshot of where the implementation stands relative to the specification.

```
Trajectory {
    id: TrajectoryId,
    task_id: TaskId,
    goal_id: Option<GoalId>,

    // The specification being converged toward (evolves via amendments)
    specification: SpecificationEvolution,

    // Ordered sequence of observations
    observations: Vec<Observation>,

    // Current attractor classification
    attractor_state: AttractorState,

    // Convergence budget (not iteration count)
    budget: ConvergenceBudget,

    // Active convergence policy
    policy: ConvergencePolicy,

    // Strategy history — what was tried and what happened
    strategy_log: Vec<StrategyEntry>,

    // Phase in the convergence lifecycle
    phase: ConvergencePhase,

    // Health of the LLM's working context (degrades over iterations)
    context_health: ContextHealth,

    // User-provided trajectory hints (always carry forward across fresh starts)
    hints: Vec<String>,

    // When set, this strategy is executed next, bypassing normal selection.
    forced_strategy: Option<StrategyKind>,

    // Total fresh starts in this trajectory (guards against infinite reset loops)
    total_fresh_starts: u32,

    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

StrategyEntry {
    strategy_kind: StrategyKind,
    observation_sequence: u32,
    convergence_delta_achieved: Option<f64>,
    tokens_used: u64,
    was_forced: bool,
    timestamp: DateTime<Utc>,
}

enum ConvergencePhase {
    Preparing,
    Iterating,
    Coordinating { children: Vec<TrajectoryId> },
    // Terminal states
    Converged,
    Exhausted,
    Trapped,
}
```

### 1.2 Observation — A Point in Solution Space

Each iteration produces an Observation. Observations are *measured by overseers* (external verification signals), never self-assessed.

```
Observation {
    id: ObservationId,
    sequence: u32,
    timestamp: DateTime<Utc>,

    // What the agent produced
    artifact: ArtifactReference,

    // Overseer measurements (external, deterministic)
    overseer_signals: OverseerSignals,

    // LLM-based verification (calibrated, periodic)
    verification: Option<VerificationResult>,

    // Convergence metrics computed from this observation
    metrics: Option<ObservationMetrics>,

    // Cost tracking
    tokens_used: u64,
    wall_time_ms: u64,

    // Strategy that produced this observation
    strategy_used: StrategyKind,
}
```

### 1.3 Overseer Signals

Overseers are external verification tools that measure implementation state without self-bias. The OverseerSignals struct aggregates their output. Each field is `Option` because not all overseers apply to every task.

```
OverseerSignals {
    test_results: Option<TestResults>,
    type_check: Option<TypeCheckResult>,
    lint_results: Option<LintResults>,
    build_result: Option<BuildResult>,
    security_scan: Option<SecurityScanResult>,
    custom_checks: Vec<CustomCheckResult>,
}

impl OverseerSignals {
    fn has_any_signal(&self) -> bool {
        self.test_results.is_some()
            || self.type_check.is_some()
            || self.build_result.is_some()
            || self.security_scan.is_some()
            || !self.custom_checks.is_empty()
    }

    fn all_passing(&self) -> bool {
        self.test_results.as_ref().map(|t| t.all_passing()).unwrap_or(true)
            && self.type_check.as_ref().map(|t| t.clean).unwrap_or(true)
            && self.build_result.as_ref().map(|b| b.success).unwrap_or(true)
            && self.security_scan.as_ref().map(|s| s.critical_count == 0).unwrap_or(true)
            && self.custom_checks.iter().all(|c| c.passed)
    }

    fn error_count(&self) -> u32 {
        let build = self.build_result.as_ref().map(|b| b.error_count).unwrap_or(0);
        let type_c = self.type_check.as_ref().map(|t| t.error_count).unwrap_or(0);
        let lint = self.lint_results.as_ref().map(|l| l.error_count).unwrap_or(0);
        build + type_c + lint
    }

    fn vulnerability_count(&self) -> u32 {
        self.security_scan.as_ref().map(|s| s.critical_count + s.high_count).unwrap_or(0)
    }
}
```

### 1.4 Convergence Metrics

Two complementary metrics drive all convergence decisions:

- **`convergence_delta`** — the *derivative* of progress. Measures change between consecutive observations. Positive means getting closer; negative means diverging. Drives attractor classification and strategy selection.
- **`convergence_level`** — the *absolute position*. Measures how close the current state is to "done." Drives termination decisions.

```
ObservationMetrics {
    // --- Per-iteration change (the derivative) ---

    // Structural distance (AST diff between consecutive artifacts)
    ast_diff_nodes: u32,

    // Functional distance
    test_pass_delta: i32,
    test_regression_count: u32,
    error_count_delta: i32,

    // Security distance
    vulnerability_delta: i32,

    // Composite progress score (negative = regressing, 0.0 = stalled, positive = converging)
    convergence_delta: f64,

    // --- Absolute position ---

    // How close to "done" (0.0 = nothing works, 1.0 = fully converged)
    convergence_level: f64,
}
```

**Computing `convergence_delta`:**

Requires the previous observation as a reference point. On the first observation, `metrics` is `None` — there is no delta to compute.

```
fn compute_convergence_delta(
    prev: &Observation,
    current_signals: &OverseerSignals,
    current_ast_diff: u32,
    context_health: &ContextHealth,
) -> f64 {
    let prev_signals = &prev.overseer_signals;

    let prev_pass = prev_signals.test_results.as_ref().map(|t| t.passed).unwrap_or(0);
    let curr_pass = current_signals.test_results.as_ref().map(|t| t.passed).unwrap_or(0);
    let total_tests = current_signals.test_results.as_ref().map(|t| t.total.max(1)).unwrap_or(1);
    let test_delta = (curr_pass as f64 - prev_pass as f64) / total_tests as f64;

    let prev_errors = prev_signals.error_count().max(1);
    let error_delta = (prev_errors as f64 - current_signals.error_count() as f64) / prev_errors as f64;

    let regression_penalty = current_signals.test_results.as_ref()
        .map(|t| t.regression_count as f64 / total_tests as f64)
        .unwrap_or(0.0);

    let structural_churn = 1.0 - (current_ast_diff as f64 / 200.0).min(1.0);

    let mut delta = w_test * test_delta
        + w_error * error_delta
        + w_regression * (1.0 - regression_penalty)
        + w_structural * structural_churn;

    // SECURITY VETO: strategies that introduce vulnerabilities never get credit
    // for "progress" — trains the bandit to avoid vuln-introducing approaches.
    let prev_vulns = prev_signals.vulnerability_count();
    let curr_vulns = current_signals.vulnerability_count();
    if curr_vulns > prev_vulns {
        delta = delta.min(0.0);
    }

    // CONTEXT DEGRADATION PENALTY: when context health degrades, the convergence
    // signal itself becomes unreliable. Scale down proportionally.
    if context_health.signal_to_noise < 0.5 {
        delta *= context_health.signal_to_noise / 0.5;
    }

    delta
}
```

Weights are configurable per task complexity. For well-tested tasks, `w_test` dominates. For exploratory tasks, structural stability matters more.

**Computing `convergence_level`:**

```
fn convergence_level(observation: &Observation) -> f64 {
    let signals = &observation.overseer_signals;

    // No overseers configured means level is 0.0.
    // The system must have at least one signal source to assess convergence.
    if !signals.has_any_signal() {
        return 0.0;
    }

    let test_level = signals.test_results
        .as_ref()
        .map(|t| t.passed as f64 / t.total.max(1) as f64)
        .unwrap_or(1.0);

    let build_level = signals.build_result
        .as_ref()
        .map(|b| if b.success { 1.0 } else { 0.0 })
        .unwrap_or(1.0);

    let type_level = signals.type_check
        .as_ref()
        .map(|t| if t.clean { 1.0 } else { 0.0 })
        .unwrap_or(1.0);

    let custom_level = if signals.custom_checks.is_empty() {
        1.0
    } else {
        signals.custom_checks.iter()
            .filter(|c| c.passed)
            .count() as f64 / signals.custom_checks.len() as f64
    };

    let level = 0.55 * test_level + 0.20 * build_level + 0.10 * type_level + 0.15 * custom_level;

    // Hard gates: build failure caps at 0.3, type failure caps at 0.6
    if build_level < 1.0 { return level.min(0.3); }
    if type_level < 1.0 { return level.min(0.6); }

    level
}
```

### 1.5 Context Health

As iterations progress, the LLM's context window fills with history and the signal-to-noise ratio degrades. ContextHealth tracks three symptoms and provides a degradation check that triggers fresh starts.

```
ContextHealth {
    // Ratio of useful context (spec + current code + latest signals + hints)
    // to noise (old iteration history, previous failed attempts, stale feedback)
    signal_to_noise: f64,

    // Average AST diff nodes per iteration over the recent window.
    // High churn with no functional progress indicates context confusion.
    structural_churn_rate: f64,

    // Are we re-generating similar code?
    artifact_self_similarity: f64,
}

fn estimate_context_health(trajectory: &Trajectory) -> ContextHealth {
    let useful_tokens = estimate_spec_tokens(&trajectory.specification)
        + estimate_code_tokens(&trajectory.latest_artifact())
        + estimate_signal_tokens(&trajectory.latest_overseer_signals())
        + estimate_hint_tokens(&trajectory.hints);

    let total_tokens = estimate_total_context(trajectory);

    let recent = &trajectory.observations[trajectory.observations.len().saturating_sub(3)..];
    let churn_rate = recent.iter()
        .filter_map(|o| o.metrics.as_ref())
        .map(|m| m.ast_diff_nodes as f64)
        .sum::<f64>() / recent.len().max(1) as f64;

    ContextHealth {
        signal_to_noise: useful_tokens as f64 / total_tokens.max(1) as f64,
        structural_churn_rate: churn_rate,
        artifact_self_similarity: compute_artifact_similarity(&trajectory.observations),
    }
}

fn context_is_degraded(trajectory: &Trajectory) -> bool {
    if trajectory.total_fresh_starts >= trajectory.policy.max_fresh_starts {
        return false;  // Guard against infinite reset loops
    }

    let health = &trajectory.context_health;

    // High structural churn with no functional progress
    let high_churn_no_progress = health.structural_churn_rate > 50.0 && {
        let recent = &trajectory.observations[trajectory.observations.len().saturating_sub(3)..];
        recent.iter().all(|o|
            o.metrics.as_ref().map(|m| m.convergence_delta.abs() < 0.03).unwrap_or(true)
        )
    };

    // Context signal-to-noise degraded below threshold
    let context_noisy = health.signal_to_noise < 0.4;

    // Security regression accelerating
    let security_regressing = {
        let recent = &trajectory.observations[trajectory.observations.len().saturating_sub(3)..];
        recent.iter().any(|o|
            o.metrics.as_ref().map(|m| m.vulnerability_delta > 2).unwrap_or(false)
        )
    };

    // Re-generating the same code
    let duplicating = health.artifact_self_similarity > 0.9
        && trajectory.observations.len() >= 2;

    high_churn_no_progress || context_noisy || security_regressing || duplicating
}
```

### 1.6 Specification Evolution

The specification co-evolves with the trajectory. Amending the specification is the *primary mechanism* for escaping limit cycles that stem from specification ambiguity — a fixed prompt produces fixed periodic states.

```
SpecificationEvolution {
    original: SpecificationSnapshot,
    amendments: Vec<SpecificationAmendment>,
    effective: SpecificationSnapshot,    // original + all amendments, recomputed on each addition
}

SpecificationAmendment {
    id: AmendmentId,
    source: AmendmentSource,
    description: String,
    rationale: String,
    observation_trigger: Option<ObservationId>,
    timestamp: DateTime<Utc>,
}

enum AmendmentSource {
    UserHint,                         // user hints or explicit feedback
    ImplicitRequirementDiscovered,    // test failure reveals missing requirement
    OverseerDiscovery,                // overseer signals expose constraint not in spec
    ArchitectAmendment,               // architect review identifies spec gap
    TestDisambiguation,               // generated tests reveal ambiguity
    SubmissionConstraint,             // constraints and anti-patterns from submission
}
```

**When amendments are created:**
- `UserHint`: When the user provides a hint. Hints always carry forward across fresh starts and are prepended to the agent's prompt as high-priority context.
- `ImplicitRequirementDiscovered`: When a test fails in a way that reveals a requirement not stated in the original spec (detected during LLM verification).
- `OverseerDiscovery`: When an overseer measurement exposes a constraint (e.g., security scan finds the spec didn't mention auth requirements).
- `ArchitectAmendment`: When the `ArchitectReview` strategy returns with a modified specification.
- `TestDisambiguation`: During preparation, when contradictory tests reveal ambiguity.
- `SubmissionConstraint`: At trajectory initialization from constraints and anti-patterns.

When an amendment is added, `effective` is recomputed by merging the original specification with all amendments. The effective specification is what gets included in the LLM prompt for subsequent iterations and in CarryForward for fresh starts.

### 1.7 Convergence Policy

The policy governs convergence behavior. Assembled from basin width estimation, priority hints, and complexity during SETUP. Never configured directly by the user.

```
ConvergencePolicy {
    // Exploitation vs exploration balance. 0.0 = pure exploit, 1.0 = pure explore.
    exploration_weight: f64,

    // Minimum convergence level to accept as "done."
    acceptance_threshold: f64,

    // Accept the best result when budget is exhausted, if it meets partial_threshold?
    partial_acceptance: bool,
    partial_threshold: f64,  // default: 0.7

    // Skip expensive overseers (full test suite, integration tests)?
    skip_expensive_overseers: bool,

    // Generate additional acceptance tests during preparation?
    generate_acceptance_tests: bool,

    // How often to run LLM-based intent verification (every Nth iteration).
    intent_verification_frequency: u32,

    // Prefer cheaper strategies in bandit selection.
    prefer_cheap_strategies: bool,

    // Priority hint that affects intervention behavior.
    priority_hint: Option<PriorityHint>,

    // Maximum total fresh starts before escalating.
    max_fresh_starts: u32,  // default: 3
}
```

### 1.8 Convergence Budget

Fixed iteration counts ignore task difficulty and strategy cost. A convergence budget is a multi-dimensional resource envelope:

```
ConvergenceBudget {
    max_tokens: u64,
    max_wall_time: Duration,
    max_iterations: u32,                // safety cap, not primary limit

    tokens_used: u64,
    wall_time_used: Duration,
    iterations_used: u32,

    extensions_requested: u32,
    extensions_granted: u32,
    max_extensions: u32,
}

impl ConvergenceBudget {
    fn remaining_fraction(&self) -> f64 {
        let token_frac = 1.0 - (self.tokens_used as f64 / self.max_tokens as f64);
        let time_frac = 1.0 - (self.wall_time_used.as_secs_f64() / self.max_wall_time.as_secs_f64());
        let iter_frac = 1.0 - (self.iterations_used as f64 / self.max_iterations as f64);
        token_frac.min(time_frac).min(iter_frac)
    }

    fn has_remaining(&self) -> bool {
        self.remaining_fraction() > 0.0
    }

    fn allows_strategy_cost(&self, strategy: &StrategyKind) -> bool {
        self.tokens_used + strategy.estimated_cost() <= self.max_tokens
            && self.iterations_used + 1 <= self.max_iterations
    }

    fn should_request_extension(&self, attractor: &AttractorState) -> bool {
        self.remaining_fraction() < 0.15
            && matches!(attractor.classification, AttractorType::FixedPoint { .. })
            && self.extensions_requested < self.max_extensions
    }

    fn consume(&mut self, tokens: u64, wall_time_ms: u64) {
        self.tokens_used += tokens;
        self.wall_time_used += Duration::from_millis(wall_time_ms);
        self.iterations_used += 1;
    }

    fn extend(&mut self, additional_tokens: u64, additional_iterations: u32) {
        self.extensions_granted += 1;
        self.max_tokens += additional_tokens;
        self.max_iterations += additional_iterations;
    }
}
```

---

## Part 2: Overseers

Overseers are external verification signals that measure implementation state without self-bias. The convergence engine receives overseers as configured inputs — infrastructure discovery is handled upstream.

### 2.1 The Overseer Trait

```
trait Overseer: Send + Sync {
    fn name(&self) -> &str;
    async fn measure(&self, artifact: &ArtifactReference, task: &Task) -> OverseerResult;
    fn cost(&self) -> OverseerCost;
}

enum OverseerCost { Cheap, Moderate, Expensive }
```

Built-in implementations: `CompilationOverseer`, `TypeCheckOverseer`, `LintOverseer`, `BuildOverseer`, `TestSuiteOverseer`, `SecurityScanOverseer`, `AcceptanceTestOverseer`.

User-extensible overseers can be defined via custom scripts with success/failure pattern matching.

### 2.2 Security Scan Overseer

Vulnerabilities accumulate non-linearly with iteration count. The security overseer feeds vulnerability counts into overseer signals, which drives the security veto in `convergence_delta` computation (1.4) and blocks false convergence at termination (6.4).

```
struct SecurityScanOverseer {
    scanner: Box<dyn SecurityScanner>,
}

impl Overseer for SecurityScanOverseer {
    fn name(&self) -> &str { "security-scan" }

    async fn measure(&self, artifact: &ArtifactReference, task: &Task) -> OverseerResult {
        let findings = self.scanner.scan(artifact).await;
        OverseerResult {
            pass: findings.critical_count == 0,
            details: SecurityScanResult {
                critical_count: findings.critical_count,
                high_count: findings.high_count,
                medium_count: findings.medium_count,
                findings: findings.details,
            },
        }
    }

    fn cost(&self) -> OverseerCost { OverseerCost::Moderate }
}
```

### 2.3 Overseer Prioritization

Cheap overseers run first. If cheap overseers show blocking failures, expensive ones are skipped.

```
impl OverseerCluster {
    async fn measure(
        &self,
        artifact: &ArtifactReference,
        task: &Task,
        policy: &ConvergencePolicy,
    ) -> OverseerSignals {
        // Phase 1: Cheap (compilation, type check) — always run
        let cheap_results = self.run_overseers_by_cost(Cheap, artifact, task).await;
        if cheap_results.has_blocking_failures() {
            return OverseerSignals::from_partial(cheap_results);
        }

        // Phase 2: Moderate (lint, fast tests, security)
        let moderate_results = self.run_overseers_by_cost(Moderate, artifact, task).await;

        // Phase 3: Expensive (full test suite, integration tests) — skippable
        if policy.skip_expensive_overseers {
            return OverseerSignals::merge(cheap_results, moderate_results, OverseerSignals::empty());
        }
        let expensive_results = self.run_overseers_by_cost(Expensive, artifact, task).await;

        OverseerSignals::merge(cheap_results, moderate_results, expensive_results)
    }
}
```

---

## Part 3: Attractor Classification

### 3.1 Attractor Types

The system classifies trajectories into attractor types in real-time:

```
AttractorState {
    classification: AttractorType,
    confidence: f64,
    detected_at: ObservationId,
    evidence: AttractorEvidence,
}

AttractorEvidence {
    recent_deltas: Vec<f64>,
    recent_signatures: Vec<String>,
    rationale: String,
}

enum AttractorType {
    // Approaching a stable correct solution.
    // convergence_delta is positive and increasing or stable.
    FixedPoint {
        estimated_remaining_iterations: u32,
        estimated_remaining_tokens: u64,
    },

    // Oscillating between N states.
    // AST fingerprints or test result signatures cycle.
    LimitCycle {
        period: u32,
        cycle_signatures: Vec<String>,
    },

    // Moving away from the target.
    // convergence_delta is negative. Test pass rate is declining.
    Divergent {
        divergence_rate: f64,
        probable_cause: DivergenceCause,
    },

    // Not enough observations to classify (< 3 iterations).
    Indeterminate {
        tendency: ConvergenceTendency,
    },

    // Stalled. convergence_delta near 0 for N iterations.
    Plateau {
        stall_duration: u32,
        plateau_level: f64,
    },
}

enum DivergenceCause {
    SpecificationAmbiguity,
    WrongApproach,
    AccumulatedRegression,
    Unknown,
}

enum ConvergenceTendency {
    Improving,  // last delta > 0
    Declining,  // last delta < 0
    Flat,       // last delta ~ 0
}
```

### 3.2 Detection Algorithm

Classification runs after each observation using a sliding window. Observations without metrics (the first observation) are skipped when computing deltas.

```
fn classify_attractor(observations: &[Observation], window: usize) -> AttractorState {
    let recent = &observations[observations.len().saturating_sub(window)..];

    if recent.len() < 3 {
        let tendency = recent.last()
            .and_then(|o| o.metrics.as_ref())
            .map(|m| match m.convergence_delta {
                d if d > 0.0 => ConvergenceTendency::Improving,
                d if d < 0.0 => ConvergenceTendency::Declining,
                _ => ConvergenceTendency::Flat,
            })
            .unwrap_or(ConvergenceTendency::Flat);
        return Indeterminate { tendency }
    }

    // Only use observations that have computed metrics
    let deltas: Vec<f64> = recent.iter()
        .filter_map(|o| o.metrics.as_ref())
        .map(|m| m.convergence_delta)
        .collect();

    if deltas.len() < 2 {
        return Indeterminate { tendency: ConvergenceTendency::Flat }
    }

    let test_signatures: Vec<String> = recent.iter()
        .map(|o| fingerprint_overseer_results(&o.overseer_signals))
        .collect();

    // Check for limit cycle: repeating signatures
    if let Some(period) = detect_cycle(&test_signatures) {
        return LimitCycle { period, cycle_signatures: test_signatures }
    }

    // Check for plateau: deltas near zero
    let avg_abs_delta = deltas.iter().map(|d| d.abs()).sum::<f64>() / deltas.len() as f64;
    if avg_abs_delta < PLATEAU_EPSILON {
        let level = convergence_level(recent.last().unwrap());
        return Plateau { stall_duration: deltas.len() as u32, plateau_level: level }
    }

    // Check for divergence: deltas consistently negative
    let negative_ratio = deltas.iter().filter(|d| **d < 0.0).count() as f64 / deltas.len() as f64;
    if negative_ratio > 0.7 {
        let rate = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let cause = infer_divergence_cause(recent);
        return Divergent { divergence_rate: rate, probable_cause: cause }
    }

    // Check for fixed point: deltas consistently positive
    let positive_ratio = deltas.iter().filter(|d| **d > 0.0).count() as f64 / deltas.len() as f64;
    if positive_ratio > 0.6 {
        let rate = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let level = convergence_level(recent.last().unwrap());
        let remaining = estimate_remaining_iterations(rate, level);
        return FixedPoint { estimated_remaining_iterations: remaining, .. }
    }

    Indeterminate { tendency: compute_tendency(&deltas) }
}

fn infer_divergence_cause(recent: &[Observation]) -> DivergenceCause {
    let has_regressions = recent.iter()
        .filter_map(|o| o.metrics.as_ref())
        .any(|m| m.test_regression_count > 0);

    // Check if recent amendments suggest ambiguity was discovered
    let has_ambiguity_amendments = recent.iter()
        .any(|o| o.verification.as_ref()
            .map(|v| v.has_ambiguity_gaps())
            .unwrap_or(false));

    let signatures_vary = {
        let sigs: Vec<_> = recent.iter()
            .map(|o| fingerprint_overseer_results(&o.overseer_signals))
            .collect();
        sigs.windows(2).all(|w| w[0] != w[1])
    };

    if has_regressions {
        DivergenceCause::AccumulatedRegression
    } else if has_ambiguity_amendments {
        DivergenceCause::SpecificationAmbiguity
    } else if signatures_vary {
        DivergenceCause::WrongApproach
    } else {
        DivergenceCause::Unknown
    }
}
```

### 3.3 Cycle Detection

Limit cycle detection uses fingerprinting. Each observation produces a signature from its overseer signals (which tests pass, which fail, error messages). The system detects repeating patterns:

```
fn detect_cycle(signatures: &[String]) -> Option<u32> {
    // Try periods 2, 3, 4 (most common per research)
    for period in 2..=4 {
        if signatures.len() < period * 2 { continue }

        let recent = &signatures[signatures.len() - period * 2..];
        let first_half = &recent[..period];
        let second_half = &recent[period..];

        if fuzzy_sequence_match(first_half, second_half, CYCLE_SIMILARITY_THRESHOLD) {
            return Some(period as u32)
        }
    }
    None
}
```

---

## Part 4: Convergence Strategies

### 4.1 Strategy Types

A Strategy is not just "retry" — it's a specific approach to moving toward the attractor. Strategies are selected based on attractor state, budget remaining, and historical effectiveness.

```
enum StrategyKind {
    // --- Exploitation strategies (refine current approach) ---

    // Re-run with overseer feedback from the last iteration appended.
    RetryWithFeedback,

    // Re-run with additional context: related code, examples, documentation.
    RetryAugmented,

    // Target specific failing tests with minimal context, maximum focus.
    FocusedRepair,

    // Address one gap at a time rather than all at once.
    IncrementalRefinement,

    // --- Exploration strategies (try different approaches) ---

    // Restructure the prompt. Instead of "fix the failing tests,"
    // reframe as "implement X from scratch given these constraints."
    Reframe,

    // Break the task into smaller sub-tasks. Transitions the trajectory
    // to Coordinating phase (Part 9).
    Decompose,

    // Explicitly instruct a different implementation approach.
    AlternativeApproach,

    // Escalate to the architect agent for re-planning. Returns a
    // SpecificationAmendment and optionally restructures the DAG.
    // Only eligible when an architect service is available.
    ArchitectReview,

    // --- Rollback strategy ---

    // Roll back to the best observation so far and branch from there.
    RevertAndBranch { target: ObservationId },

    // --- Fresh Start ---
    // Start a new LLM execution carrying forward KNOWLEDGE (not context).
    // The filesystem persists code; the trajectory persists metadata;
    // the LLM gets a clean context with only curated CarryForward signals.
    FreshStart { carry_forward: CarryForward },
}

impl StrategyKind {
    fn estimated_cost(&self) -> u64 {
        match self {
            FocusedRepair => 15_000,
            RetryWithFeedback => 20_000,
            RevertAndBranch { .. } => 20_000,
            IncrementalRefinement => 25_000,
            RetryAugmented | ArchitectReview | FreshStart { .. } => 30_000,
            AlternativeApproach => 35_000,
            Reframe => 40_000,
            Decompose => 50_000,
        }
    }
}

enum StrategyOutcome {
    Success,   // convergence_delta > threshold
    Marginal,  // convergence_delta > 0 but below threshold
    Neutral,   // convergence_delta near 0
    Failure,   // convergence_delta < -threshold
}
```

**Strategy execution** translates a `StrategyKind` into an LLM prompt and context assembly, producing an artifact:

- **RetryWithFeedback**: Appends the latest overseer signals (test failures, error messages) as feedback to the original task prompt.
- **FocusedRepair**: Narrows context to only the failing test(s) and relevant source code.
- **Reframe**: Rewrites the task as a fresh implementation problem with constraints derived from what has been learned.
- **FreshStart**: Discards the LLM context entirely, injects only CarryForward signals into a fresh session.
- **Decompose**: Proposes subtasks via LLM, then delegates to `decompose_and_coordinate` (Part 9).
- **RevertAndBranch**: Resets the working filesystem to the target observation's artifact, then continues from there.
- **ArchitectReview**: Sends the current trajectory state to the architect agent, which returns a `SpecificationAmendment` (applied to `SpecificationEvolution`) and optionally a revised approach. The next iteration uses the amended specification.

### 4.2 CarryForward — Knowledge Across Context Boundaries

What gets carried across a fresh start boundary. The filesystem persists code; neural context is discarded; curated signals are injected into the new context.

```
CarryForward {
    // The effective specification (original + all amendments)
    specification: SpecificationSnapshot,

    // Best overseer signals achieved — not the code, the SIGNALS.
    // "You previously got 10/12 tests passing. Tests 5 and 7 remain."
    best_signals: OverseerSignals,
    best_artifact: ArtifactReference,

    // What was tried and failed — compressed to anti-patterns, not full history.
    failure_summary: String,

    // Specific remaining gaps
    remaining_gaps: Vec<IntentGap>,

    // User hints (always carry forward — high-signal)
    hints: Vec<String>,
}
```

### 4.3 Strategy Eligibility — The Attractor-Based Filter

The eligibility filter determines which strategies are *candidates* based on the current attractor state. This is deterministic — it narrows the field. Actual selection among eligible strategies is done by the bandit (4.4).

```
fn eligible_strategies(
    trajectory: &Trajectory,
    attractor: &AttractorState,
    budget: &ConvergenceBudget,
) -> Vec<StrategyKind> {
    let history = &trajectory.strategy_log;
    let fresh_starts_remaining = trajectory.policy.max_fresh_starts
        .saturating_sub(trajectory.total_fresh_starts);

    match attractor.classification {
        FixedPoint { remaining, .. } => {
            // Converging. Only exploitation strategies.
            if remaining <= 2 {
                vec![RetryWithFeedback, IncrementalRefinement]
            } else {
                vec![RetryWithFeedback, FocusedRepair, IncrementalRefinement, RetryAugmented]
            }
        }

        LimitCycle { period, .. } => {
            // Trapped in a cycle. Only exploration strategies.
            // Never re-use the strategy that got us here.
            let used_recently = history.last_n(period * 2)
                .map(|e| e.strategy_kind)
                .collect::<HashSet<_>>();

            let mut candidates: Vec<StrategyKind> = vec![
                Reframe, AlternativeApproach, Decompose,
            ].into_iter()
                .filter(|s| !used_recently.contains(s))
                .collect();

            if candidates.is_empty() {
                if budget.allows_strategy_cost(&Decompose) {
                    candidates.push(Decompose);
                }
                // No candidates and can't decompose → Trapped (handled by loop control)
            }
            candidates
        }

        Divergent { cause, .. } => {
            match cause {
                SpecificationAmbiguity => vec![ArchitectReview, Reframe],
                WrongApproach => vec![AlternativeApproach, Reframe],
                AccumulatedRegression => vec![RevertAndBranch {
                    target: best_observation_id(history),
                }],
                Unknown => vec![Reframe, AlternativeApproach],
            }
        }

        Plateau { stall_duration, plateau_level, .. } => {
            if stall_duration >= 3 && fresh_starts_remaining > 0 {
                vec![FreshStart {
                    carry_forward: extract_carry_forward(trajectory),
                }]
            } else if stall_duration >= 3 {
                // Fresh start limit reached — escalate.
                vec![Decompose, AlternativeApproach, ArchitectReview]
            } else if plateau_level > 0.8 {
                vec![FocusedRepair, IncrementalRefinement]
            } else if plateau_level > 0.5 {
                vec![AlternativeApproach, Reframe, Decompose]
            } else {
                vec![Decompose, ArchitectReview]
            }
        }

        Indeterminate { .. } => {
            vec![RetryAugmented, RetryWithFeedback, FocusedRepair]
        }
    }
}
```

### 4.4 Strategy Selection — Thompson Sampling

The eligibility filter (4.3) narrows the field. Thompson Sampling selects *which* eligible strategy to use, learning from historical outcomes.

```
StrategyBandit {
    // Different learned distributions for different attractor states
    context_arms: HashMap<AttractorType, HashMap<StrategyKind, BetaDistribution>>,
}

impl StrategyBandit {
    fn select(
        &self,
        attractor: &AttractorType,
        eligible: &[StrategyKind],
        policy: &ConvergencePolicy,
    ) -> StrategyKind {
        let priors = self.context_arms.get(attractor);

        eligible.iter()
            .map(|s| {
                let dist = priors
                    .and_then(|p| p.get(s))
                    .unwrap_or(&BetaDistribution::uniform());
                let mut score = dist.sample();

                if policy.prefer_cheap_strategies {
                    let cost_factor = 1.0 / (1.0 + s.estimated_cost() as f64 / 100_000.0);
                    score *= 1.0 + cost_factor;
                }

                (s, score)
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(s, _)| *s)
            .unwrap()
    }

    fn update(&mut self, strategy: StrategyKind, attractor: &AttractorType, observation: &Observation) {
        let outcome = evaluate_strategy_outcome(observation);
        let dist = self.context_arms
            .entry(attractor.clone())
            .or_default()
            .entry(strategy)
            .or_insert(BetaDistribution::uniform());

        match outcome {
            StrategyOutcome::Success  => dist.alpha += 1.0,
            StrategyOutcome::Marginal => dist.alpha += 0.5,
            StrategyOutcome::Neutral  => {},
            StrategyOutcome::Failure  => dist.beta += 1.0,
        }
    }
}

fn evaluate_strategy_outcome(observation: &Observation) -> StrategyOutcome {
    match observation.metrics.as_ref() {
        Some(m) if m.convergence_delta > STRATEGY_SUCCESS_THRESHOLD => StrategyOutcome::Success,
        Some(m) if m.convergence_delta > 0.0 => StrategyOutcome::Marginal,
        Some(m) if m.convergence_delta > -STRATEGY_SUCCESS_THRESHOLD => StrategyOutcome::Neutral,
        _ => StrategyOutcome::Failure,
    }
}
```

The bandit's learned distributions persist across tasks via the memory system (Part 8).

### 4.5 Strategy Effectiveness Tracking

Exploitation strategies have a shelf life. The system fits an exponential decay curve to recent deltas and rotates when projected progress drops below threshold:

```
fn should_rotate_strategy(
    current_strategy: &StrategyKind,
    consecutive_uses: u32,
    recent_deltas: &[f64],
) -> bool {
    if let Some((e0, lambda)) = fit_decay_curve(recent_deltas) {
        let t_theta = -ln(MINIMUM_USEFUL_PROGRESS / e0) / lambda;
        consecutive_uses as f64 >= t_theta
    } else {
        // Can't fit curve. Simple heuristic: rotate after 3 consecutive
        // uses with diminishing returns.
        consecutive_uses >= 3 && is_diminishing(recent_deltas)
    }
}
```

When decay-aware rotation triggers, the main loop proceeds to normal strategy re-selection. When context degradation is detected (1.5), the engine forces a FreshStart via `trajectory.forced_strategy`.

---

## Part 5: Budget & Basin Estimation

### 5.1 Budget Allocation by Complexity

```
fn allocate_budget(task: &Task) -> ConvergenceBudget {
    let (tokens, iters, time_mins) = match task.inferred_complexity {
        Trivial  => (50_000,   3,  15),
        Simple   => (150_000,  5,  30),
        Moderate => (400_000,  8,  60),
        Complex  => (1_000_000, 12, 120),
    };

    ConvergenceBudget {
        max_tokens: tokens,
        max_iterations: iters,
        max_wall_time: Duration::from_secs(time_mins * 60),
        max_extensions: if task.inferred_complexity >= Complex { 3 } else { 1 },
        ..Default::default()
    }
}
```

### 5.2 Basin Width — Predicting Convergence Before Starting

A well-specified task with comprehensive tests has a *wide attractor basin* — many starting points lead to the correct solution. A vague task with no tests has a *narrow basin* — the system must get lucky. Basin width drives budget scaling, strategy mix, and whether to generate convergence infrastructure proactively.

```
BasinWidth {
    score: f64,             // 0.0 to 1.0
    classification: BasinClassification,
}

enum BasinClassification { Wide, Moderate, Narrow }

fn estimate_basin_width(submission: &TaskSubmission) -> BasinWidth {
    let mut score = 0.5;

    // Specification quality signals (widen the basin)
    if submission.infrastructure.acceptance_tests.len() > 0 { score += 0.15; }
    if submission.infrastructure.examples.len() > 0 { score += 0.10; }
    if submission.infrastructure.invariants.len() > 0 { score += 0.10; }
    if submission.infrastructure.anti_examples.len() > 0 { score += 0.05; }
    if submission.infrastructure.context_files.len() > 0 { score += 0.05; }

    // Specification complexity signals (narrow the basin)
    let word_count = submission.description.split_whitespace().count();
    if word_count < 20 { score -= 0.15; }
    if word_count > 500 { score -= 0.10; }

    // Historical signal
    if let Some(historical_rate) = convergence_rate_for_similar_tasks(&submission) {
        score = 0.6 * historical_rate + 0.4 * score;
    }

    BasinWidth {
        score: score.clamp(0.0, 1.0),
        classification: match score {
            s if s > 0.7 => BasinClassification::Wide,
            s if s > 0.4 => BasinClassification::Moderate,
            _ => BasinClassification::Narrow,
        },
    }
}
```

### 5.3 Basin Width Effects on Budget and Policy

Basin width adjusts the budget and policy assembled in SETUP. These adjustments compose with the base allocation from complexity (5.1) and priority hints (7.2).

```
fn apply_basin_width(
    basin: &BasinWidth,
    budget: &mut ConvergenceBudget,
    policy: &mut ConvergencePolicy,
) {
    match basin.classification {
        Wide => {
            budget.max_iterations = (budget.max_iterations as f64 * 0.75) as u32;
            policy.exploration_weight = 0.2;
        }
        Moderate => {
            policy.exploration_weight = 0.4;
        }
        Narrow => {
            budget.max_iterations = (budget.max_iterations as f64 * 1.5) as u32;
            budget.max_tokens = (budget.max_tokens as f64 * 1.3) as u64;
            policy.exploration_weight = 0.6;
            policy.generate_acceptance_tests = true;
        }
    }
}
```

### 5.4 Convergence Cost Estimation

Estimates expected convergence cost from historical data or heuristics. Used by the proactive decomposition decision (Part 9) to compare monolithic vs. decomposed approaches.

```
ConvergenceEstimate {
    expected_iterations: f64,
    p95_iterations: u32,
    convergence_probability: f64,
    expected_tokens: u64,
}

fn estimate_convergence(
    submission: &TaskSubmission,
    basin: &BasinWidth,
    historical: &TrajectoryRepository,
) -> ConvergenceEstimate {
    let similar = historical.get_similar_trajectories(
        &submission.description, submission.infrastructure.tags(), 50,
    );

    if similar.len() >= 10 {
        // Enough history — use empirical distribution
        let iterations: Vec<f64> = similar.iter()
            .map(|t| t.observations.len() as f64).collect();

        ConvergenceEstimate {
            expected_iterations: mean(&iterations),
            p95_iterations: percentile(&iterations, 0.95) as u32,
            convergence_probability: similar.iter()
                .filter(|t| t.phase == ConvergencePhase::Converged)
                .count() as f64 / similar.len() as f64,
            expected_tokens: mean_tokens(&similar),
        }
    } else {
        // Insufficient history — heuristic
        let base = match submission.complexity() {
            Trivial => 2.0, Simple => 4.0, Moderate => 6.0, Complex => 9.0,
        };
        let adjusted = base / basin.score;

        ConvergenceEstimate {
            expected_iterations: adjusted,
            p95_iterations: (adjusted * 1.8).ceil() as u32,
            convergence_probability: basin.score,
            expected_tokens: (adjusted * 30_000.0) as u64,
        }
    }
}
```

---

## Part 6: The Convergence Engine

### 6.1 Engine Structure

The ConvergenceEngine owns the full lifecycle of a trajectory.

```
ConvergenceEngine {
    overseer_cluster: OverseerCluster,
    strategy_bandit: StrategyBandit,
    trajectory_store: TrajectoryRepository,
    intent_verifier: IntentVerifierService,
    memory_service: MemoryService,
    event_bus: EventBus,

    config: ConvergenceEngineConfig {
        classification_window: usize,              // default: 5
        plateau_epsilon: f64,                      // default: 0.02
        cycle_similarity_threshold: f64,           // default: 0.85
        min_observations_for_classification: usize, // default: 3
        decay_rotation_threshold: f64,             // default: 0.05
    },
}
```

### 6.2 Preparation

Before entering the iteration loop, the engine prepares convergence infrastructure. Infrastructure discovery (test frameworks, build tools, type checkers) is performed upstream and provided as input. Preparation focuses on generating convergence-specific assets.

```
impl ConvergenceEngine {
    async fn prepare(
        &self,
        submission: &TaskSubmission,
        policy: &ConvergencePolicy,
    ) -> ConvergenceInfrastructure {
        // 1. Start from discovered project infrastructure (provided as input)
        let mut infra = ConvergenceInfrastructure::from(
            &submission.discovered_infrastructure,
        );

        // 2. Merge with user-provided references
        infra.merge_user_references(&submission.references);
        infra.add_invariants(&submission.constraints);
        infra.add_anti_patterns(&submission.anti_patterns);

        // 3. Generate acceptance tests when needed
        if infra.acceptance_tests.is_empty() || policy.generate_acceptance_tests {
            let generated = self.generate_acceptance_tests(
                &submission.description, &infra.examples,
            ).await;
            infra.acceptance_tests.extend(generated);
        }

        // 4. Infer invariants if none provided
        if infra.invariants.is_empty() && submission.constraints.is_empty() {
            infra.invariants = self.infer_invariants(
                &submission.description, &infra.acceptance_tests,
            ).await;
        }

        // 5. Check for spec ambiguity via test contradiction analysis
        let contradictions = self.detect_test_contradictions(&infra.acceptance_tests);
        if !contradictions.is_empty() {
            self.event_bus.publish(SpecificationAmbiguityDetected {
                task_id: submission.task_id,
                contradictions,
                suggested_clarifications: self.suggest_clarifications(&contradictions),
            });
        }

        infra
    }
}
```

### 6.3 Main Loop

```
impl ConvergenceEngine {
    async fn converge(&self, task: &Task, goal: Option<&Goal>) -> ConvergenceOutcome {
        // --- SETUP ---
        let spec = SpecificationEvolution::new(self.snapshot_specification(task, goal));
        let basin = estimate_basin_width(&task.submission);
        let mut budget = self.allocate_budget(task);
        let mut policy = ConvergencePolicy::default();
        apply_basin_width(&basin, &mut budget, &mut policy);
        if let Some(hint) = &task.submission.priority_hint {
            hint.apply(&mut policy, &mut budget);
        }

        // --- PREPARE ---
        let mut trajectory = Trajectory::new(task.id, goal.map(|g| g.id), spec, budget, policy);
        trajectory.phase = ConvergencePhase::Preparing;

        let infra = self.prepare(&task.submission, &trajectory.policy).await;
        self.overseer_cluster.configure(infra.to_overseers());
        if !infra.acceptance_tests.is_empty() {
            self.overseer_cluster.add(AcceptanceTestOverseer::new(&infra.acceptance_tests));
        }

        // Recall memories from similar past trajectories
        let memories = self.memory_service.recall_by_tags(
            &["convergence"], Some(&task.category), 10,
        ).await;

        // --- DECIDE ---

        // Proactive decomposition check
        if let DecompositionDecision::Recommend { decomposition, savings_estimate } =
            self.maybe_decompose_proactively(&task.submission, &basin).await
        {
            self.event_bus.publish(DecompositionRecommended {
                task_id: task.id,
                subtask_count: decomposition.subtasks.len(),
                savings_estimate,
            });
            if matches!(basin.classification, BasinClassification::Narrow)
                && trajectory.policy.priority_hint.is_some()
            {
                return self.decompose_and_coordinate(task, goal, &mut trajectory).await;
            }
        }

        // Convergence mode selection
        let mode = select_convergence_mode(&basin, &trajectory.policy, &task.submission);
        if let ConvergenceMode::Parallel { initial_samples } = mode {
            return self.converge_parallel(
                task, goal, initial_samples, trajectory.budget, trajectory.policy,
            ).await;
        }

        // --- ITERATE ---
        trajectory.phase = ConvergencePhase::Iterating;
        let bandit = self.initialize_bandit(task, &memories).await;

        self.event_bus.publish(TrajectoryStarted {
            trajectory_id: trajectory.id,
            task_id: task.id,
            budget: trajectory.budget.clone(),
        });

        loop {
            // 1. Select strategy
            let (strategy, was_forced) = if let Some(forced) = trajectory.forced_strategy.take() {
                (forced, true)
            } else {
                let eligible = eligible_strategies(
                    &trajectory, &trajectory.attractor_state, &trajectory.budget,
                );
                let selected = bandit.select(
                    &trajectory.attractor_state.classification,
                    &eligible,
                    &trajectory.policy,
                );
                (selected, false)
            };

            // Track fresh starts
            if matches!(&strategy, StrategyKind::FreshStart { .. }) {
                trajectory.total_fresh_starts += 1;
            }

            self.event_bus.publish(StrategySelected {
                trajectory_id: trajectory.id,
                strategy: strategy.clone(),
                attractor: trajectory.attractor_state.classification.clone(),
                budget_remaining: trajectory.budget.remaining_fraction(),
            });

            // 2. Execute (Decompose transitions to Coordinating phase)
            if let StrategyKind::Decompose = &strategy {
                return self.decompose_and_coordinate(task, goal, &mut trajectory).await;
            }
            let artifact = self.execute_strategy(&strategy, task, &trajectory).await?;

            // 3. Measure with overseers
            let overseer_signals = self.overseer_cluster.measure(
                &artifact, task, &trajectory.policy,
            ).await;

            // 4. Optional LLM verification
            let verification = if self.should_verify(&trajectory) {
                Some(self.intent_verifier.verify_intent(
                    task, goal, &artifact, &overseer_signals,
                ).await)
            } else {
                None
            };

            // 5. Compute convergence metrics
            let metrics = if let Some(prev) = trajectory.observations.last() {
                let ast_diff = compute_ast_diff(&prev.artifact, &artifact);
                let delta = compute_convergence_delta(
                    prev, &overseer_signals, ast_diff, &trajectory.context_health,
                );
                let level = convergence_level_from_signals(&overseer_signals);
                let vuln_delta = overseer_signals.vulnerability_count() as i32
                    - prev.overseer_signals.vulnerability_count() as i32;
                Some(ObservationMetrics {
                    ast_diff_nodes: ast_diff,
                    test_pass_delta: compute_test_pass_delta(prev, &overseer_signals),
                    test_regression_count: overseer_signals.regression_count(),
                    error_count_delta: compute_error_delta(prev, &overseer_signals),
                    vulnerability_delta: vuln_delta,
                    convergence_delta: delta,
                    convergence_level: level,
                })
            } else {
                None
            };

            // 6. Record observation
            let observation = Observation {
                sequence: trajectory.observations.len() as u32,
                artifact, overseer_signals, verification, metrics,
                tokens_used: /* from execution */,
                wall_time_ms: /* from execution */,
                strategy_used: strategy.clone(),
                ..
            };
            trajectory.observations.push(observation);
            trajectory.budget.consume(observation.tokens_used, observation.wall_time_ms);

            trajectory.strategy_log.push(StrategyEntry {
                strategy_kind: strategy.clone(),
                observation_sequence: observation.sequence,
                convergence_delta_achieved: metrics.map(|m| m.convergence_delta),
                tokens_used: observation.tokens_used,
                was_forced,
                timestamp: Utc::now(),
            });

            // 7. Update context health
            trajectory.context_health = estimate_context_health(&trajectory);

            // 8. Classify attractor
            trajectory.attractor_state = self.classify_attractor(
                &trajectory.observations, self.config.classification_window,
            );

            self.event_bus.publish(AttractorClassified {
                trajectory_id: trajectory.id,
                attractor_type: trajectory.attractor_state.classification.clone(),
                confidence: trajectory.attractor_state.confidence,
            });

            // 9. Update bandit (skip forced strategies to avoid contamination)
            if !was_forced {
                bandit.update(
                    strategy.clone(),
                    &trajectory.attractor_state.classification,
                    &observation,
                );
            }

            self.event_bus.publish(ObservationRecorded {
                trajectory_id: trajectory.id,
                observation_sequence: observation.sequence,
                convergence_delta: metrics.map(|m| m.convergence_delta),
                convergence_level: metrics.map(|m| m.convergence_level),
                budget_remaining: trajectory.budget.remaining_fraction(),
            });

            // 10. Check loop control
            match self.check_loop_control(&trajectory) {
                LoopControl::Converged => {
                    trajectory.phase = ConvergencePhase::Converged;
                    self.finalize(&trajectory, &bandit, task, true).await;
                    return ConvergenceOutcome::Converged {
                        artifact: observation.artifact,
                        iterations: trajectory.observations.len(),
                        total_tokens: trajectory.budget.tokens_used,
                    };
                }
                LoopControl::BudgetExhausted => {
                    if trajectory.budget.should_request_extension(&trajectory.attractor_state) {
                        if self.request_extension(&mut trajectory).await {
                            continue;
                        }
                    }

                    let best = self.best_observation(&trajectory);
                    let best_level = convergence_level(&best);
                    if trajectory.policy.partial_acceptance
                        && best_level >= trajectory.policy.partial_threshold
                    {
                        trajectory.phase = ConvergencePhase::Converged;
                        self.finalize(&trajectory, &bandit, task, true).await;
                        return ConvergenceOutcome::Converged {
                            artifact: best.artifact,
                            iterations: trajectory.observations.len(),
                            total_tokens: trajectory.budget.tokens_used,
                        };
                    }

                    trajectory.phase = ConvergencePhase::Exhausted;
                    self.finalize(&trajectory, &bandit, task, false).await;
                    return ConvergenceOutcome::Exhausted {
                        best_artifact: best.artifact,
                        attractor: trajectory.attractor_state,
                    };
                }
                LoopControl::ForceFreshStart => {
                    self.event_bus.publish(ContextDegradationDetected {
                        trajectory_id: trajectory.id,
                        signal_to_noise: trajectory.context_health.signal_to_noise,
                    });
                    trajectory.forced_strategy = Some(FreshStart {
                        carry_forward: extract_carry_forward(&trajectory),
                    });
                    continue;
                }
                LoopControl::Trapped => {
                    trajectory.phase = ConvergencePhase::Trapped;
                    self.finalize(&trajectory, &bandit, task, false).await;
                    return ConvergenceOutcome::Trapped {
                        cycle: trajectory.attractor_state,
                        best_artifact: self.best_observation(&trajectory).artifact,
                    };
                }
                LoopControl::Continue => continue,
            }
        }
    }

    fn should_verify(&self, trajectory: &Trajectory) -> bool {
        let iteration = trajectory.observations.len() as u32;
        if iteration == 0 { return true; }
        iteration % trajectory.policy.intent_verification_frequency == 0
    }
}
```

### 6.4 Loop Control

```
enum LoopControl {
    Converged,
    BudgetExhausted,
    ForceFreshStart,  // NOT terminal — forces a clean context on next iteration
    Trapped,
    Continue,
}

enum ConvergenceOutcome {
    Converged {
        artifact: ArtifactReference,
        iterations: usize,
        total_tokens: u64,
    },
    Exhausted {
        best_artifact: ArtifactReference,
        attractor: AttractorState,
    },
    Trapped {
        cycle: AttractorState,
        best_artifact: ArtifactReference,
    },
}

impl ConvergenceEngine {
    fn check_loop_control(&self, trajectory: &Trajectory) -> LoopControl {
        let latest = trajectory.observations.last().unwrap();
        let level = convergence_level(latest);

        // --- Converged ---
        if level >= trajectory.policy.acceptance_threshold {
            // Security veto: block convergence if latest observation introduced vulnerabilities.
            let has_security_regression = latest.metrics
                .as_ref()
                .map(|m| m.vulnerability_delta > 0)
                .unwrap_or(false);

            if !has_security_regression {
                if let Some(ref v) = latest.verification {
                    if v.satisfied() {
                        return LoopControl::Converged;
                    }
                    // Overseers above threshold but verification sees gaps.
                    // Trust overseers (external > self-assessment) unless level < 1.0.
                    if level >= 1.0 || latest.overseer_signals.all_passing() {
                        return LoopControl::Converged;
                    }
                } else {
                    return LoopControl::Converged;
                }
            }
        }

        // --- Budget exhausted ---
        if !trajectory.budget.has_remaining() {
            return LoopControl::BudgetExhausted;
        }

        // --- Context degradation (non-terminal: forces FreshStart) ---
        if context_is_degraded(trajectory) {
            return LoopControl::ForceFreshStart;
        }

        // --- Trapped in limit cycle with no unexplored escape strategies ---
        if let LimitCycle { .. } = &trajectory.attractor_state.classification {
            let eligible = eligible_strategies(
                trajectory, &trajectory.attractor_state, &trajectory.budget,
            );
            if eligible.is_empty() {
                return LoopControl::Trapped;
            }
        }

        LoopControl::Continue
    }
}
```

### 6.5 Budget Extension

```
impl ConvergenceEngine {
    async fn request_extension(&self, trajectory: &mut Trajectory) -> bool {
        trajectory.budget.extensions_requested += 1;

        let extension = BudgetExtension {
            additional_tokens: trajectory.budget.max_tokens / 4,
            additional_iterations: 3,
        };

        self.event_bus.publish(BudgetExtensionRequested {
            trajectory_id: trajectory.id,
            current_budget: trajectory.budget.clone(),
            requested_extension: extension.clone(),
            convergence_evidence: format!(
                "Attractor: {:?}, convergence level: {:.0}%",
                trajectory.attractor_state.classification,
                convergence_level(trajectory.observations.last().unwrap()) * 100.0,
            ),
        });

        // Auto-approve unless Thorough priority requires explicit user approval.
        let approved = match trajectory.policy.priority_hint {
            Some(PriorityHint::Thorough) => {
                self.wait_for_user_approval(InterventionPoint::BudgetExtension).await
            }
            _ => true,
        };

        if approved {
            trajectory.budget.extend(extension.additional_tokens, extension.additional_iterations);
        }
        approved
    }
}
```

### 6.6 Convergence Modes

The engine supports two convergence modes: sequential iteration (default) and parallel trajectory sampling.

```
enum ConvergenceMode {
    Sequential,
    Parallel { initial_samples: u32 },
}

fn select_convergence_mode(
    basin: &BasinWidth,
    policy: &ConvergencePolicy,
    submission: &TaskSubmission,
) -> ConvergenceMode {
    if let Some(samples) = submission.parallel_samples {
        return ConvergenceMode::Parallel { initial_samples: samples };
    }

    match (basin.classification, policy.priority_hint) {
        (Wide, _) => ConvergenceMode::Sequential,
        (Narrow, Some(Thorough)) => ConvergenceMode::Parallel { initial_samples: 3 },
        (Narrow, Some(Fast)) => ConvergenceMode::Parallel { initial_samples: 2 },
        (Narrow, Some(Cheap)) => ConvergenceMode::Sequential,
        (Narrow, None) => ConvergenceMode::Parallel { initial_samples: 2 },
        (Moderate, _) => ConvergenceMode::Sequential,
    }
}
```

**Parallel trajectory sampling:** When the basin is narrow, running parallel independent trajectories and selecting the best can outperform sequential iteration. A Thompson Sampling bandit selects *which trajectory* to invest the next iteration in.

```
impl ConvergenceEngine {
    async fn converge_parallel(
        &self,
        task: &Task,
        goal: Option<&Goal>,
        initial_samples: u32,
        mut shared_budget: ConvergenceBudget,
        policy: ConvergencePolicy,
    ) -> ConvergenceOutcome {
        let mut trajectories: Vec<Trajectory> = vec![];

        // Phase 1: Generate N independent starts.
        for _ in 0..initial_samples {
            let trajectory = self.start_new_trajectory(task, goal, &policy).await;
            shared_budget.consume(
                trajectory.budget.tokens_used,
                trajectory.budget.wall_time_used.as_millis() as u64,
            );
            trajectories.push(trajectory);
        }

        // Phase 2: Iterative refinement — invest in the most promising trajectory.
        let mut trajectory_scores: Vec<BetaDistribution> =
            vec![BetaDistribution::uniform(); trajectories.len()];

        while shared_budget.has_remaining() {
            // Thompson sample to select which trajectory to iterate
            let selected = trajectory_scores.iter()
                .enumerate()
                .filter(|(i, _)| !matches!(
                    trajectories[*i].attractor_state.classification,
                    Divergent { .. }
                ) || trajectories[*i].observations.len() < 3)
                .map(|(i, dist)| (i, dist.sample()))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(i, _)| i);

            let selected = match selected {
                Some(i) => i,
                None => break, // All trajectories divergent
            };

            let observation = self.iterate(&mut trajectories[selected], task).await;
            shared_budget.consume(observation.tokens_used, observation.wall_time_ms);

            match observation.metrics.as_ref().map(|m| m.convergence_delta) {
                Some(delta) if delta > 0.0 => trajectory_scores[selected].alpha += 1.0,
                Some(delta) if delta < 0.0 => trajectory_scores[selected].beta += 1.0,
                _ => {},
            }

            if let Some(converged) = self.check_any_converged(&trajectories, &policy) {
                return converged;
            }
        }

        self.best_across_all(&trajectories)
    }
}
```

### 6.7 Finalization

Consolidates end-of-trajectory work: emit terminal event, store convergence memory, persist bandit state, save trajectory.

```
impl ConvergenceEngine {
    async fn finalize(
        &self,
        trajectory: &Trajectory,
        bandit: &StrategyBandit,
        task: &Task,
        success: bool,
    ) {
        // Emit terminal event
        match trajectory.phase {
            ConvergencePhase::Converged => {
                self.event_bus.publish(TrajectoryConverged {
                    trajectory_id: trajectory.id,
                    iterations: trajectory.observations.len(),
                    total_tokens: trajectory.budget.tokens_used,
                });
            }
            ConvergencePhase::Exhausted => {
                self.event_bus.publish(TrajectoryExhausted {
                    trajectory_id: trajectory.id,
                    best_observation_sequence: self.best_observation(trajectory).sequence,
                    budget_consumed: trajectory.budget.tokens_used,
                });
            }
            ConvergencePhase::Trapped => {
                self.event_bus.publish(TrajectoryTrapped {
                    trajectory_id: trajectory.id,
                    cycle_period: match &trajectory.attractor_state.classification {
                        AttractorType::LimitCycle { period, .. } => *period,
                        _ => 0,
                    },
                    escape_attempts: trajectory.strategy_log.len(),
                });
            }
            _ => {}
        }

        // Store convergence memory and persist bandit state
        if success {
            self.store_success_memory(trajectory, task).await;
        } else {
            self.store_failure_memory(trajectory, task).await;
        }
        self.persist_bandit_state(bandit, task).await;
        self.trajectory_store.save(trajectory).await;
    }
}
```

---

## Part 7: User Interaction

The convergence engine works *without the user knowing any of it exists.* A task submission is just a sentence. Everything else — basin estimation, budget allocation, strategy selection — is inferred. The surface area below is progressive disclosure: users can enrich their submissions to widen the attractor basin, but nothing is required.

### 7.1 Task Submission

```
TaskSubmission {
    description: String,

    // Inferred
    goal_id: Option<GoalId>,
    inferred_complexity: Complexity,
    discovered_infrastructure: DiscoveredInfrastructure,

    // User-provided enrichment
    priority_hint: Option<PriorityHint>,
    constraints: Vec<String>,          // invariants: "must X"
    references: Vec<Reference>,        // code/test file references
    anti_patterns: Vec<String>,        // "not Y"

    // Advanced
    parallel_samples: Option<u32>,
}
```

### 7.2 Priority Hints

Priority hints adjust both budget and policy. They compose with basin width adjustments — basin width runs first, then priority hints overlay.

```
enum PriorityHint { Fast, Thorough, Cheap }

impl PriorityHint {
    fn apply(&self, policy: &mut ConvergencePolicy, budget: &mut ConvergenceBudget) {
        match self {
            Fast => {
                budget.max_iterations = budget.max_iterations.min(5);
                policy.acceptance_threshold = 0.85;
                policy.skip_expensive_overseers = true;
                policy.partial_acceptance = true;
                policy.exploration_weight = 0.1;
            }
            Thorough => {
                budget.max_extensions += 2;
                policy.acceptance_threshold = 0.98;
                policy.skip_expensive_overseers = false;
                policy.partial_acceptance = false;
                policy.exploration_weight = 0.4;
                policy.generate_acceptance_tests = true;
            }
            Cheap => {
                budget.max_tokens = (budget.max_tokens as f64 * 0.7) as u64;
                policy.skip_expensive_overseers = true;
                policy.intent_verification_frequency = 3;
                policy.prefer_cheap_strategies = true;
            }
        }
    }
}
```

### 7.3 Intervention Points

The system pauses at natural convergence boundaries. The engine emits an intervention event and waits for a response (approval, rejection, or user-provided context).

```
enum InterventionPoint {
    AttractorTransition { from: AttractorType, to: AttractorType },
    StrategyEscalation { proposed_strategy: StrategyKind, reason: String },
    BudgetExtension { current_budget: ConvergenceBudget, proposed_extension: BudgetExtension },
    AmbiguityDetected { contradictions: Vec<String> },
    PartialResult { best_observation: Observation, remaining_gaps: Vec<IntentGap> },
    HumanEscalation { context: CarryForward, reason: String },
}
```

Priority hints control which interventions pause vs. auto-decide:

- **Fast**: Only `PartialResult` triggers. Everything else is auto-decided.
- **Thorough**: All intervention points pause for user input.
- **No hint**: `AttractorTransition` and `BudgetExtension` notify; `StrategyEscalation`, `AmbiguityDetected`, and `HumanEscalation` pause.

`HumanEscalation` is the terminal intervention: when all escape strategies for a limit cycle are exhausted and the budget allows no further exploration, the engine pauses with full convergence context (CarryForward) and waits for human guidance before either continuing or accepting the Trapped outcome.

---

## Part 8: Memory Integration

Every trajectory produces memories that improve future convergence.

### 8.1 Convergence Memory

```
async fn store_success_memory(&self, trajectory: &Trajectory, task: &Task) {
    self.memory_service.store(Memory {
        tier: MemoryTier::Episodic,
        memory_type: MemoryType::Pattern,
        content: format!(
            "Task type '{}' converged in {} iterations. \
             Effective strategy sequence: {:?}. \
             Attractor path: {:?}. \
             Key overseer signals that drove convergence: {:?}",
            task.category,
            trajectory.observations.len(),
            trajectory.effective_strategy_sequence(),
            trajectory.attractor_path(),
            trajectory.decisive_overseer_changes(),
        ),
        relevance_score: 0.8,
        tags: vec!["convergence", "success", &task.category],
        associated_goal_id: trajectory.goal_id,
        ..
    }).await;
}

async fn store_failure_memory(&self, trajectory: &Trajectory, task: &Task) {
    self.memory_service.store(Memory {
        tier: MemoryTier::Episodic,
        memory_type: MemoryType::Error,
        content: format!(
            "Task type '{}' failed to converge after {} iterations. \
             Terminal attractor: {:?}. \
             Strategies attempted: {:?}. \
             Persistent gaps: {:?}",
            task.category,
            trajectory.observations.len(),
            trajectory.attractor_state,
            trajectory.all_strategies_used(),
            trajectory.persistent_gaps(),
        ),
        relevance_score: 0.9,  // failures are more valuable than successes
        tags: vec!["convergence", "failure", &task.category],
        ..
    }).await;
}
```

### 8.2 Bandit State Persistence

The bandit's learned distributions persist across tasks. During initialization, past trajectory memories prime strategy preferences:

```
async fn persist_bandit_state(&self, bandit: &StrategyBandit, task: &Task) {
    self.memory_service.store(Memory {
        tier: MemoryTier::Semantic,
        memory_type: MemoryType::Pattern,
        content: serialize_bandit_state(bandit),
        tags: vec!["strategy-bandit", &task.category],
        ..
    }).await;
}

async fn initialize_bandit(&self, task: &Task, memories: &[Memory]) -> StrategyBandit {
    let mut bandit = if let Some(memory) = self.memory_service.recall_by_tags(
        &["strategy-bandit", &task.category], None, 1
    ).await.first() {
        deserialize_bandit_state(&memory.content)
    } else {
        StrategyBandit::with_default_priors()
    };

    // Boost strategies that succeeded for similar tasks
    for memory in memories.iter().filter(|m| m.tags.contains(&"success")) {
        if let Some(effective_strategies) = extract_strategies_from_memory(memory) {
            for strategy in effective_strategies {
                bandit.nudge(&strategy, 0.3);
            }
        }
    }

    bandit
}
```

---

## Part 9: Decomposition as Convergence

### 9.1 Proactive Decomposition

Convergence probability increases super-linearly with specification simplicity. A task that takes 12 iterations as a monolith might take 3+3+2 = 8 iterations as three subtasks — and each subtask is more likely to converge at all.

Called during `converge()` before entering the iteration loop. If decomposition would be significantly more efficient, it's recommended (and auto-applied for narrow basins with a priority hint).

```
enum DecompositionDecision {
    NoDecomposition,
    Recommend {
        decomposition: TaskDecomposition,
        savings_estimate: f64,
    },
}

impl ConvergenceEngine {
    async fn maybe_decompose_proactively(
        &self,
        submission: &TaskSubmission,
        basin: &BasinWidth,
    ) -> DecompositionDecision {
        if basin.classification == BasinClassification::Wide {
            return DecompositionDecision::NoDecomposition;
        }

        let monolithic_expected_iters = estimate_convergence(
            submission, basin, &self.trajectory_store,
        ).expected_iterations;

        let decomposition = self.propose_decomposition(submission).await;
        let decomposed_expected_iters: f64 = decomposition.subtasks.iter()
            .map(|st| {
                let sub_basin = estimate_basin_width(st);
                estimate_convergence(st, &sub_basin, &self.trajectory_store).expected_iterations
            })
            .sum();

        let overhead = decomposition.subtasks.len() as f64 * 0.5;
        let decomposed_total = decomposed_expected_iters + overhead;

        if decomposed_total < monolithic_expected_iters * 0.8 {
            DecompositionDecision::Recommend {
                decomposition,
                savings_estimate: monolithic_expected_iters - decomposed_total,
            }
        } else {
            DecompositionDecision::NoDecomposition
        }
    }
}
```

### 9.2 Decompose-and-Coordinate Flow

When the Decompose strategy is selected (proactively or reactively), the parent trajectory transitions to `Coordinating` phase:

```
impl ConvergenceEngine {
    async fn decompose_and_coordinate(
        &self,
        task: &Task,
        goal: Option<&Goal>,
        parent_trajectory: &mut Trajectory,
    ) -> ConvergenceOutcome {
        let decomposition = self.propose_decomposition(&task.submission).await;

        // 25% of budget reserved for integration (non-negotiable)
        let (child_budgets, integration_budget) =
            allocate_decomposed_budget(&parent_trajectory.budget, decomposition.subtasks.len());

        let child_ids: Vec<TrajectoryId> = decomposition.subtasks.iter()
            .map(|_| TrajectoryId::new()).collect();
        parent_trajectory.phase = ConvergencePhase::Coordinating {
            children: child_ids.clone(),
        };

        // Each child runs through the full convergence engine
        let mut child_outcomes = vec![];
        for (subtask, child_budget) in decomposition.subtasks.iter().zip(child_budgets) {
            let outcome = self.converge(subtask, goal).await;
            child_outcomes.push(outcome);
        }

        if child_outcomes.iter().any(|o| !o.is_converged()) {
            return ConvergenceOutcome::Exhausted {
                best_artifact: self.best_child_artifact(&child_outcomes),
                attractor: parent_trajectory.attractor_state.clone(),
            };
        }

        // Mandatory integration trajectory
        self.run_integration_trajectory(
            task, goal, &child_outcomes, integration_budget,
        ).await
    }
}
```

### 9.3 The Integration Guard

After all subtask trajectories converge, a **mandatory integration trajectory** runs with overseers that specifically check composition: integration tests, end-to-end scenarios, and interface contract verification. This guards against the documented 92% performance gap between single-function and compositional verification.

```
fn allocate_decomposed_budget(
    total_budget: &ConvergenceBudget,
    subtask_count: usize,
) -> (Vec<ConvergenceBudget>, ConvergenceBudget) {
    let integration_budget = total_budget.scale(0.25);
    let per_subtask_budget = total_budget.scale(0.75 / subtask_count as f64);
    (vec![per_subtask_budget; subtask_count], integration_budget)
}

// Parent convergence = integration convergence, NOT the sum of child deltas.
fn compute_parent_convergence(
    child_trajectories: &[Trajectory],
    integration_trajectory: &Trajectory,
) -> f64 {
    if child_trajectories.iter().all(|t| t.is_converged()) {
        integration_trajectory.latest_convergence_delta()
    } else {
        let worst_child = child_trajectories.iter()
            .map(|t| t.latest_convergence_delta())
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        worst_child * 0.5
    }
}
```

### 9.4 Recursive Decomposition

Decomposition is fractal — each subtask is itself a convergence problem with its own trajectory, attractor classification, and strategy selection:

```
Trajectory (monolith)
  -> Decompose
  -> Trajectory (subtask 1) -> converges in 3 iterations
  -> Trajectory (subtask 2) -> limit cycle -> further decomposition
      -> Trajectory (subtask 2a) -> converges in 2 iterations
      -> Trajectory (subtask 2b) -> converges in 2 iterations
  -> MANDATORY: Trajectory (integration) -> converges in 2 iterations
```

---

## Part 10: Observability & Persistence

### 10.1 Event Catalog

All convergence events flow through the EventBus. Events are emitted inline throughout the engine (see Part 6 for emission points).

**Lifecycle events:**
- `TrajectoryStarted { trajectory_id, task_id, budget }` — emitted at iteration loop entry
- `TrajectoryConverged { trajectory_id, iterations, total_tokens }` — emitted at finalization
- `TrajectoryExhausted { trajectory_id, best_observation_sequence, budget_consumed }` — emitted at finalization
- `TrajectoryTrapped { trajectory_id, cycle_period, escape_attempts }` — emitted at finalization

**Per-iteration events:**
- `StrategySelected { trajectory_id, strategy_kind, attractor_type, budget_remaining }`
- `ObservationRecorded { trajectory_id, sequence, convergence_delta, convergence_level, budget_remaining }`
- `AttractorClassified { trajectory_id, attractor_type, confidence }`

**Intervention events:**
- `ContextDegradationDetected { trajectory_id, signal_to_noise }` — triggers ForceFreshStart
- `BudgetExtensionRequested { trajectory_id, current_budget, requested_extension, convergence_evidence }`
- `SpecificationAmbiguityDetected { task_id, contradictions, suggested_clarifications }` — emitted during preparation
- `DecompositionRecommended { task_id, subtask_count, savings_estimate }` — emitted during DECIDE phase

### 10.2 Trajectory Persistence

```
trait TrajectoryRepository: Send + Sync {
    async fn save(&self, trajectory: &Trajectory) -> Result<()>;
    async fn get(&self, id: TrajectoryId) -> Result<Option<Trajectory>>;
    async fn get_by_task(&self, task_id: TaskId) -> Result<Vec<Trajectory>>;
    async fn get_by_goal(&self, goal_id: GoalId) -> Result<Vec<Trajectory>>;

    async fn avg_iterations_by_complexity(&self, complexity: Complexity) -> Result<f64>;
    async fn strategy_effectiveness(&self, strategy: StrategyKind) -> Result<StrategyStats>;
    async fn attractor_distribution(&self) -> Result<HashMap<AttractorType, u32>>;
    async fn convergence_rate_by_task_type(&self, category: &str) -> Result<f64>;
    async fn get_similar_trajectories(
        &self, description: &str, tags: &[String], limit: usize,
    ) -> Result<Vec<Trajectory>>;
}
```

Storage schema is implementation-defined. The repository trait abstracts storage concerns — SQLite, Postgres, or in-memory implementations are all valid backends.

---

## Implementation Phases

### Phase 1: Foundation — Trajectory Model + Measurement
- `Trajectory`, `Observation`, `ObservationMetrics`, `ContextHealth` types
- `convergence_level()` and `convergence_delta` computation
- `TrajectoryRepository` trait + initial storage adapter
- Wire into existing convergence loop as measurement-only (emit events, don't control flow)

### Phase 2: Classification — Attractor Detection
- `AttractorState`, `AttractorType`, `AttractorEvidence` types
- Classification algorithm with cycle detection
- Event emission from existing convergence loop (observability before control)

### Phase 3: Strategy Engine + Budget
- `StrategyKind`, `CarryForward`, `ConvergencePolicy` types
- `ConvergenceBudget` with `consume()`, `extend()`, `allows_strategy_cost()`
- Attractor-based eligibility filter with fresh start guard
- `StrategyBandit` with Thompson Sampling and cost-weighted selection
- Decay-aware rotation and context degradation detection
- Replace fixed `max_iterations` with budget-driven loop control

### Phase 4: Overseers + Convergence Estimation
- `Overseer` trait + built-in implementations
- `SecurityScanOverseer` with security veto integration
- Phased overseer execution (cheap first, skip expensive per policy)
- `BasinWidth` estimation from spec quality + historical data
- `ConvergenceEstimate` with expected iterations and probability

### Phase 5: The Convergence Engine
- `ConvergenceEngine` assembling all Phase 1-4 components
- Preparation with test generation and ambiguity detection
- Main convergence loop with full loop control (including partial acceptance)
- Strategy execution layer (translating `StrategyKind` into LLM prompts)
- Replace existing convergence loop

### Phase 6: User Interaction + Memory
- Priority hints, specification evolution, user hints
- Intervention points with policy-aware approval
- Budget extension flow
- Convergence memory storage + bandit state persistence
- Bandit initialization with memory-based priming

### Phase 7: Decomposition + Parallel Sampling
- Proactive decomposition for narrow basins
- `decompose_and_coordinate` with mandatory integration trajectory
- Parallel trajectory sampling with shared budget and Thompson Sampling selection
- Convergence mode selection based on basin width

---

## Appendix: Research Foundation

Each design decision traces to empirical research:

| Research | Finding | Design Decision |
|----------|---------|-----------------|
| **DDI** (Nature Scientific Reports, 2025) | LLM debugging effectiveness follows exponential decay. GPT-4 exhausts capability by iteration 3. Strategic fresh starts improved accuracy 72.6% → 82.8%. | Decay-aware strategy rotation (4.5). Fresh start with curated carry-forward (4.2). |
| **Self-Repair Limitations** (ICLR 2024) | Self-assessed convergence is unreliable. External feedback increases repair success 1.58x. Feedback quality is the bottleneck. | Overseer-driven measurement, never self-assessment (Part 2). User hints as high-value external feedback (1.6). |
| **LLM Attractor Cycles** (ACL 2025) | Under iteration, LLMs converge to stable periodic states (typically 2-period cycles). Self-reinforcing nature traps the system. | Limit cycle detection via fingerprinting (3.3). Specification amendment as primary cycle escape (1.6). |
| **REx** (NeurIPS 2024) | Iterative repair as a bandit problem. Thompson Sampling for exploit/explore decisions. 1.5-5x fewer API calls. | Strategy selection via Thompson Sampling (4.4). Context-dependent bandit arms per attractor state. |
| **Reflexion** (NeurIPS 2023) | Verbal reinforcement learning with episodic memory. 88% pass@1 vs 67% base GPT-4. Persistent memory is the key differentiator. | Convergence memory across trajectories (Part 8). Bandit state persistence for cross-task learning. |
| **TICODER** (IEEE TSE 2024) | Tests as specification disambiguators and convergence detectors. 45.97% improvement within 5 interactions. | Acceptance test generation during preparation (6.2). Tests widen the attractor basin. |
| **Chaotic Dynamics** (2025) | 23% smooth convergence, 41% oscillatory, 36% chaotic. "Context window degradation" — losing global awareness as iterations progress. | Context health tracking (1.5). Context degradation forces fresh starts (6.4). |
| **Security Degradation** (IEEE-ISTAS 2025) | 37.6% increase in critical vulnerabilities after 5 iterations. Accumulation is non-linear. | Security scan overseer (2.2). Security veto on convergence_delta and termination (1.4, 6.4). |
| **Local Success Does Not Compose** (2025) | 92% performance gap between single-function and compositional verification. | Mandatory integration trajectory after decomposition (9.3). |
| **Self-Refine** (NeurIPS 2023) | Generate-feedback-refine loop: ~20% improvement but self-bias accumulates monotonically. | External overseers instead of self-assessment. Attractor classification detects when self-bias causes plateaus. |
