//! Tests for `abathur memory ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn memory_store_creates_memory() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "store", "test-key", "test content"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Memory created"));
}

#[test]
fn memory_store_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "store", "test-key", "test content", "--json"],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert!(memory["id"].as_str().is_some(), "memory should have an id");
    assert_eq!(json_str(memory, "key"), "test-key");
    assert_eq!(json_str(memory, "namespace"), "default");
    assert_eq!(json_str(memory, "tier"), "working");
    assert_eq!(json_str(memory, "memory_type"), "fact");
}

#[test]
fn memory_recall_by_id() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory and get its ID
    let store = run_json(
        dir,
        &["memory", "store", "recall-key", "recall content", "--json"],
    );
    let id = json_str(&store["memory"], "id");

    // Recall by ID
    let recall = run_json(dir, &["memory", "recall", &id, "--json"]);

    assert_eq!(json_str(&recall["memory"], "id"), id);
    assert_eq!(json_str(&recall["memory"], "key"), "recall-key");
    assert_eq!(recall["content"].as_str().unwrap(), "recall content");
}

#[test]
fn memory_recall_by_key() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory
    let store = run_json(
        dir,
        &[
            "memory",
            "store",
            "key-recall",
            "key recall content",
            "--json",
        ],
    );
    let id = json_str(&store["memory"], "id");

    // Recall by key with namespace
    let recall = run_json(
        dir,
        &["memory", "recall", "key-recall", "-n", "default", "--json"],
    );

    assert_eq!(json_str(&recall["memory"], "id"), id);
    assert_eq!(json_str(&recall["memory"], "key"), "key-recall");
    assert_eq!(recall["content"].as_str().unwrap(), "key recall content");
}

#[test]
fn memory_search_finds_memories() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory with distinctive content
    abathur_cmd(dir)
        .args([
            "memory",
            "store",
            "searchable-key",
            "unique searchable content",
        ])
        .assert()
        .success_without_warnings();

    // Search for it
    let json = run_json(dir, &["memory", "search", "searchable", "--json"]);

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert!(
        !memories.is_empty(),
        "Search should find at least one memory"
    );
    assert!(
        memories
            .iter()
            .any(|m| json_str(m, "key") == "searchable-key"),
        "Search results should include the stored memory"
    );
}

#[test]
fn memory_list_shows_memories() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory
    abathur_cmd(dir)
        .args(["memory", "store", "listed-key", "listed content"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["memory", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("listed-key"));
}

#[test]
fn memory_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store two memories
    abathur_cmd(dir)
        .args(["memory", "store", "list-key-a", "content a"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "list-key-b", "content b"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["memory", "list", "--json"]);

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert!(memories.len() >= 2);
    assert!(json["total"].as_u64().unwrap() >= 2);
}

#[test]
fn memory_list_filter_by_namespace() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store memories in different namespaces
    abathur_cmd(dir)
        .args(["memory", "store", "ns-key", "ns content", "-n", "custom-ns"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "default-key", "default content"])
        .assert()
        .success_without_warnings();

    // Filter by custom namespace
    let json = run_json(dir, &["memory", "list", "-n", "custom-ns", "--json"]);

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert_eq!(memories.len(), 1, "Should only find memory in custom-ns");
    assert_eq!(json_str(&memories[0], "namespace"), "custom-ns");
}

#[test]
fn memory_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No memories found"));
}

#[test]
fn memory_forget_deletes_memory() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory
    let store = run_json(
        dir,
        &["memory", "store", "forget-key", "forget content", "--json"],
    );
    let id = json_str(&store["memory"], "id");

    // Forget it
    let forget = run_json(dir, &["memory", "forget", &id, "--json"]);

    assert_eq!(forget["success"], true);
    assert!(
        forget["message"].as_str().unwrap().contains("deleted"),
        "Message should confirm deletion"
    );

    // Verify it is gone from the list
    let list = run_json(dir, &["memory", "list", "--json"]);
    let memories = list["memories"].as_array().unwrap();
    assert!(
        !memories.iter().any(|m| json_str(m, "id") == id),
        "Deleted memory should not appear in list"
    );
}

#[test]
fn memory_prune_runs_maintenance() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "prune"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Maintenance complete"));
}

#[test]
fn memory_prune_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "prune", "--json"]);

    assert!(json.get("expired_pruned").is_some());
    assert!(json.get("decayed_pruned").is_some());
    assert!(json.get("promoted").is_some());
    assert!(json.get("conflicts_resolved").is_some());
}

#[test]
fn memory_prune_expired_only() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "prune", "--expired-only", "--json"]);

    assert!(json.get("expired_pruned").is_some());
    assert_eq!(
        json["decayed_pruned"].as_u64().unwrap(),
        0,
        "expired-only mode should not report decayed pruning"
    );
}

