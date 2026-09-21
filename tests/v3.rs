use schedule::{
    agents::{Catalog, Changes, HubLock},
    domain::State,
    hub,
    store::Store,
    workspace::{self, LocalSettings},
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn invoke(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tc"))
        .arg("--data")
        .arg(data)
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}
fn ok(data: &Path, args: &[&str]) -> Value {
    let out = invoke(data, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn bad(data: &Path, args: &[&str]) -> String {
    let out = invoke(data, args);
    assert!(!out.status.success(), "expected failure {args:?}");
    String::from_utf8(out.stderr).unwrap()
}
fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn id(value: &Value) -> String {
    value["id"].as_str().unwrap().into()
}
fn init(root: &Path) -> (PathBuf, PathBuf) {
    let data = root.join("data");
    let sync = root.join("hub");
    ok(&data, &["hub", "init", s(&sync)]);
    ok(&data, &["machine", "alias", "主机"]);
    (data, sync)
}
fn bare(path: &Path) {
    fs::create_dir_all(path).unwrap();
    workspace::git(path, &["init", "--bare", "--initial-branch=main"]).unwrap();
}
fn code_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    workspace::git(path, &["init", "--initial-branch=main"]).unwrap();
    fs::write(path.join("code.txt"), "original code").unwrap();
    workspace::git(path, &["add", "."]).unwrap();
    workspace::git(
        path,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@example.test",
            "commit",
            "-m",
            "initial",
        ],
    )
    .unwrap();
}
fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}
fn import(data: &Path, agent: &str, path: &Path) -> String {
    let value = ok(data, &["agent", "import", agent, s(path), "--apply"]);
    value["summary"]["root"].as_str().unwrap().into()
}
fn content(sync: &Path, node: &str, file: &str) -> PathBuf {
    sync.join("content").join(node).join(file)
}

