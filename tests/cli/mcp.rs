//! Tests for `abathur mcp ...`.

use super::{AssertExt, abathur_cmd, init_project, run_json};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn mcp_status_shows_servers() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    abathur_cmd(dir)
        .args(["mcp", "status"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("MCP Server Status")
                .and(predicates::str::contains("STOPPED")),
        );
}

#[test]
fn mcp_status_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_project(dir);

    let json = run_json(dir, &["mcp", "status", "--json"]);

    let servers = json["servers"]
        .as_array()
        .expect("servers should be an array");
    assert!(!servers.is_empty(), "Should list at least one server");

    // Each server should have required fields
    for server in servers {
        assert!(server["name"].as_str().is_some());
        assert!(server["port"].as_u64().is_some());
        assert!(server["running"].as_bool().is_some());
        // All servers should be stopped in a fresh project
        assert!(
            !server["running"].as_bool().unwrap(),
            "Server {} should be stopped",
            server["name"]
        );
    }
}

#[test]
fn mcp_help_shows_usage() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("Usage").and(predicates::str::contains("mcp")));
}

#[test]
fn mcp_memory_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "memory-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("memory-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host")),
        );
}

#[test]
fn mcp_tasks_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "tasks-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("tasks-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host")),
        );
}

#[test]
fn mcp_agents_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "agents-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("agents-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host")),
        );
}

#[test]
fn mcp_a2a_http_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "a2a-http", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("a2a-http"))
                .and(predicates::str::contains("--port"))
                .and(predicates::str::contains("--host"))
                .and(predicates::str::contains("--no-streaming"))
                .and(predicates::str::contains("--heartbeat-ms")),
        );
}

#[test]
fn mcp_all_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "all", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("all"))
                .and(predicates::str::contains("--memory-port"))
                .and(predicates::str::contains("--tasks-port"))
                .and(predicates::str::contains("--agents-port"))
                .and(predicates::str::contains("--a2a-port")),
        );
}

#[test]
fn mcp_stdio_help() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["mcp", "stdio", "--help"])
        .assert()
        .success_without_warnings()
        .stdout(
            predicates::str::contains("Usage")
                .and(predicates::str::contains("stdio"))
                .and(predicates::str::contains("--db-path"))
                .and(predicates::str::contains("--task-id")),
        );
}

#[test]
fn mcp_stdio_missing_db_path_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // mcp stdio requires --db-path; clap should reject without it
    abathur_cmd(dir)
        .args(["mcp", "stdio"])
        .assert()
        .failure()
        .stderr(predicates::str::is_match("required|--db-path").unwrap());
}
