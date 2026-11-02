//! The command surface, run as a real binary.

use assert_cmd::Command;
use predicates::str::contains;

fn immich_mount() -> Command {
    Command::cargo_bin("immich-mount").expect("the binary builds")
}

#[test]
fn llm_guide_embeds_every_doc() {
    let output = immich_mount().arg("--llm").assert().success();
    let guide = String::from_utf8(output.get_output().stdout.clone()).expect("the guide is UTF-8");
    for section in ["COMMAND REFERENCE", "README", "WORKFLOWS", "SPEC", "ARCHITECTURE", "PRINCIPLES", "ROADMAP"] {
        assert!(guide.contains(&format!("===== {section} =====")), "--llm is missing {section}");
    }
}

#[test]
fn llm_is_only_read_from_leading_arguments() {
    immich_mount().args(["completions", "bash", "--llm"]).assert().failure();
}

#[test]
fn man_page_renders() {
    immich_mount().arg("--man").assert().success().stdout(contains("immich-mount"));
}

#[test]
fn completions_render() {
    immich_mount().args(["completions", "zsh"]).assert().success().stdout(contains("immich-mount"));
}

#[test]
fn configure_round_trips_through_the_config_file() {
    let config_home = tempfile::tempdir().unwrap();
    let run = |arguments: &[&str]| {
        immich_mount().env("XDG_CONFIG_HOME", config_home.path()).args(arguments).assert()
    };
    run(&["configure", "set", "albums.layout", "flat"]).success().stdout(contains("flat"));
    run(&["configure", "get", "albums.layout"]).success().stdout("flat\n");
    run(&["configure", "set", "albums.layout", "sideways"]).failure();
    run(&["configure", "unset", "albums.layout"]).success().stdout("year-prefix\n");
    let listing = run(&["--json", "configure", "list"]).success();
    let rows: serde_json::Value = serde_json::from_slice(&listing.get_output().stdout).unwrap();
    assert!(rows.as_array().unwrap().iter().any(|row| row["key"] == "files.naming"));
}

#[test]
fn doctor_writes_one_json_document() {
    let output = immich_mount().args(["--json", "doctor"]).assert().success();
    let report: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).expect("one JSON document");
    assert!(report["checks"].as_array().unwrap().iter().any(|check| check["name"] == "fusermount3"));
}