#[test]
fn directories_and_nested_repositories_keep_correct_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A", "--provider", "codex"]));
    let source = tmp.path().join("X");
    write(&source.join("MEMORY.md"), "持久记忆");
    write(&source.join("sub/script.py"), "print(1)");
    write(&source.join("asset.bin"), "binary-ish");
    write(&source.join(".gitignore"), "ignored/\n*.tmp\n!keep.tmp\n");
    write(&source.join(".tcignore"), "private.txt\n");
    write(&source.join("ignored/a.txt"), "ignored");
    write(&source.join("private.txt"), "private");
    write(&source.join("keep.tmp"), "kept");
    write(&source.join("discard.tmp"), "ignored");
    write(&source.join(".env"), "SECRET");
    write(&source.join("target/cache"), "cache");
    code_repo(&source.join("code"));
    let preview = ok(&data, &["agent", "import", &a, s(&source)]);
    assert!(!preview["applied"].as_bool().unwrap());
    assert!(Catalog::load(&sync).unwrap().nodes.is_empty());
    let n = import(&data, &a, &source);
    let cat = Catalog::load(&sync).unwrap();
    assert_eq!(cat.repos.len(), 1);
    let repo = cat.repos.values().next().unwrap();
    assert!(repo.remote.is_none());
    assert_eq!(cat.machines[&repo.locations[0].machine].alias, "主机");
    assert_eq!(
        fs::read_to_string(content(&sync, &n, "MEMORY.md")).unwrap(),
        "持久记忆"
    );
    assert!(content(&sync, &n, "keep.tmp").exists());
    assert!(!content(&sync, &n, ".env").exists());
    assert!(!cat.nodes.values().any(|n| n.name == "ignored"));
    let export = tmp.path().join("restore");
    ok(&data, &["agent", "export", &a, "--to", s(&export)]);
    assert_eq!(
        fs::read_to_string(export.join("X/sub/script.py")).unwrap(),
        "print(1)"
    );
    assert!(!export.join("X/code/code.txt").exists());
    assert!(export.join(".tc-export.json").exists());
    assert_eq!(
        workspace::git(&source.join("code"), &["status", "--porcelain"]).unwrap(),
        ""
    );
    ok(&data, &["hub", "check"]);
}
#[test]
fn a_has_x_y_z_and_structure_changes_leave_ledger_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    ok(&data, &["init", "--demo"]);
    let before = fs::read(data.join("state.json")).unwrap();
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let b = id(&ok(&data, &["agent", "create", "B"]));
    let x = id(&ok(&data, &["agent", "folder", &a, "X"]));
    ok(&data, &["agent", "folder", &a, "Y"]);
    ok(&data, &["agent", "folder", &a, "Z"]);
    ok(&data, &["agent", "link", &a, "--task", "1", "--task", "2"]);
    ok(&data, &["agent", "link", &b, "--task", "1"]);
    let chat = id(&ok(
        &data,
        &["agent", "chat", &a, "讨论", "--source", "chat-123"],
    ));
    ok(
        &data,
        &["agent", "link", &chat, "--task", "1", "--task", "3"],
    );
    let tree = ok(&data, &["agent", "tree", &a]);
    assert!(tree["tree"].as_str().unwrap().contains("X [文件夹]"));
    let split = id(&ok(&data, &["agent", "split", &x, "--name", "分离"]));
    let cat = Catalog::load(&sync).unwrap();
    assert_eq!(cat.nodes[&x].agent, split);
    ok(&data, &["agent", "merge", &split, &b, "--apply"]);
    let cat = Catalog::load(&sync).unwrap();
    assert!(cat.agents[&split].archived);
    assert_eq!(cat.agents[&split].merged_into, Some(b.clone()));
    assert_eq!(cat.nodes[&x].agent, b);
    assert_eq!(before, fs::read(data.join("state.json")).unwrap());
}
#[test]
fn machine_aliases_and_repo_references_keep_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let code = tmp.path().join("code");
    code_repo(&code);
    let r = id(&ok(&data, &["repo", "add", "代码", s(&code)]));
    let cat = Catalog::load(&sync).unwrap();
    let machine = cat.repos[&r].locations[0].machine.clone();
    ok(&data, &["machine", "alias", "实验机"]);
    assert_eq!(
        Catalog::load(&sync).unwrap().machines[&machine].alias,
        "实验机"
    );
    let err = bad(
        &data,
        &["repo", "checkout", &r, "--to", s(&tmp.path().join("copy"))],
    );
    assert!(err.contains("只有本地索引"));
    ok(
        &data,
        &["repo", "remote", &r, "https://github.com/example/code.git"],
    );
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let b = id(&ok(&data, &["agent", "create", "B"]));
    ok(&data, &["agent", "attach", &a, &r]);
    ok(&data, &["agent", "attach", &b, &r]);
    let cat = Catalog::load(&sync).unwrap();
    assert_eq!(cat.repos.len(), 1);
    assert_eq!(cat.nodes.len(), 2);
    assert!(
        bad(
            &data,
            &[
                "repo",
                "remote",
                &r,
                "https://secret@github.com/example/code.git"
            ]
        )
        .contains("凭据")
    );
}
#[test]
fn collect_three_way_conflicts_missing_sources_and_prune() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let source = tmp.path().join("X");
    write(&source.join("memo.md"), "base");
    let n = import(&data, &a, &source);
    write(&content(&sync, &n, "memo.md"), "central");
    ok(&data, &["agent", "collect", &n, "--apply"]);
    assert_eq!(
        fs::read_to_string(content(&sync, &n, "memo.md")).unwrap(),
        "central"
    );
    write(&source.join("memo.md"), "source");
    assert!(bad(&data, &["agent", "collect", &n, "--apply"]).contains("双方都有修改"));
    assert_eq!(
        fs::read_to_string(content(&sync, &n, "memo.md")).unwrap(),
        "central"
    );
    write(&content(&sync, &n, "memo.md"), "source");
    ok(&data, &["agent", "collect", &n, "--apply"]);
    fs::remove_file(source.join("memo.md")).unwrap();
    ok(&data, &["agent", "collect", &n, "--apply"]);
    assert!(content(&sync, &n, "memo.md").exists());
    ok(&data, &["agent", "collect", &n, "--apply", "--prune"]);
    assert!(!content(&sync, &n, "memo.md").exists());
    fs::rename(&source, tmp.path().join("offline")).unwrap();
    assert!(bad(&data, &["agent", "collect", &n, "--apply", "--prune"]).contains("不会推断为删除"));
}
#[test]
fn export_bind_can_collect_back_without_overwriting_other_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let source = tmp.path().join("X");
    write(&source.join("sub/memo.md"), "base");
    let n = import(&data, &a, &source);
    let target = tmp.path().join("restored");
    ok(
        &data,
        &["agent", "export", &a, "--to", s(&target), "--bind"],
    );
    write(&target.join("X/sub/memo.md"), "updated");
    ok(&data, &["agent", "collect", &n, "--apply"]);
    let cat = Catalog::load(&sync).unwrap();
    let sub = cat.nodes.values().find(|n| n.name == "sub").unwrap();
    assert_eq!(
        fs::read_to_string(content(&sync, &sub.id, "memo.md")).unwrap(),
        "updated"
    );
    assert_eq!(
        fs::read_to_string(source.join("sub/memo.md")).unwrap(),
        "base"
    );
}
#[test]
fn two_devices_selective_content_and_agent_only_ledger_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let remote = tmp.path().join("remote.git");
    bare(&remote);
    ok(&data, &["hub", "remote", s(&remote)]);
    ok(&data, &["init", "--demo"]);
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let b = id(&ok(&data, &["agent", "create", "B"]));
    let source = tmp.path().join("X");
    write(&source.join("memo.md"), "base");
    let n = import(&data, &a, &source);
    let source2 = tmp.path().join("Y");
    write(&source2.join("memo.md"), "B file");
    let n2 = import(&data, &b, &source2);
    ok(&data, &["hub", "push"]);
    let data2 = tmp.path().join("device2");
    let sync2 = tmp.path().join("hub2");
    ok(
        &data2,
        &["hub", "clone", s(&remote), s(&sync2), "--metadata-only"],
    );
    ok(&data2, &["hub", "pull", "--agents-only"]);
    assert!(!data2.join("state.json").exists());
    assert!(!content(&sync2, &n, "memo.md").exists());
    ok(&data2, &["hub", "select", &a]);
    assert!(content(&sync2, &n, "memo.md").exists());
    assert!(!content(&sync2, &n2, "memo.md").exists());
    assert!(
        bad(
            &data2,
            &[
                "agent",
                "export",
                &b,
                "--to",
                s(&tmp.path().join("incomplete"))
            ]
        )
        .contains("正文未下载")
    );
    ok(&data2, &["machine", "alias", "笔记本"]);
    write(&content(&sync2, &n, "memo.md"), "from second device");
    ok(&data2, &["hub", "push", "--agents-only"]);
    ok(&data, &["hub", "pull"]);
    assert_eq!(
        fs::read_to_string(content(&sync, &n, "memo.md")).unwrap(),
        "from second device"
    );
    let cat = Catalog::load(&sync).unwrap();
    assert_eq!(cat.machines.len(), 2);
    ok(&data2, &["hub", "pull"]);
    assert_eq!(ok(&data2, &["task", "list"]).as_array().unwrap().len(), 5);
    ok(&data2, &["hub", "select", "--all"]);
    assert!(content(&sync2, &n2, "memo.md").exists());
}
#[test]
fn divergent_edits_merge_but_semantic_collision_preserves_primary_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, _sync) = init(tmp.path());
    let remote = tmp.path().join("remote.git");
    bare(&remote);
    ok(&data, &["hub", "remote", s(&remote)]);
    let a = id(&ok(&data, &["agent", "create", "A"]));
    ok(&data, &["hub", "push"]);
    let data2 = tmp.path().join("d2");
    let sync2 = tmp.path().join("h2");
    ok(&data2, &["hub", "clone", s(&remote), s(&sync2)]);
    ok(&data, &["agent", "folder", &a, "X"]);
    ok(&data, &["hub", "push"]);
    ok(&data2, &["agent", "folder", &a, "Y"]);
    ok(&data2, &["hub", "commit"]);
    ok(&data2, &["hub", "pull", "--agents-only"]);
    assert_eq!(Catalog::load(&sync2).unwrap().nodes.len(), 2);
    ok(&data2, &["hub", "push", "--agents-only"]);
    ok(&data, &["hub", "pull", "--agents-only"]);
    ok(&data, &["agent", "folder", &a, "collision"]);
    ok(&data, &["hub", "push", "--agents-only"]);
    ok(&data2, &["agent", "folder", &a, "collision"]);
    ok(&data2, &["hub", "commit"]);
    let head = workspace::git(&sync2, &["rev-parse", "HEAD"]).unwrap();
    assert!(bad(&data2, &["hub", "pull", "--agents-only"]).contains("同级名称冲突"));
    assert_eq!(
        head,
        workspace::git(&sync2, &["rev-parse", "HEAD"]).unwrap()
    );
    assert!(
        workspace::git(&sync2, &["status", "--porcelain"])
            .unwrap()
            .is_empty()
    );
}
#[test]
fn graph_validation_prevents_cycles_names_and_unsafe_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let x = id(&ok(&data, &["agent", "folder", &a, "X"]));
    let y = id(&ok(&data, &["agent", "folder", &a, "Y", "--parent", &x]));
    assert!(bad(&data, &["agent", "move", &x, "--parent", &y]).contains("自身"));
    assert!(bad(&data, &["agent", "folder", &a, "x"]).contains("名称冲突"));
    assert!(bad(&data, &["agent", "folder", &a, "../escape"]).contains("跨目录"));
    assert_eq!(Catalog::load(&sync).unwrap().nodes[&x].parent, None);
    let lock = HubLock::open(&sync).unwrap();
    let change = Changes::from([(PathBuf::from("../escape.txt"), Some(b"bad".to_vec()))]);
    assert!(lock.apply(&change).is_err());
    assert!(!tmp.path().join("escape.txt").exists());
}
#[test]
fn remove_requires_saved_version_and_keeps_source_and_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let source = tmp.path().join("X");
    write(&source.join("memo.md"), "base");
    code_repo(&source.join("code"));
    let n = import(&data, &a, &source);
    assert!(bad(&data, &["agent", "remove", &n, "--apply"]).contains("先 hub commit"));
    ok(&data, &["hub", "commit"]);
    ok(&data, &["agent", "remove", &n, "--apply"]);
    ok(&data, &["hub", "check"]);
    assert!(source.join("memo.md").exists());
    assert!(source.join("code/.git").exists());
    assert_eq!(Catalog::load(&sync).unwrap().repos.len(), 1);
    assert!(!content(&sync, &n, "memo.md").exists());
}
#[test]
fn migration_preserves_ledger_memory_and_legacy_bundle_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let store = Store::open(data.clone()).unwrap();
    let mut state = State::default();
    let sync = workspace::init_hub(&tmp.path().join("hub"), None).unwrap();
    let remote = tmp.path().join("remote.git");
    bare(&remote);
    workspace::git(&sync, &["remote", "add", "origin", s(&remote)]).unwrap();
    let code = tmp.path().join("code");
    code_repo(&code);
    let mut local = LocalSettings {
        hub: Some(sync.clone()),
        ..LocalSettings::default()
    };
    workspace::register(
        &mut state,
        &mut local,
        "old".into(),
        &code,
        None,
        None,
        String::new(),
    )
    .unwrap();
    state.projects[0].memory = "旧记忆".into();
    workspace::snapshot(&state, &local, "old").unwrap();
    workspace::push(&state, &mut local).unwrap();
    store.save(&state).unwrap();
    local.save(&store).unwrap();
    drop(store);
    let before = fs::read(data.join("state.json")).unwrap();
    ok(&data, &["hub", "migrate"]);
    assert_eq!(hub::format(&sync).unwrap(), 1);
    ok(&data, &["hub", "migrate", "--apply"]);
    assert_eq!(hub::format(&sync).unwrap(), 2);
    assert_eq!(before, fs::read(data.join("state.json")).unwrap());
    assert!(sync.join("legacy/v02/projects/old/project.bundle").exists());
    assert!(workspace::check_hub(&sync).is_err());
    let cat = Catalog::load(&sync).unwrap();
    let folder = cat.nodes.values().find(|n| n.name == "old").unwrap();
    assert_eq!(
        fs::read_to_string(content(&sync, &folder.id, "MEMORY.md")).unwrap(),
        "旧记忆"
    );
    assert!(
        ok(&data, &["hub", "migrate", "--apply"])["already_migrated"]
            .as_bool()
            .unwrap()
    );
    let r = cat.repos.keys().next().unwrap();
    ok(
        &data,
        &[
            "repo",
            "checkout",
            r,
            "--to",
            s(&tmp.path().join("restored")),
        ],
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("restored/code.txt")).unwrap(),
        "original code"
    );
}
#[test]
fn interrupted_transaction_is_restored_before_next_command() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let path = PathBuf::from(format!("catalog/agents/{a}.json"));
    let original = fs::read(sync.join(&path)).unwrap();
    let journal = sync.join(".git/schedule-transaction");
    fs::create_dir(&journal).unwrap();
    fs::write(journal.join("0.bin"), &original).unwrap();
    fs::write(
        journal.join("undo.json"),
        serde_json::to_vec(&serde_json::json!([{"path":path,"backup":"0.bin"}])).unwrap(),
    )
    .unwrap();
    fs::write(sync.join(&path), "broken write").unwrap();
    ok(&data, &["hub", "check"]);
    assert_eq!(original, fs::read(sync.join(path)).unwrap());
    assert!(!journal.exists());
}

