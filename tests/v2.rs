use schedule::{
    cli::demo_state,
    domain::*,
    ledger::*,
    store::decode_state,
    workspace::{self, LocalSettings, Project, ProjectMode},
};
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn call(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_schedule"))
        .arg("--data")
        .arg(dir)
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}
fn ok(dir: &Path, args: &[&str]) -> Value {
    let o = call(dir, args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
fn setup(p: &Path) {
    ok(p, &["model", "add", "a", "--capability", "5"]);
    ok(p, &["model", "add", "b", "--capability", "3"]);
    ok(
        p,
        &[
            "subscription",
            "set",
            "a",
            "--weekly-tokens",
            "100",
            "--renewal-day",
            "31",
        ],
    );
    ok(p, &["subscription", "set", "b", "--weekly-tokens", "200"]);
    ok(p, &["policy", "models", "--prefer", "a>b"]);
    ok(
        p,
        &[
            "task",
            "add",
            "重要任务",
            "--input",
            "100",
            "--minutes",
            "20",
        ],
    );
}
fn assignments(p: &Path) -> Vec<Value> {
    ok(p, &["plan", "--minutes", "60", "--reserve-percent", "0"])["plan"]["items"][0]["assignments"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn disabled_api_can_record_actual_overrun_but_cannot_plan_more() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["api", "set", "a", "--limit", "0"]);
    ok(
        p,
        &["usage", "log", "a", "--source", "api", "--input", "30"],
    );
    let quotas = ok(p, &["api", "list"]);
    assert_eq!(quotas[0]["used"], 30.0);
    assert_eq!(quotas[0]["available"], 0.0);
    assert!(assignments(p).iter().all(|a| a["funding"] != "api"));
}
#[test]
fn subscription_does_not_reinterpret_custom_quota_units() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    ok(
        p,
        &[
            "model",
            "add",
            "points",
            "--unit",
            "points",
            "--output-weight",
            "2",
        ],
    );
    let before = fs::read(p.join("state.json")).unwrap();
    assert!(
        !call(
            p,
            &["subscription", "set", "points", "--weekly-tokens", "500"]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(p.join("state.json")).unwrap(), before);
}
#[test]
fn api_zero_disables_and_downgrades() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["policy", "funding", "model-first"]);
    ok(p, &["api", "set", "a", "--limit", "0"]);
    ok(p, &["usage", "reconcile", "a", "week", "--percent", "100"]);
    let a = assignments(p);
    assert_eq!(a[0]["model"], "b");
    assert_eq!(a[0]["funding"], "subscription");
}
#[test]
fn model_first_uses_own_bounded_api_then_downgrades() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["policy", "funding", "model-first"]);
    ok(p, &["api", "set", "a", "--limit", "40"]);
    ok(p, &["api", "set", "b", "--limit", "9000"]);
    ok(p, &["usage", "reconcile", "a", "week", "--percent", "100"]);
    let a = assignments(p);
    assert_eq!(a.len(), 2);
    assert_eq!(a[0]["model"], "a");
    assert_eq!(a[0]["funding"], "api");
    assert_eq!(a[0]["input"], 40);
    assert_eq!(a[1]["model"], "b");
    assert_eq!(a[1]["input"], 60);
}
#[test]
fn subscription_first_preserves_api_for_later() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["api", "set", "a", "--limit", "100"]);
    ok(p, &["usage", "reconcile", "a", "week", "--percent", "100"]);
    let a = assignments(p);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["model"], "b");
}
#[test]
fn subscription_only_never_allocates_api() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["api", "set", "a", "--limit", "1000"]);
    ok(p, &["policy", "funding", "subscription-only"]);
    ok(p, &["usage", "reconcile", "a", "week", "--percent", "100"]);
    ok(p, &["usage", "reconcile", "b", "week", "--percent", "100"]);
    assert!(
        ok(p, &["plan"])["plan"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn no_split_allows_subscription_and_api_of_same_model() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["policy", "funding", "model-first"]);
    ok(p, &["task", "edit", "1", "--splittable", "false"]);
    ok(p, &["api", "set", "a", "--limit", "60"]);
    ok(p, &["usage", "reconcile", "a", "week", "--percent", "50"]);
    let a = assignments(p);
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|v| v["model"] == "a"));
    assert_eq!(a[0]["input"], 50);
    assert_eq!(a[1]["input"], 50);
}
#[test]
fn api_balances_and_reservations_are_per_model_and_source() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["api", "set", "a", "--limit", "100"]);
    ok(p, &["api", "set", "b", "--limit", "1000"]);
    ok(
        p,
        &[
            "budget", "set", "1", "a", "--source", "api", "--input", "50",
        ],
    );
    ok(
        p,
        &[
            "usage", "log", "a", "--task", "1", "--source", "api", "--input", "20",
        ],
    );
    let q = ok(p, &["quota", "list"]);
    let a = q
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["pool"] == "a" && q["funding"] == "api")
        .unwrap();
    assert_eq!(a["used"], 20.0);
    assert_eq!(a["reserved"], 30.0);
    let b = q
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["pool"] == "b" && q["funding"] == "api")
        .unwrap();
    assert_eq!(b["used"], 0.0);
    assert_eq!(b["available"], 1000.0);
    ok(p, &["reset", "--models", "a", "--source", "tibo"]);
    assert_eq!(ok(p, &["api", "list"])[0]["used"], 20.0);
}
#[test]
fn percent_observation_is_preserved_across_estimate_change() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["usage", "reconcile", "a", "week", "--percent", "80"]);
    ok(p, &["subscription", "estimate", "a", "200"]);
    let q = ok(p, &["quota", "list", "--model", "a"]);
    assert_eq!(q[0]["observed_percent"], 80.0);
    assert_eq!(q[0]["used"], 160.0);
    assert_eq!(q[0]["approximate"], true);
    ok(p, &["usage", "log", "a", "--input", "10"]);
    assert_eq!(ok(p, &["quota", "list", "--model", "a"])[0]["used"], 170.0);
}
#[test]
fn zero_cap_releases_old_api_reservations() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    ok(p, &["api", "set", "a", "--limit", "100"]);
    ok(
        p,
        &[
            "budget", "set", "1", "a", "--source", "api", "--input", "100",
        ],
    );
    ok(p, &["api", "set", "a", "--limit", "0"]);
    assert!(ok(p, &["budget", "list"]).as_array().unwrap().is_empty());
}
#[test]
fn work_journal_links_usage_without_double_counting() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    let w = ok(
        p,
        &[
            "work",
            "log",
            "1",
            "--minutes",
            "25",
            "--model",
            "a",
            "--input",
            "40",
            "--output",
            "10",
            "--done",
            "完成核心函数",
            "--learned",
            "边界需要单独处理",
            "--next",
            "补测试",
            "--progress",
            "50",
        ],
    );
    let r = ok(p, &["work", "summary", "1"]);
    assert_eq!(r["minutes"], 25);
    assert_eq!(r["input"], 40);
    assert_eq!(r["output"], 10);
    assert_eq!(r["sessions"][0]["learned"], "边界需要单独处理");
    ok(p, &["work", "void", &w["id"].to_string()]);
    let r = ok(p, &["work", "summary", "1"]);
    assert_eq!(r["minutes"], 0);
    assert_eq!(r["input"], 0);
    assert_eq!(r["task"]["progress"], 50.0);
}
#[test]
fn fee_flags_removed_and_json_contains_no_fee_fields() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    setup(p);
    assert!(
        !call(p, &["model", "add", "x", "--input-price", "1"])
            .status
            .success()
    );
    let s = fs::read_to_string(p.join("state.json")).unwrap();
    assert!(!s.contains("price"));
    assert!(!s.contains("cost"));
    assert!(!s.contains("currency"));
    assert!(!s.contains("monthly_fee"));
}
#[test]
fn v1_migration_preserves_usage_and_discards_retired_fee_fields() {
    let now = parse_time("2026-09-21T00:00:00Z", false).unwrap();
    let s = demo_state(now).unwrap();
    let mut v = serde_json::to_value(s).unwrap();
    v["version"] = serde_json::json!(1);
    v["currency"] = serde_json::json!("CNY");
    v["models"][0]["input_price"] = serde_json::json!(10);
    for e in v["events"].as_array_mut().unwrap() {
        e.as_object_mut().unwrap().remove("funding");
        e["cost"] = serde_json::json!(15);
    }
    let new = decode_state(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(new.version, 2);
    assert_eq!(new.quotas(now)[0].used, 84000.0);
    let text = serde_json::to_string(&new).unwrap();
    assert!(!text.contains("price"));
    assert!(!text.contains("cost"));
}
#[test]
fn renewal_clamps_calendar_days_and_is_optional() {
    let sub = Subscription {
        plan: "test".into(),
        renewal_day: Some(31),
        active: true,
        estimate_note: "".into(),
    };
    let today = chrono::NaiveDate::from_ymd_opt(2026, 2, 3).unwrap();
    assert_eq!(sub.next_renewal(today).unwrap().to_string(), "2026-02-28");
    assert!(
        Subscription {
            renewal_day: None,
            ..sub
        }
        .next_renewal(today)
        .is_none()
    );
}
#[test]
fn tc_alias_runs_same_data() {
    let d = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_tc"))
        .args(["--data", d.path().to_str().unwrap(), "--json", "init"])
        .output()
        .unwrap();
    assert!(o.status.success());
    assert_eq!(ok(d.path(), &["doctor"])["version"], 2);
}

fn git_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    workspace::git(path, &["init", "--initial-branch=main"]).unwrap();
    fs::write(path.join("hello.txt"), "hello").unwrap();
    workspace::git(path, &["add", "--", "hello.txt"]).unwrap();
    workspace::git(
        path,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-m",
            "initial",
        ],
    )
    .unwrap();
}
fn remote(root: &Path) -> String {
    let path = root.join("remote.git");
    fs::create_dir_all(&path).unwrap();
    workspace::git(&path, &["init", "--bare", "--initial-branch=main"]).unwrap();
    path.to_string_lossy().into()
}
#[test]
fn register_reference_keeps_paths_local_and_aggregates_dirty_status() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("代码 repo");
    git_repo(&repo);
    workspace::git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/project.git",
        ],
    )
    .unwrap();
    let mut s = State::default();
    let mut l = LocalSettings::default();
    workspace::register(
        &mut s,
        &mut l,
        "project".into(),
        &repo,
        None,
        None,
        "desc".into(),
    )
    .unwrap();
    assert_eq!(s.projects[0].mode, ProjectMode::Reference);
    assert!(
        !serde_json::to_string(&s)
            .unwrap()
            .contains(repo.to_str().unwrap())
    );
    fs::write(repo.join("hello.txt"), "changed").unwrap();
    assert_eq!(workspace::statuses(&s, &l)[0].changes.len(), 1);
}
#[test]
fn hub_sync_roundtrip_preserves_memory_and_does_not_copy_reference_code() {
    let d = tempfile::tempdir().unwrap();
    let r = remote(d.path());
    let mut a = LocalSettings {
        hub: Some(workspace::init_hub(&d.path().join("hub-a"), Some(&r)).unwrap()),
        ..LocalSettings::default()
    };
    let mut s = State::default();
    s.projects.push(Project {
        name: "repo".into(),
        mode: ProjectMode::Reference,
        remote: Some("https://github.com/example/code.git".into()),
        description: "".into(),
        context: "任务上下文".into(),
        memory: "长期规则".into(),
    });
    workspace::push(&s, &mut a).unwrap();
    assert!(
        !a.hub()
            .unwrap()
            .join("projects/repo/project.bundle")
            .exists()
    );
    let mut b = LocalSettings {
        hub: Some(workspace::clone_hub(&r, &d.path().join("hub-b")).unwrap()),
        ..LocalSettings::default()
    };
    let pulled = workspace::pull(&State::default(), &mut b, false).unwrap();
    assert_eq!(pulled.projects[0].memory, "长期规则");
    assert!(b.paths.is_empty());
}
#[test]
fn concurrent_workspace_changes_rejected_without_overwriting() {
    let d = tempfile::tempdir().unwrap();
    let r = remote(d.path());
    let mut a = LocalSettings {
        hub: Some(workspace::init_hub(&d.path().join("a"), Some(&r)).unwrap()),
        ..LocalSettings::default()
    };
    let mut s = State::default();
    workspace::push(&s, &mut a).unwrap();
    let mut b = LocalSettings {
        hub: Some(workspace::clone_hub(&r, &d.path().join("b")).unwrap()),
        ..LocalSettings::default()
    };
    let mut sb = workspace::pull(&State::default(), &mut b, false).unwrap();
    s.funding_policy = FundingPolicy::ModelFirst;
    workspace::push(&s, &mut a).unwrap();
    sb.funding_policy = FundingPolicy::SubscriptionOnly;
    assert!(workspace::push(&sb, &mut b).is_err());
    assert!(workspace::pull(&sb, &mut b, false).is_err());
    let restored = workspace::pull(&sb, &mut b, true).unwrap();
    assert_eq!(restored.funding_policy, FundingPolicy::ModelFirst);
}
#[test]
fn snapshot_bundle_restores_committed_project_without_mutating_source() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("source");
    git_repo(&repo);
    let mut s = State::default();
    let mut l = LocalSettings {
        hub: Some(workspace::init_hub(&d.path().join("hub"), None).unwrap()),
        ..LocalSettings::default()
    };
    workspace::register(
        &mut s,
        &mut l,
        "local-project".into(),
        &repo,
        None,
        None,
        "".into(),
    )
    .unwrap();
    let head = workspace::git(&repo, &["rev-parse", "HEAD"]).unwrap();
    workspace::snapshot(&s, &l, "local-project").unwrap();
    workspace::checkout(&s, &mut l, "local-project", &d.path().join("restored")).unwrap();
    assert_eq!(
        fs::read_to_string(d.path().join("restored/hello.txt")).unwrap(),
        "hello"
    );
    assert_eq!(workspace::git(&repo, &["rev-parse", "HEAD"]).unwrap(), head);
    assert!(
        workspace::git(&repo, &["status", "--porcelain"])
            .unwrap()
            .is_empty()
    );
}
#[test]
fn dirty_snapshot_and_unsafe_project_names_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("source");
    git_repo(&repo);
    let mut s = State::default();
    let mut l = LocalSettings {
        hub: Some(workspace::init_hub(&d.path().join("hub"), None).unwrap()),
        ..LocalSettings::default()
    };
    assert!(
        workspace::register(
            &mut s,
            &mut l,
            "../escape".into(),
            &repo,
            None,
            None,
            "".into()
        )
        .is_err()
    );
    workspace::register(&mut s, &mut l, "safe".into(), &repo, None, None, "".into()).unwrap();
    fs::write(repo.join("hello.txt"), "dirty").unwrap();
    assert!(workspace::snapshot(&s, &l, "safe").is_err());
    assert!(!workspace::portable_remote(
        "https://user:secret@github.com/a/b.git"
    ));
}
#[test]
fn push_does_not_stage_arbitrary_local_files() {
    let d = tempfile::tempdir().unwrap();
    let r = remote(d.path());
    let mut l = LocalSettings {
        hub: Some(workspace::init_hub(&d.path().join("hub"), Some(&r)).unwrap()),
        ..LocalSettings::default()
    };
    let s = State::default();
    fs::write(l.hub().unwrap().join("private.txt"), "private").unwrap();
    workspace::push(&s, &mut l).unwrap();
    assert!(
        !workspace::git(l.hub().unwrap(), &["ls-files"])
            .unwrap()
            .contains("private.txt")
    );
}
