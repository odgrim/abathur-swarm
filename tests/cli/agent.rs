//! Tests for `abathur agent ...`.

use super::{AssertExt, abathur_cmd, init_project, json_str, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn agent_register_creates_worker() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "test-worker",
            "-p",
            "You are a test agent",
        ])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent created"));
}

#[test]
fn agent_register_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "test-worker",
            "-p",
            "You are a test agent",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let agent = &json["agent"];
    assert!(agent["id"].as_str().is_some(), "agent should have an id");
    assert_eq!(json_str(agent, "name"), "test-worker");
    assert_eq!(json_str(agent, "tier"), "worker");
    assert_eq!(json_str(agent, "status"), "active");
    assert_eq!(agent["version"].as_u64().unwrap(), 1);
}

#[test]
fn agent_register_specialist_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "test-specialist",
            "-p",
            "You are a specialist",
            "-t",
            "specialist",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "test-specialist");
    assert_eq!(json_str(&json["agent"], "tier"), "specialist");
}

#[test]
fn agent_register_with_tools() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "with-tools",
            "-p",
            "Agent with tools",
            "--tool",
            "bash:Run shell commands",
            "--tool",
            "read:Read files",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "with-tools");
    assert_eq!(json["agent"]["tools_count"].as_u64().unwrap(), 2);
}

#[test]
fn agent_list_shows_agents() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register an agent first
    abathur_cmd(dir)
        .args(["agent", "register", "listed-worker", "-p", "Listed agent"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["agent", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("listed-worker"));
}

#[test]
fn agent_list_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register two agents
    abathur_cmd(dir)
        .args(["agent", "register", "agent-a", "-p", "Agent A"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "agent-b",
            "-p",
            "Agent B",
            "-t",
            "specialist",
        ])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "list", "--json"]);

    let agents = json["agents"]
        .as_array()
        .expect("agents should be an array");
    assert!(agents.len() >= 2);
    assert!(json["total"].as_u64().unwrap() >= 2);
}

#[test]
fn agent_list_filter_by_tier() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register agents with different tiers
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "tier-worker",
            "-p",
            "A worker",
            "-t",
            "worker",
        ])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "tier-specialist",
            "-p",
            "A specialist",
            "-t",
            "specialist",
        ])
        .assert()
        .success_without_warnings();

    // Filter by worker tier
    let json = run_json(dir, &["agent", "list", "-t", "worker", "--json"]);

    let agents = json["agents"]
        .as_array()
        .expect("agents should be an array");
    assert!(!agents.is_empty(), "Should find worker agents");
    for agent in agents {
        assert_eq!(
            json_str(agent, "tier"),
            "worker",
            "All listed agents should be workers"
        );
    }
}

#[test]
fn agent_list_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No agents found"));
}

#[test]
fn agent_show_displays_agent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "show-worker", "-p", "Show agent"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["agent", "show", "show-worker"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("show-worker"));
}

#[test]
fn agent_show_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "show-json-worker",
            "-p",
            "Show JSON agent",
        ])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "show", "show-json-worker", "--json"]);

    assert_eq!(json_str(&json["agent"], "name"), "show-json-worker");
    assert_eq!(json_str(&json["agent"], "status"), "active");
    assert_eq!(json_str(&json["agent"], "tier"), "worker");
}

#[test]
fn agent_disable_disables_agent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "disable-worker", "-p", "Disable me"])
        .assert()
        .success_without_warnings();

    abathur_cmd(dir)
        .args(["agent", "disable", "disable-worker"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent disabled"));
}

#[test]
fn agent_disable_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "disable-json-worker",
            "-p",
            "Disable me",
        ])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "disable", "disable-json-worker", "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "status"), "disabled");
    assert_eq!(json_str(&json["agent"], "name"), "disable-json-worker");
}