#[test]
fn content_sync_can_continue_when_local_planning_diverges() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, _sync) = init(tmp.path());
    let remote = tmp.path().join("remote.git");
    bare(&remote);
    ok(&data, &["hub", "remote", s(&remote)]);
    ok(&data, &["init", "--demo"]);
    ok(&data, &["agent", "create", "A"]);
    ok(&data, &["hub", "push"]);
    let other = tmp.path().join("other");
    let hub2 = tmp.path().join("hub2");
    ok(&other, &["hub", "clone", s(&remote), s(&hub2)]);
    ok(&other, &["hub", "pull"]);
    ok(&data, &["policy", "funding", "model-first"]);
    ok(&data, &["hub", "push"]);
    ok(&other, &["policy", "funding", "subscription-only"]);
    let before = fs::read(other.join("state.json")).unwrap();
    assert!(bad(&other, &["hub", "pull"]).contains("账本都已改变"));
    ok(&other, &["hub", "pull", "--agents-only"]);
    assert_eq!(before, fs::read(other.join("state.json")).unwrap());
    ok(&other, &["agent", "create", "B"]);
    ok(&other, &["hub", "push", "--agents-only"]);
    assert!(bad(&other, &["hub", "push"]).contains("规划副本"));
}

#[test]
fn text_conflicts_do_not_dirty_original_hub() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let remote = tmp.path().join("remote.git");
    bare(&remote);
    ok(&data, &["hub", "remote", s(&remote)]);
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let x = tmp.path().join("X");
    write(&x.join("memo.md"), "base\n");
    let n = import(&data, &a, &x);
    ok(&data, &["hub", "push"]);
    let other = tmp.path().join("other");
    let hub2 = tmp.path().join("hub2");
    ok(&other, &["hub", "clone", s(&remote), s(&hub2)]);
    write(&content(&sync, &n, "memo.md"), "left\n");
    ok(&data, &["hub", "push"]);
    write(&content(&hub2, &n, "memo.md"), "right\n");
    ok(&other, &["hub", "commit"]);
    let before = workspace::git(&hub2, &["rev-parse", "HEAD"]).unwrap();
    assert!(bad(&other, &["hub", "pull", "--agents-only"]).contains("合并存在冲突"));
    assert_eq!(
        before,
        workspace::git(&hub2, &["rev-parse", "HEAD"]).unwrap()
    );
    assert!(
        workspace::git(&hub2, &["status", "--porcelain"])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(content(&hub2, &n, "memo.md")).unwrap(),
        "right\n"
    );
}

