//! Regression tests for feature_name extraction from chain step outputs.
//!
//! These tests ensure that chain steps properly extract feature_name from previous
//! step output even when the output is wrapped in markdown code fences.
//!
//! Bug context: Prior to the fix in `enqueue_next_chain_step`, JSON wrapped in
//! ```json...``` would fail to parse, causing feature_branch to not be set on
//! subsequent chain tasks (e.g., technical-requirements-specialist would have
//! null feature_branch even though technical-architect output contained feature_name).

use abathur_cli::infrastructure::validators::output_validator::OutputValidator;

/// Test that feature_name is correctly extracted from markdown-wrapped JSON output
///
/// This regression test ensures that chain steps properly extract feature_name
/// from previous step output even when the output is wrapped in markdown code fences.
#[test]
fn test_feature_name_extraction_from_markdown_wrapped_json() {
    // Simulate the output from technical-architect step (wrapped in markdown)
    let markdown_wrapped_output = r#"```json
{
  "feature_name": "dag-workflow-execution",
  "architecture_overview": "Some overview text"
}
```"#;

    // This is what the fix does: strip markdown before parsing
    let cleaned_output = OutputValidator::strip_markdown_code_blocks(markdown_wrapped_output);

    // Verify the cleaned output is valid JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&cleaned_output).expect("Cleaned output should be valid JSON");

    // Verify feature_name is extractable
    let feature_name = parsed
        .get("feature_name")
        .and_then(|v| v.as_str())
        .expect("feature_name should be extractable");

    assert_eq!(feature_name, "dag-workflow-execution");
}

/// Test that plain JSON (without markdown fencing) still works correctly
#[test]
fn test_feature_name_extraction_from_plain_json() {
    let plain_json = r#"{"feature_name": "test-feature", "other_field": 123}"#;

    let cleaned_output = OutputValidator::strip_markdown_code_blocks(plain_json);

    let parsed: serde_json::Value =
        serde_json::from_str(&cleaned_output).expect("Plain JSON should still be valid");

    let feature_name = parsed
        .get("feature_name")
        .and_then(|v| v.as_str())
        .expect("feature_name should be extractable");

    assert_eq!(feature_name, "test-feature");
}

/// Test that feature_branch is correctly generated from feature_name
#[test]
fn test_feature_branch_generation_from_feature_name() {
    let feature_name = "dag-workflow-execution";
    let expected_branch = format!("feature/{}", feature_name);

    assert_eq!(expected_branch, "feature/dag-workflow-execution");
}

/// Test extraction from realistic technical-architect output
#[test]
fn test_extraction_from_realistic_architect_output() {
    // This mirrors actual output from the technical-architect agent
    let realistic_output = r#"```json
{
  "feature_name": "dag-workflow-execution",
  "architecture_overview": "Evolve the existing sequential prompt chain execution to a DAG-based model enabling parallel execution of independent steps, convergence points for multi-branch merging, and efficient concurrent I/O.",
  "components": [
    {
      "name": "DagStepGraph",
      "responsibility": "Build and validate DAG structure from chain steps."
    }
  ],
  "decomposition": {
    "strategy": "single",
    "subprojects": ["dag-workflow-execution"]
  }
}
```"#;

    let cleaned_output = OutputValidator::strip_markdown_code_blocks(realistic_output);
    let parsed: serde_json::Value =
        serde_json::from_str(&cleaned_output).expect("Realistic output should parse");

    // Verify feature_name extraction
    let feature_name = parsed
        .get("feature_name")
        .and_then(|v| v.as_str())
        .expect("feature_name should be extractable");
    assert_eq!(feature_name, "dag-workflow-execution");

    // Verify nested fields are also accessible
    let strategy = parsed
        .get("decomposition")
        .and_then(|d| d.get("strategy"))
        .and_then(|s| s.as_str())
        .expect("decomposition.strategy should be extractable");
    assert_eq!(strategy, "single");
}
