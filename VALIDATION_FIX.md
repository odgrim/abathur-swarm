# Requirements Gatherer Output Validation Fix

## Problem

The requirements-gatherer agent was failing to parse with the error:
```
Validation error for step gather_requirements: Requirements must include problem statement,
functional requirements, and success criteria - Failed to parse output as JSON
```

## Root Cause

The validation system had two validation methods with inconsistent behavior:

1. **`OutputValidator::validate()`** (line 92-112)
   - Strips markdown code blocks before validating JSON ✓
   - Used in `execute_step()` for basic format validation

2. **`OutputValidator::validate_json()`** (line 115-146)
   - Did NOT strip markdown blocks ✗
   - Tried to parse JSON directly
   - Used by `validate_with_rule()` for schema validation

## The Flow

```
execute_step()
  → validator.validate() [strips markdown, validates successfully]
  → Creates StepResult with original output (still has markdown)

execute_single_step()
  → validate_step_output()
  → validator.validate_with_rule()
  → validator.validate_json() [does NOT strip markdown, fails parsing]
```

## The Fix

Modified `OutputValidator::validate_json()` to strip markdown code blocks before parsing,
matching the behavior of `validate()`:

```rust
pub fn validate_json(&self, output: &str, schema: &serde_json::Value) -> Result<bool> {
    // Strip markdown code blocks before validating (LLMs often wrap JSON in code blocks)
    let cleaned = Self::strip_markdown_code_blocks(output);

    // Parse the cleaned output as JSON
    let instance: serde_json::Value = serde_json::from_str(&cleaned)
        .context("Failed to parse output as JSON")?;

    // ... rest of validation
}
```

## Test Added

Added `test_validate_json_strips_markdown_code_blocks()` to verify the fix:
- Tests that `validate_json()` can handle markdown-wrapped JSON
- Mirrors the actual output format from LLM agents

## Files Changed

- `src/infrastructure/validators/output_validator.rs`
  - Modified `validate_json()` method (line 115)
  - Added test case (line 333)

## Impact

This fix ensures that both validation paths (basic format validation and schema validation)
consistently handle LLM outputs that wrap JSON in markdown code blocks, which is a common
behavior even when the prompt instructs otherwise.