#[test]
fn second_legacy_device_follows_central_migration_identities() {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    bare(&remote);
    let sync = workspace::init_hub(&tmp.path().join("hub"), Some(s(&remote))).unwrap();
    let state = State::default();
    let mut local = LocalSettings {
        hub: Some(sync.clone()),
        ..LocalSettings::default()
    };
    workspace::push(&state, &mut local).unwrap();
    let data = tmp.path().join("data");
    {
        let store = Store::open(data.clone()).unwrap();
        store.save(&state).unwrap();
        local.save(&store).unwrap();
    }
    let other = tmp.path().join("other");
    let hub2 = tmp.path().join("hub2");
    ok(&other, &["hub", "clone", s(&remote), s(&hub2)]);
    ok(&other, &["hub", "pull"]);
    ok(&data, &["hub", "migrate", "--apply"]);
    let agent = id(&ok(&data, &["agent", "create", "New"]));
    ok(&data, &["hub", "push"]);
    let result = ok(&other, &["hub", "pull"]);
    assert!(result["upgraded"].as_bool().unwrap());
    assert!(Catalog::load(&hub2).unwrap().agents.contains_key(&agent));
    assert_eq!(hub::format(&hub2).unwrap(), 2);
}

#[test]
fn selecting_folder_keeps_catalog_and_missing_is_not_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let x = tmp.path().join("X");
    write(&x.join("memo.md"), "base");
    let n = import(&data, &a, &x);
    let y = tmp.path().join("Y");
    write(&y.join("memo.md"), "another");
    let n2 = import(&data, &a, &y);
    ok(&data, &["hub", "commit"]);
    ok(&data, &["hub", "select", &n]);
    assert!(content(&sync, &n, "memo.md").exists());
    assert!(!content(&sync, &n2, "memo.md").exists());
    assert_eq!(Catalog::load(&sync).unwrap().nodes.len(), 2);
    assert!(bad(&data, &["agent", "collect", &n2, "--apply"]).contains("正文未选择"));
    assert!(
        workspace::git(&sync, &["status", "--porcelain"])
            .unwrap()
            .is_empty()
    );
    ok(&data, &["hub", "select", "--all"]);
    assert!(content(&sync, &n2, "memo.md").exists());
}

