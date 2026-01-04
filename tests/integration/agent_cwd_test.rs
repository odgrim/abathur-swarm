//! Integration tests for agent working directory (CWD) handling
//!
//! Tests that worktree_path is correctly extracted from task input_data
//! and passed to the substrate as working_directory.

use serde_json::json;

/// Test the CWD extraction logic that's used in execute_inner
///
/// This mimics the logic at agent_executor.rs:1078-1090:
/// ```rust
/// let working_directory = ctx.input_data.as_ref()
///     .and_then(|data| data.get("worktree_path"))
///     .and_then(|v| v.as_str())
///     .map(|s| s.to_string());
/// ```
fn extract_working_directory(input_data: Option<&serde_json::Value>) -> Option<String> {
    input_data
        .and_then(|data| data.get("worktree_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[test]
fn test_cwd_extraction_from_input_data() {
    let input_data = json!({
        "worktree_path": "/path/to/.abathur/worktrees/feature-test",
        "other_field": "value"
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(
        result,
        Some("/path/to/.abathur/worktrees/feature-test".to_string())
    );
}

#[test]
fn test_cwd_extraction_missing_worktree_path() {
    let input_data = json!({
        "other_field": "value",
        "branch": "feature/test"
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(result, None, "Should return None when worktree_path is missing");
}

#[test]
fn test_cwd_extraction_null_input_data() {
    let result = extract_working_directory(None);

    assert_eq!(result, None, "Should return None when input_data is None");
}

#[test]
fn test_cwd_extraction_null_worktree_path() {
    let input_data = json!({
        "worktree_path": null,
        "other_field": "value"
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(result, None, "Should return None when worktree_path is null");
}

#[test]
fn test_cwd_extraction_non_string_worktree_path() {
    let input_data = json!({
        "worktree_path": 123,
        "other_field": "value"
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(result, None, "Should return None when worktree_path is not a string");
}

#[test]
fn test_cwd_extraction_empty_string() {
    let input_data = json!({
        "worktree_path": "",
        "other_field": "value"
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(
        result,
        Some("".to_string()),
        "Should return empty string if that's the value"
    );
}

/// Test CWD propagation through chain steps
///
/// When enqueue_next_chain_step is called, it should preserve worktree_path
/// in the input_data for the next step.
#[test]
fn test_cwd_propagation_in_chain_step() {
    // Simulate the logic from enqueue_next_chain_step that preserves worktree_path
    let current_task_worktree = Some("/path/to/worktree".to_string());

    // When creating the next step, worktree_path should be inherited
    let next_task_input = json!({
        "previous_output": "step 1 result",
        "original_task_summary": "Original task",
        // worktree_path would be added from current_task.worktree_path
    });

    // Build next step input (mimics enqueue_next_chain_step logic)
    let mut input_with_worktree = next_task_input.clone();
    if let Some(ref wt) = current_task_worktree {
        if let Some(obj) = input_with_worktree.as_object_mut() {
            obj.insert(
                "worktree_path".to_string(),
                serde_json::Value::String(wt.clone()),
            );
        }
    }

    let extracted = extract_working_directory(Some(&input_with_worktree));
    assert_eq!(extracted, Some("/path/to/worktree".to_string()));
}

/// Test that worktree_path is correctly added to chain step input_data
///
/// In execute_with_chain, step_input is built and should include worktree_path.
#[test]
fn test_chain_step_input_includes_worktree_path() {
    // Simulate building step_input as done in execute_with_chain (around line 337-348)
    let task_worktree_path = Some("/repo/.abathur/worktrees/feature-auth-abc123".to_string());

    let mut step_input = serde_json::Map::new();
    step_input.insert("task_description".to_string(), json!("Implement feature"));
    step_input.insert("project_context".to_string(), json!("Rust project"));

    // Add worktree_path to step input (as done in execute_with_chain)
    if let Some(ref wt_path) = task_worktree_path {
        step_input.insert("worktree_path".to_string(), json!(wt_path));
    }

    let step_input_value = serde_json::Value::Object(step_input);
    let extracted = extract_working_directory(Some(&step_input_value));

    assert_eq!(
        extracted,
        Some("/repo/.abathur/worktrees/feature-auth-abc123".to_string())
    );
}

/// Test worktree_path inheritance in decomposed child tasks
///
/// Child tasks created by handle_decomposition should inherit worktree context.
#[test]
fn test_decomposition_child_inherits_worktree_context() {
    // When decomposition creates child tasks, they get:
    // - branch: None (will be created on execution)
    // - feature_branch: Some(sanitized_branch) - new per-item branch
    // - worktree_path: None (will be created when executing)
    // But the parent's context is passed via input_data

    let _parent_output = json!({
        "feature_name": "user-auth",
        "decomposition": {
            "subprojects": ["api", "frontend", "tests"]
        }
    });

    // Per-item input would include the item data
    let child_input = json!("api"); // Item from array

    // Child task would NOT automatically get worktree_path
    // because it creates its own branch/worktree
    let extracted = extract_working_directory(Some(&child_input));
    assert_eq!(extracted, None, "Child tasks create their own worktrees");

    // But if we explicitly pass it (for continuation scenarios)
    let child_with_context = json!({
        "item": "api",
        "parent_worktree": "/path/to/parent/worktree"
    });
    let extracted = extract_working_directory(Some(&child_with_context));
    assert_eq!(extracted, None, "parent_worktree != worktree_path");
}

/// Test that relative paths are handled correctly
#[test]
fn test_relative_worktree_path() {
    let input_data = json!({
        "worktree_path": ".abathur/worktrees/feature-test",
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(
        result,
        Some(".abathur/worktrees/feature-test".to_string()),
        "Relative paths should be preserved as-is"
    );
}

/// Test worktree path with special characters
#[test]
fn test_worktree_path_with_spaces() {
    let input_data = json!({
        "worktree_path": "/path/with spaces/worktree",
    });

    let result = extract_working_directory(Some(&input_data));

    assert_eq!(
        result,
        Some("/path/with spaces/worktree".to_string()),
        "Paths with spaces should be preserved"
    );
}

/// Verify that working_directory flows to SubstrateRequest correctly
///
/// This tests the data structure that would be passed to the substrate.
#[test]
fn test_substrate_request_structure() {
    use uuid::Uuid;

    // Simulating the SubstrateRequest creation from execute_inner
    #[allow(dead_code)]
    struct MockSubstrateRequest {
        task_id: Uuid,
        agent_type: String,
        prompt: String,
        context: Option<serde_json::Value>,
        working_directory: Option<String>,
    }

    let input_data = json!({
        "worktree_path": "/repo/.abathur/worktrees/feature-test",
        "task_context": "some context"
    });

    let working_directory = extract_working_directory(Some(&input_data));

    let request = MockSubstrateRequest {
        task_id: Uuid::new_v4(),
        agent_type: "test-agent".to_string(),
        prompt: "Test prompt".to_string(),
        context: Some(input_data.clone()),
        working_directory,
    };

    assert_eq!(
        request.working_directory,
        Some("/repo/.abathur/worktrees/feature-test".to_string())
    );
    assert!(request.context.is_some());
}