#[test]
fn memory_stats_shows_statistics() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["memory", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Memory Statistics"));
}

#[test]
fn memory_stats_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store one memory so stats are nonzero
    abathur_cmd(dir)
        .args(["memory", "store", "stats-key", "stats content"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["memory", "stats", "--json"]);

    assert!(json.get("working").is_some());
    assert!(json.get("episodic").is_some());
    assert!(json.get("semantic").is_some());
    assert!(json.get("total").is_some());
    assert!(
        json["working"].as_u64().unwrap() >= 1,
        "Should have at least one working memory"
    );
    assert_eq!(
        json["total"].as_u64().unwrap(),
        json["working"].as_u64().unwrap()
            + json["episodic"].as_u64().unwrap()
            + json["semantic"].as_u64().unwrap(),
        "Total should equal sum of tiers"
    );
}

#[test]
fn memory_stats_json_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["memory", "stats", "--json"]);

    assert_eq!(json["working"].as_u64().unwrap(), 0);
    assert_eq!(json["episodic"].as_u64().unwrap(), 0);
    assert_eq!(json["semantic"].as_u64().unwrap(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn memory_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Store a memory
    let store = run_json(
        dir,
        &[
            "memory",
            "store",
            "lifecycle-key",
            "lifecycle content",
            "--json",
        ],
    );
    assert_eq!(store["success"], true);
    let id = json_str(&store["memory"], "id");
    assert_eq!(json_str(&store["memory"], "key"), "lifecycle-key");

    // 2. List should include it
    let list = run_json(dir, &["memory", "list", "--json"]);
    let memories = list["memories"].as_array().unwrap();
    assert!(memories.iter().any(|m| json_str(m, "id") == id));

    // 3. Recall by ID
    let recall_id = run_json(dir, &["memory", "recall", &id, "--json"]);
    assert_eq!(json_str(&recall_id["memory"], "id"), id);
    assert_eq!(recall_id["content"].as_str().unwrap(), "lifecycle content");

    // 4. Recall by key
    let recall_key = run_json(
        dir,
        &[
            "memory",
            "recall",
            "lifecycle-key",
            "-n",
            "default",
            "--json",
        ],
    );
    assert_eq!(json_str(&recall_key["memory"], "id"), id);
    assert_eq!(recall_key["content"].as_str().unwrap(), "lifecycle content");

    // 5. Search
    let search = run_json(dir, &["memory", "search", "lifecycle", "--json"]);
    let search_results = search["memories"].as_array().unwrap();
    assert!(search_results.iter().any(|m| json_str(m, "id") == id));

    // 6. Stats should show one working memory
    let stats = run_json(dir, &["memory", "stats", "--json"]);
    assert!(stats["working"].as_u64().unwrap() >= 1);

    // 7. Forget the memory
    let forget = run_json(dir, &["memory", "forget", &id, "--json"]);
    assert_eq!(forget["success"], true);

    // 8. Stats should reflect deletion
    let stats_after = run_json(dir, &["memory", "stats", "--json"]);
    assert!(
        stats_after["total"].as_u64().unwrap() < stats["total"].as_u64().unwrap(),
        "Total should decrease after forgetting"
    );
}

#[test]
fn memory_store_with_tier_episodic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "epi-key",
            "episodic content",
            "--tier",
            "episodic",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert_eq!(json_str(memory, "tier"), "episodic");
    assert_eq!(json_str(memory, "key"), "epi-key");
}

#[test]
fn memory_store_with_tier_semantic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "sem-key",
            "semantic content",
            "--tier",
            "semantic",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert_eq!(json_str(memory, "tier"), "semantic");
    assert_eq!(json_str(memory, "key"), "sem-key");
}

#[test]
fn memory_store_with_type_code() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "code-key",
            "fn main() {}",
            "--memory-type",
            "code",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "code");
}

#[test]
fn memory_store_with_type_decision() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "decision-key",
            "chose option A",
            "--memory-type",
            "decision",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "decision");
}

#[test]
fn memory_store_with_type_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "error-key",
            "panicked at line 42",
            "--memory-type",
            "error",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "error");
}

#[test]
fn memory_store_with_type_pattern() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "pattern-key",
            "retry with backoff",
            "--memory-type",
            "pattern",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "pattern");
}

#[test]
fn memory_store_with_type_reference() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "ref-key",
            "see RFC 1234",
            "--memory-type",
            "reference",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "reference");
}

#[test]
fn memory_store_with_type_context() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "ctx-key",
            "running on linux",
            "--memory-type",
            "context",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["memory"], "memory_type"), "context");
}