#[test]
fn agent_enable_reenables_agent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "enable-worker", "-p", "Enable me"])
        .assert()
        .success_without_warnings();

    // Disable first
    abathur_cmd(dir)
        .args(["agent", "disable", "enable-worker"])
        .assert()
        .success_without_warnings();

    // Then re-enable
    abathur_cmd(dir)
        .args(["agent", "enable", "enable-worker"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent enabled"));
}

#[test]
fn agent_enable_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "register", "enable-json-worker", "-p", "Enable me"])
        .assert()
        .success_without_warnings();

    // Disable then enable
    abathur_cmd(dir)
        .args(["agent", "disable", "enable-json-worker"])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "enable", "enable-json-worker", "--json"]);

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "status"), "active");
    assert_eq!(json_str(&json["agent"], "name"), "enable-json-worker");
}

#[test]
fn agent_instances_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "instances"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No running instances"));
}

#[test]
fn agent_instances_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "instances", "--json"]);

    let instances = json["instances"]
        .as_array()
        .expect("instances should be an array");
    assert_eq!(instances.len(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
}

#[test]
fn agent_stats_shows_statistics() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "stats"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Agent Statistics"));
}

#[test]
fn agent_stats_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register agents of different tiers
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "stats-worker",
            "-p",
            "Worker",
            "-t",
            "worker",
        ])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "stats-specialist",
            "-p",
            "Specialist",
            "-t",
            "specialist",
        ])
        .assert()
        .success_without_warnings();

    let json = run_json(dir, &["agent", "stats", "--json"]);

    assert!(json.get("architect_count").is_some());
    assert!(json.get("specialist_count").is_some());
    assert!(json.get("worker_count").is_some());
    assert!(json.get("total").is_some());
    assert!(json.get("running_instances").is_some());

    assert!(json["worker_count"].as_u64().unwrap() >= 1);
    assert!(json["specialist_count"].as_u64().unwrap() >= 1);
    assert!(json["total"].as_u64().unwrap() >= 2);
    assert_eq!(json["running_instances"].as_u64().unwrap(), 0);
}

#[test]
fn agent_stats_json_empty() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "stats", "--json"]);

    assert_eq!(json["architect_count"].as_u64().unwrap(), 0);
    assert_eq!(json["specialist_count"].as_u64().unwrap(), 0);
    assert_eq!(json["worker_count"].as_u64().unwrap(), 0);
    assert_eq!(json["total"].as_u64().unwrap(), 0);
    assert_eq!(json["running_instances"].as_u64().unwrap(), 0);
}

#[test]
fn agent_gateway_status_graceful_when_not_running() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // The gateway-status command should succeed even when no gateway is running.
    // It reports the status as NOT RUNNING rather than failing.
    abathur_cmd(dir)
        .args(["agent", "gateway-status"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("NOT RUNNING"));
}

#[test]
fn agent_gateway_status_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "gateway-status", "--json"]);

    assert!(!json["running"].as_bool().unwrap());
    assert!(json["url"].as_str().is_some());
    assert!(json["message"].as_str().is_some());
    assert_eq!(json["agents"].as_u64().unwrap(), 0);
}

#[test]
fn agent_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // 1. Register an agent
    let register = run_json(
        dir,
        &[
            "agent",
            "register",
            "lifecycle-worker",
            "-p",
            "Lifecycle agent",
            "--json",
        ],
    );
    assert_eq!(register["success"], true);
    let name = json_str(&register["agent"], "name");
    assert_eq!(name, "lifecycle-worker");
    assert_eq!(json_str(&register["agent"], "status"), "active");
    assert_eq!(json_str(&register["agent"], "tier"), "worker");

    // 2. List should include it
    let list = run_json(dir, &["agent", "list", "--json"]);
    let agents = list["agents"].as_array().unwrap();
    assert!(
        agents
            .iter()
            .any(|a| json_str(a, "name") == "lifecycle-worker")
    );

    // 3. Disable the agent
    let disable = run_json(dir, &["agent", "disable", "lifecycle-worker", "--json"]);
    assert_eq!(disable["success"], true);
    assert_eq!(json_str(&disable["agent"], "status"), "disabled");

    // 4. Enable the agent
    let enable = run_json(dir, &["agent", "enable", "lifecycle-worker", "--json"]);
    assert_eq!(enable["success"], true);
    assert_eq!(json_str(&enable["agent"], "status"), "active");

    // 5. Stats should reflect the registered agent
    let stats = run_json(dir, &["agent", "stats", "--json"]);
    assert!(stats["worker_count"].as_u64().unwrap() >= 1);
    assert!(stats["total"].as_u64().unwrap() >= 1);
}

