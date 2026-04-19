//! Tests for `abathur init`.

use super::{AssertExt, abathur_cmd, init_project, run_json};
use tempfile::TempDir;

#[test]
fn init_creates_project_structure() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["init"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("initialized successfully"));

    // Verify the expected directories and files were created
    assert!(dir.join(".abathur").is_dir());
    assert!(dir.join(".abathur/abathur.db").exists());
    assert!(dir.join(".claude").is_dir());
}

#[test]
fn init_already_initialized_without_force() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // First init
    init_project(dir);

    // Second init without --force
    abathur_cmd(dir)
        .args(["init"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("already initialized"));
}

#[test]
fn init_force_reinitializes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    init_project(dir);

    abathur_cmd(dir)
        .args(["init", "--force"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("reinitialized successfully"));

    // Structure should still be intact
    assert!(dir.join(".abathur/abathur.db").exists());
}

#[test]
fn init_json_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let json = run_json(dir, &["init", "--json"]);

    assert_eq!(json["success"], true);
    assert!(json["message"].as_str().unwrap().contains("initialized"));
    assert_eq!(json["database_initialized"], true);
}

#[test]
fn init_json_already_initialized() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    init_project(dir);

    let json = run_json(dir, &["init", "--json"]);

    assert_eq!(json["success"], false);
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .contains("already initialized")
    );
}

#[test]
fn init_custom_path() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    abathur_cmd(dir)
        .args(["init", "subdir"])
        .assert()
        .success_without_warnings()
        .stdout(predicates::str::contains("initialized successfully"));

    assert!(dir.join("subdir/.abathur").is_dir());
    assert!(dir.join("subdir/.abathur/abathur.db").exists());
}