#[test]
fn memory_search_with_namespace_filter() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store a memory in a custom namespace
    abathur_cmd(dir)
        .args([
            "memory",
            "store",
            "ns-search-key",
            "findme in custom",
            "-n",
            "custom-ns",
        ])
        .assert()
        .success_without_warnings();

    // Store a memory in the default namespace with similar content
    abathur_cmd(dir)
        .args(["memory", "store", "default-search-key", "findme in default"])
        .assert()
        .success_without_warnings();

    // Search with namespace filter
    let json = run_json(
        dir,
        &["memory", "search", "findme", "-n", "custom-ns", "--json"],
    );

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert!(
        !memories.is_empty(),
        "Should find at least one memory in custom-ns"
    );
    for mem in memories {
        assert_eq!(
            json_str(mem, "namespace"),
            "custom-ns",
            "All search results should be in custom-ns"
        );
    }
}

#[test]
fn memory_search_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store multiple memories with a common keyword
    abathur_cmd(dir)
        .args(["memory", "store", "limit-a", "limitword alpha"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "limit-b", "limitword beta"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "limit-c", "limitword gamma"])
        .assert()
        .success_without_warnings();

    // Search with limit 1
    let json = run_json(
        dir,
        &["memory", "search", "limitword", "--limit", "1", "--json"],
    );

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert!(
        memories.len() <= 1,
        "Should return at most 1 result when limit is 1"
    );
}

#[test]
fn memory_search_empty_results() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &["memory", "search", "zzz_nonexistent_term_zzz", "--json"],
    );

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert_eq!(
        memories.len(),
        0,
        "Search for nonexistent term should return empty results"
    );
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn memory_list_filter_by_tier_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store memories in different tiers
    abathur_cmd(dir)
        .args([
            "memory",
            "store",
            "working-key",
            "working content",
            "--tier",
            "working",
        ])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "memory",
            "store",
            "episodic-key",
            "episodic content",
            "--tier",
            "episodic",
        ])
        .assert()
        .success_without_warnings();

    // Filter by working tier
    let json = run_json(dir, &["memory", "list", "--tier", "working", "--json"]);

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert!(
        !memories.is_empty(),
        "Should find at least one working memory"
    );
    for mem in memories {
        assert_eq!(
            json_str(mem, "tier"),
            "working",
            "All listed memories should be in working tier"
        );
    }
}

#[test]
fn memory_list_with_limit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Store several memories
    abathur_cmd(dir)
        .args(["memory", "store", "lim-a", "content a"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "lim-b", "content b"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["memory", "store", "lim-c", "content c"])
        .assert()
        .success_without_warnings();

    // List with limit 2
    let json = run_json(dir, &["memory", "list", "--limit", "2", "--json"]);

    let memories = json["memories"]
        .as_array()
        .expect("memories should be an array");
    assert!(
        memories.len() <= 2,
        "Should return at most 2 memories when limit is 2"
    );
}

#[test]
fn memory_recall_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let fake_uuid = "00000000-0000-0000-0000-000000000000";

    // Recall with a nonexistent UUID should either fail or return a not-found message
    let output = abathur_cmd(dir)
        .args(["memory", "recall", fake_uuid, "--json"])
        .output()
        .unwrap();

    if output.status.success() {
        // If it succeeds, the JSON should indicate not found
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            json["success"], false,
            "Recalling nonexistent memory should report success=false"
        );
        assert!(
            json["message"].as_str().unwrap().contains("not found"),
            "Message should indicate memory not found"
        );
    }
    // If it fails (non-zero exit), that is also acceptable behavior
}

#[test]
fn memory_forget_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let fake_uuid = "00000000-0000-0000-0000-000000000000";

    abathur_cmd(dir)
        .args(["memory", "forget", fake_uuid])
        .assert()
        .failure();
}

#[test]
fn memory_store_invalid_tier_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "memory",
            "store",
            "bad-tier-key",
            "content",
            "--tier",
            "bogus",
        ])
        .assert()
        .failure();
}

#[test]
fn memory_store_invalid_type_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "memory",
            "store",
            "bad-type-key",
            "content",
            "--memory-type",
            "bogus",
        ])
        .assert()
        .failure();
}

#[test]
fn memory_store_all_options_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "memory",
            "store",
            "all-opts-key",
            "all options content",
            "-n",
            "myns",
            "--tier",
            "episodic",
            "--memory-type",
            "decision",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let memory = &json["memory"];
    assert_eq!(json_str(memory, "key"), "all-opts-key");
    assert_eq!(json_str(memory, "namespace"), "myns");
    assert_eq!(json_str(memory, "tier"), "episodic");
    assert_eq!(json_str(memory, "memory_type"), "decision");
}

#[test]
fn memory_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["memory", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("memory")));
}

#[test]
fn memory_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["memory", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No memories found"));

    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn memory_store_missing_key_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["memory", "store"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn memory_store_missing_content_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["memory", "store", "mykey"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}