#[test]
fn agent_register_architect_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "test-architect",
            "-p",
            "You are an architect",
            "-t",
            "architect",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "test-architect");
    assert_eq!(json_str(&json["agent"], "tier"), "architect");
    assert_eq!(json["agent"]["version"].as_u64().unwrap(), 1);
}

#[test]
fn agent_register_with_description_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "described-agent",
            "-p",
            "A prompt",
            "-d",
            "A helper agent",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json_str(&json["agent"], "name"), "described-agent");

    // Verify description appears in agent show output
    let show = run_json(dir, &["agent", "show", "described-agent", "--json"]);
    assert_eq!(show["description"].as_str().unwrap(), "A helper agent");
}

#[test]
fn agent_register_with_max_turns_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "capped-agent",
            "-p",
            "A prompt",
            "--max-turns",
            "15",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    assert_eq!(json["agent"]["max_turns"].as_u64().unwrap(), 15);
}

#[test]
fn agent_list_active_only() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register two agents
    abathur_cmd(dir)
        .args(["agent", "register", "active-agent", "-p", "I stay active"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "disabled-agent",
            "-p",
            "I get disabled",
        ])
        .assert()
        .success_without_warnings();

    // Disable one
    abathur_cmd(dir)
        .args(["agent", "disable", "disabled-agent"])
        .assert()
        .success_without_warnings();

    // List with --active-only
    let json = run_json(dir, &["agent", "list", "--active-only", "--json"]);

    let agents = json["agents"]
        .as_array()
        .expect("agents should be an array");
    assert!(!agents.is_empty(), "Should have at least one active agent");
    for agent in agents {
        assert_eq!(
            json_str(agent, "status"),
            "active",
            "All listed agents should be active"
        );
    }
    // The disabled agent should not appear
    assert!(
        !agents
            .iter()
            .any(|a| json_str(a, "name") == "disabled-agent"),
        "Disabled agent should not appear in active-only list"
    );
}

#[test]
fn agent_register_versioning() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register the same agent name twice
    let v1 = run_json(
        dir,
        &[
            "agent",
            "register",
            "versioned-agent",
            "-p",
            "Version one prompt",
            "--json",
        ],
    );
    assert_eq!(v1["success"], true);
    assert_eq!(v1["agent"]["version"].as_u64().unwrap(), 1);

    let v2 = run_json(
        dir,
        &[
            "agent",
            "register",
            "versioned-agent",
            "-p",
            "Version two prompt",
            "--json",
        ],
    );
    assert_eq!(v2["success"], true);
    assert_eq!(v2["agent"]["version"].as_u64().unwrap(), 2);
}

#[test]
fn agent_show_specific_version() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register the same agent name twice to create v1 and v2
    abathur_cmd(dir)
        .args(["agent", "register", "multi-ver", "-p", "First version"])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args(["agent", "register", "multi-ver", "-p", "Second version"])
        .assert()
        .success_without_warnings();

    // Show with --version 1, verify version is 1
    let json = run_json(
        dir,
        &["agent", "show", "multi-ver", "--version", "1", "--json"],
    );
    assert_eq!(json["agent"]["version"].as_u64().unwrap(), 1);
}

#[test]
fn agent_show_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["agent", "show", "no-such-agent", "--json"]);

    assert_eq!(json["success"], false);
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("not found"),
        "Message should indicate agent was not found"
    );
}

#[test]
fn agent_disable_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "disable", "ghost-agent"])
        .assert()
        .failure();
}

#[test]
fn agent_enable_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["agent", "enable", "ghost-agent"])
        .assert()
        .failure();
}

#[test]
fn agent_register_invalid_tier_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "bad-tier-agent",
            "-p",
            "A prompt",
            "-t",
            "bogus",
        ])
        .assert()
        .failure();
}

