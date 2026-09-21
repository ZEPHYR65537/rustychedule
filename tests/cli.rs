use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

fn invoke(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_schedule"))
        .arg("--data")
        .arg(dir)
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}
fn ok(dir: &Path, args: &[&str]) -> Value {
    let o = invoke(dir, args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
#[test]
fn full_workflow_and_readonly_preview() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    ok(p, &["init", "--demo"]);
    let before = std::fs::read(p.join("state.json")).unwrap();
    let preview = ok(p, &["plan", "--minutes", "240", "--reserve-percent", "0"]);
    assert!(!preview["plan"]["items"].as_array().unwrap().is_empty());
    assert_eq!(before, std::fs::read(p.join("state.json")).unwrap());
    ok(p, &["plan", "--commit", "--reserve-percent", "0"]);
    let budgets = ok(p, &["budget", "list"]);
    assert!(!budgets.as_array().unwrap().is_empty());
    ok(
        p,
        &[
            "usage",
            "log",
            "deep",
            "--task",
            "1",
            "--input",
            "3000",
            "--output",
            "1000",
            "--progress",
            "25",
        ],
    );
    let t = ok(p, &["task", "show", "1"]);
    assert_eq!(t["task"]["progress"], 25.0);
    assert_eq!(t["actual_input"], 3000);
    ok(p, &["usage", "reconcile", "deep", "5h", "--percent", "100"]);
    let plan = ok(p, &["plan", "--rebalance", "--commit"]);
    assert!(plan["rebalanced"].as_bool().unwrap());
    ok(p, &["reset", "--models", "deep", "--source", "tibo"]);
    let quotas = ok(p, &["quota", "list", "--model", "deep"]);
    assert_eq!(quotas[0]["used"], 0.0);
    ok(p, &["task", "status", "1", "done"]);
    let t = ok(p, &["task", "show", "1"]);
    assert_eq!(t["task"]["progress"], 100.0);
    ok(p, &["report", "--days", "7"]);
    ok(p, &["doctor"]);
}
#[test]
fn invalid_changes_are_atomic() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    ok(p, &["init", "--demo"]);
    let before = std::fs::read(p.join("state.json")).unwrap();
    for args in [
        vec![
            "task",
            "edit",
            "1",
            "--prefer",
            "deep>fast",
            "--prefer",
            "fast>deep",
        ],
        vec!["task", "edit", "1", "--depends", "1"],
        vec!["usage", "log", "deep", "--input", "1", "--cached", "2"],
        vec!["task", "add", "bad", "--importance", "9"],
        vec!["reset", "--models", "fast", "--credit", "7"],
    ] {
        let out = invoke(p, &args);
        assert!(!out.status.success());
        assert_eq!(before, std::fs::read(p.join("state.json")).unwrap());
        assert!(serde_json::from_slice::<Value>(&out.stderr).is_ok());
    }
}
#[test]
fn export_import_and_corrupt_file_recovery() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("data");
    ok(&p, &["init", "--demo"]);
    let backup = d.path().join("backup.json");
    let b = backup.to_str().unwrap();
    ok(&p, &["export", b]);
    assert!(!invoke(&p, &["export", b]).status.success());
    ok(
        &p,
        &["task", "edit", "1", "--note", "create a valid backup"],
    );
    let good_backup = std::fs::read(p.join("state.json.bak")).unwrap();
    std::fs::write(p.join("state.json"), "not json").unwrap();
    assert!(!invoke(&p, &["doctor"]).status.success());
    ok(&p, &["import", b, "--replace"]);
    assert_eq!(ok(&p, &["doctor"])["valid"], true);
    assert_eq!(
        std::fs::read(p.join("state.json.bak")).unwrap(),
        good_backup
    );
    assert_eq!(
        std::fs::read(p.join("state.json.corrupt")).unwrap(),
        b"not json"
    );
}
#[test]
fn help_tree_and_empty_dashboard() {
    let d = tempfile::tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["task", "add", "--help"],
        vec!["task", "edit", "--help"],
        vec!["usage", "log", "--help"],
        vec!["reset", "--help"],
        vec!["plan", "--help"],
    ] {
        assert!(invoke(d.path(), &args).status.success());
    }
    assert!(ok(d.path(), &[])["models"].as_array().unwrap().is_empty());
}
#[test]
fn independent_model_quota_cli() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    ok(p, &["model", "add", "alpha"]);
    ok(p, &["model", "add", "beta"]);
    ok(
        p,
        &[
            "quota", "add", "alpha", "daily", "--limit", "100", "--period", "1d",
        ],
    );
    ok(
        p,
        &[
            "quota", "add", "beta", "daily", "--limit", "1000", "--period", "1d",
        ],
    );
    ok(p, &["usage", "log", "alpha", "--input", "70"]);
    ok(
        p,
        &["usage", "reconcile", "beta", "daily", "--percent", "30"],
    );
    let q = ok(p, &["quota", "list"]);
    assert_eq!(q[0]["used"], 70.0);
    assert_eq!(q[1]["used"], 300.0);
}