#[cfg(unix)]
#[test]
fn symlink_payloads_never_escape_collection_or_restoration() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let (data, sync) = init(tmp.path());
    let a = id(&ok(&data, &["agent", "create", "A"]));
    let source = tmp.path().join("X");
    write(&source.join("memo.md"), "allowed");
    let outside = tmp.path().join("outside");
    write(&outside.join("secret.md"), "private");
    symlink(&outside, source.join("escape")).unwrap();
    let n = import(&data, &a, &source);
    assert!(
        !Catalog::load(&sync)
            .unwrap()
            .nodes
            .values()
            .any(|n| n.name == "escape")
    );
    symlink(outside.join("secret.md"), content(&sync, &n, "injected.md")).unwrap();
    assert!(bad(&data, &["hub", "check"]).contains("符号链接"));
}

#[test]
fn initializing_first_hub_imports_preexisting_v2_registrations() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let code = tmp.path().join("code");
    code_repo(&code);
    ok(&data, &["project", "register", "old", s(&code)]);
    ok(
        &data,
        &["project", "note", "old", "--memory", "existing memory"],
    );
    let before = fs::read(data.join("state.json")).unwrap();
    let sync = tmp.path().join("hub");
    ok(&data, &["hub", "init", s(&sync)]);
    let cat = Catalog::load(&sync).unwrap();
    assert_eq!(cat.repos.len(), 1);
    assert_eq!(cat.agents.len(), 1);
    let node = cat.nodes.values().find(|n| n.name == "old").unwrap();
    assert_eq!(
        fs::read_to_string(content(&sync, &node.id, "MEMORY.md")).unwrap(),
        "existing memory"
    );
    assert_eq!(before, fs::read(data.join("state.json")).unwrap());
    ok(&data, &["hub", "check"]);
}