#[test]
fn agent_list_filter_specialist_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Register a specialist and a worker
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "filter-specialist",
            "-p",
            "Specialist prompt",
            "-t",
            "specialist",
        ])
        .assert()
        .success_without_warnings();
    abathur_cmd(dir)
        .args([
            "agent",
            "register",
            "filter-worker",
            "-p",
            "Worker prompt",
            "-t",
            "worker",
        ])
        .assert()
        .success_without_warnings();

    // List filtered to specialist tier only
    let json = run_json(dir, &["agent", "list", "-t", "specialist", "--json"]);

    let agents = json["agents"]
        .as_array()
        .expect("agents should be an array");
    assert!(!agents.is_empty(), "Should find at least one specialist");
    for agent in agents {
        assert_eq!(
            json_str(agent, "tier"),
            "specialist",
            "All listed agents should be specialists"
        );
    }
    assert!(
        !agents
            .iter()
            .any(|a| json_str(a, "name") == "filter-worker"),
        "Worker agent should not appear in specialist-filtered list"
    );
}

#[test]
fn agent_register_with_all_options_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "register",
            "full-options-agent",
            "-d",
            "desc",
            "-t",
            "specialist",
            "-p",
            "prompt",
            "--tool",
            "bash:Run commands",
            "--max-turns",
            "40",
            "--json",
        ],
    );

    assert_eq!(json["success"], true);
    let agent = &json["agent"];
    assert_eq!(json_str(agent, "name"), "full-options-agent");
    assert_eq!(json_str(agent, "tier"), "specialist");
    assert_eq!(agent["version"].as_u64().unwrap(), 1);
    assert_eq!(json_str(agent, "status"), "active");
    assert_eq!(agent["tools_count"].as_u64().unwrap(), 1);
    // Specialist tier floor is 35, so 40 > 35 → effective is 40
    assert_eq!(agent["max_turns"].as_u64().unwrap(), 40);

    // Verify description and prompt appear in show output
    let show = run_json(dir, &["agent", "show", "full-options-agent", "--json"]);
    assert_eq!(show["description"].as_str().unwrap(), "desc");
    assert!(
        show["prompt_preview"].as_str().unwrap().contains("prompt"),
        "Show output should include the system prompt"
    );
    let tools = show["tools"].as_array().expect("tools should be an array");
    assert_eq!(tools.len(), 1);
    assert!(
        tools[0].as_str().unwrap().contains("bash"),
        "Tool entry should reference bash"
    );
}

#[test]
fn agent_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("agent")));
}

#[test]
fn agent_without_init_auto_creates_db() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    assert!(!dir.join(".abathur").exists());

    abathur_cmd(dir)
        .args(["agent", "list"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("No agents found"));

    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn agent_register_missing_prompt_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "register", "myname"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--prompt").unwrap());
}

#[test]
fn agent_send_missing_required_args_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Missing body, --to, and --subject should fail at clap validation
    abathur_cmd(dir)
        .args(["agent", "send"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_send_missing_to_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Provide body and --subject but omit --to
    abathur_cmd(dir)
        .args(["agent", "send", "Hello world", "--subject", "test-subject"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--to").unwrap());
}

#[test]
fn agent_send_missing_subject_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Provide body and --to but omit --subject
    abathur_cmd(dir)
        .args(["agent", "send", "Hello world", "--to", "agent-1"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--subject").unwrap());
}

#[test]
fn agent_send_missing_body_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Provide --to and --subject but omit the positional body argument
    abathur_cmd(dir)
        .args([
            "agent",
            "send",
            "--to",
            "agent-1",
            "--subject",
            "test-subject",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_send_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "send", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("--to"))
                .and(predicates::str::contains("--subject"))
                .and(predicates::str::contains("send")),
        );
}

#[test]
fn agent_send_gateway_unavailable_succeeds_with_failure_message() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Send a message to a gateway that is not running.
    // The command should succeed (exit 0) but report success: false in output.
    let json = run_json(
        dir,
        &[
            "agent",
            "send",
            "Hello world",
            "--to",
            "agent-1",
            "--subject",
            "test-subject",
            "--gateway",
            "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert!(
        !json["success"].as_bool().unwrap(),
        "send should report success: false when gateway is unreachable"
    );
    assert!(
        json["message_id"].as_str().is_some(),
        "should include a message_id"
    );
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("connect") || msg.contains("gateway") || msg.contains("Failed"),
        "message should indicate a connection failure, got: {}",
        msg
    );
}

#[test]
fn agent_send_gateway_unavailable_human_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Without --json, the human-readable output should mention the failure
    abathur_cmd(dir)
        .args([
            "agent",
            "send",
            "Hello world",
            "--to",
            "agent-1",
            "--subject",
            "test-subject",
            "--gateway",
            "http://127.0.0.1:19999",
        ])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::is_match("(?i)fail|connect|gateway").unwrap());
}

#[test]
fn agent_send_with_message_type_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Verify custom --message-type is accepted by clap even when gateway is down
    let json = run_json(
        dir,
        &[
            "agent",
            "send",
            "Error details",
            "--to",
            "agent-1",
            "--subject",
            "error-report",
            "--message-type",
            "error",
            "--gateway",
            "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert!(!json["success"].as_bool().unwrap());
    assert!(json["message_id"].as_str().is_some());
}

#[test]
fn agent_send_with_from_and_task_id_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // Use --from and --task-id options. task-id resolves to the UUID directly.
    // The send fails because the gateway is unreachable but exits 0 with success: false.
    let json = run_json(
        dir,
        &[
            "agent",
            "send",
            "Progress update",
            "--to",
            "agent-2",
            "--subject",
            "progress",
            "--from",
            "agent-1",
            "--task-id",
            "00000000-0000-0000-0000-000000000000",
            "--gateway",
            "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert!(!json["success"].as_bool().unwrap());
    assert!(json["message_id"].as_str().is_some());
}

#[test]
fn agent_cards_list_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "list", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("list")));
}

#[test]
fn agent_cards_export_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "export", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("export")));
}

#[test]
fn agent_cards_show_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "show", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("show"))
                .and(predicates::str::contains("AGENT_ID")),
        );
}

#[test]
fn agent_cards_show_missing_id_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Missing positional agent_id argument
    abathur_cmd(dir)
        .args(["agent", "cards", "show"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|Usage").unwrap());
}

#[test]
fn agent_cards_list_gateway_unavailable_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    // cards list should succeed (exit 0) but report failure when gateway is down
    let json = run_json(
        dir,
        &[
            "agent",
            "cards",
            "list",
            "--gateway",
            "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert!(
        !json["success"].as_bool().unwrap(),
        "cards list should report success: false when gateway is unreachable"
    );
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("connect") || msg.contains("gateway") || msg.contains("Cannot"),
        "message should indicate a connection failure, got: {}",
        msg
    );
}

#[test]
fn agent_cards_list_gateway_unavailable_human() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args([
            "agent",
            "cards",
            "list",
            "--gateway",
            "http://127.0.0.1:19999",
        ])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::is_match("(?i)cannot connect|gateway|fail").unwrap());
}

#[test]
fn agent_cards_export_gateway_unavailable_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "cards",
            "export",
            "--gateway",
            "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert!(
        !json["success"].as_bool().unwrap(),
        "cards export should report success: false when gateway is unreachable"
    );
    assert!(json["message"].as_str().is_some());
}

#[test]
fn agent_cards_show_gateway_unavailable_json() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(
        dir,
        &[
            "agent",
            "cards",
            "show",
            "some-agent-id",
            "--gateway",
            "http://127.0.0.1:19999",
            "--json",
        ],
    );

    assert!(
        !json["success"].as_bool().unwrap(),
        "cards show should report success: false when gateway is unreachable"
    );
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("connect") || msg.contains("gateway") || msg.contains("Cannot"),
        "message should indicate a connection failure, got: {}",
        msg
    );
}

#[test]
fn agent_cards_help_shows_subcommands() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["agent", "cards", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("list")
                .and(predicates::str::contains("export"))
                .and(predicates::str::contains("show")),
        );
}
