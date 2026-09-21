//! Hub format 2: editable agent documents plus a separately synchronized ledger.
use crate::{
    agent_files::{check_content, git_input},
    agents::*,
    domain::State,
    store::{atomic_write, decode_state},
    workspace::{self, LocalSettings, git},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub fn format(root: &Path) -> Result<u64> {
    let marker = safe_path(root, Path::new(".schedule-hub.json"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(marker).context("不是 schedule sync 仓库")?)?;
    ensure!(value["application"] == "schedule", "工作空间标识不匹配");
    let version = value["version"].as_u64().context("缺少工作空间版本")?;
    ensure!(version == 1 || version == 2, "不支持的 hub 格式 {version}");
    if version == 2 {
        valid_id(value["workspace_id"].as_str().context("缺少工作空间身份")?)?;
    }
    let actual = fs::canonicalize(git(root, &["rev-parse", "--show-toplevel"])?)?;
    ensure!(
        actual == fs::canonicalize(root)?,
        "hub 必须是专用 Git 仓库根目录"
    );
    Ok(version)
}
pub fn require_v2(root: &Path) -> Result<()> {
    ensure!(
        format(root)? == 2,
        "这是 v0.2 的旧 hub；先运行 hub migrate，审阅后 hub migrate --apply"
    );
    Ok(())
}
pub fn init(path: &Path, remote: Option<&str>, state: &State) -> Result<PathBuf> {
    let root = workspace::init_hub(path, remote)?;
    let marker = json!({"version":2,"application":"schedule","workspace_id":new_id("workspace")?});
    atomic_write(
        &root.join(".schedule-hub.json"),
        &serde_json::to_vec_pretty(&marker)?,
    )?;
    atomic_write(
        &root.join("planning/state.json"),
        &serde_json::to_vec_pretty(state)?,
    )?;
    atomic_write(&root.join("README.md"), b"# Schedule workspace\n\nAgent containers and their files are authoritative in catalog/ and content/.\nThe planning ledger is synchronized separately in planning/state.json.\nRepository references do not contain code. Local-only references include machine aliases and paths.\nUse tc agent tree, tc hub check, tc hub commit and tc hub push.\n")?;
    Ok(root)
}
/// Upgrade users who registered v0.2 projects but never connected a legacy hub.
/// New data is written only into the newly created, otherwise empty workspace.
pub fn seed_legacy(
    root: &Path,
    state: &State,
    local: &LocalSettings,
    device: &Device,
) -> Result<()> {
    if state.projects.is_empty() {
        return Ok(());
    }
    let lock = HubLock::open(root)?;
    let mut cat = check(root)?;
    ensure!(
        cat.agents.is_empty() && cat.repos.is_empty(),
        "旧登记只可导入新建的空工作空间"
    );
    device.register(&mut cat);
    let agent = cat.add_agent("v0.2 待整理资料".into(), None)?;
    let mut changes = Changes::from([(
        PathBuf::from("legacy/v02/workspace/state.json"),
        Some(serde_json::to_vec_pretty(state)?),
    )]);
    for project in &state.projects {
        let id = new_id("repo")?;
        let metadata = format!("legacy/v02/projects/{}/project.json", project.name);
        let locations = local
            .paths
            .get(&project.name)
            .map(|p| {
                path_string(p).map(|path| {
                    vec![Location {
                        machine: device.id.clone(),
                        path,
                    }]
                })
            })
            .transpose()?
            .unwrap_or_default();
        cat.repos.insert(
            id.clone(),
            Repository {
                id: id.clone(),
                name: project.name.clone(),
                remote: project.remote.clone(),
                locations,
                legacy: Some(metadata.clone()),
            },
        );
        changes.insert(
            PathBuf::from(metadata),
            Some(serde_json::to_vec_pretty(project)?),
        );
        let folder = cat.add_node(&agent, None, project.name.clone(), NodeKind::Folder, None)?;
        cat.add_node(
            &agent,
            Some(folder.clone()),
            "代码仓库".into(),
            NodeKind::Repository,
            Some(id),
        )?;
        for (file, body) in [
            ("CONTEXT.md", &project.context),
            ("MEMORY.md", &project.memory),
        ] {
            changes.insert(
                crate::agent_files::content_path(&folder, file),
                Some(body.as_bytes().to_vec()),
            );
        }
    }
    changes.extend(cat.changes(&Catalog::default())?);
    lock.apply(&changes)
}
pub fn clone(remote: &str, path: &Path, metadata_only: bool) -> Result<PathBuf> {
    ensure!(!path.exists(), "目标已经存在；请指定新目录");
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("目标目录无效")?;
    let mut args = vec!["clone"];
    if metadata_only {
        args.extend(["--filter=blob:none", "--sparse"]);
    }
    args.extend(["--", remote, name]);
    git(parent, &args)?;
    let root = fs::canonicalize(path)?;
    if metadata_only {
        git(
            &root,
            &["sparse-checkout", "set", "catalog", "planning", "workspace"],
        )?;
    }
    if format(&root)? == 2 {
        check(&root)?;
    } else {
        workspace::check_hub(&root)?;
    }
    Ok(root)
}
pub fn read_plan(root: &Path) -> Result<State> {
    decode_state(&fs::read(safe_path(
        root,
        Path::new("planning/state.json"),
    )?)?)
}
pub fn check(root: &Path) -> Result<Catalog> {
    require_v2(root)?;
    ensure!(
        git(root, &["ls-files", "--unmerged"])?.is_empty(),
        "Git 合并尚有冲突；请解决后再校验"
    );
    let cat = Catalog::load(root)?;
    let dir = safe_path(root, Path::new("catalog"))?;
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            ensure!(
                ["agents", "nodes", "repos", "machines"]
                    .iter()
                    .any(|name| entry.file_name() == *name),
                "catalog 中存在未知目录"
            );
        }
    }
    read_plan(root)?;
    let deleted = git(root, &["diff", "--name-only", "--diff-filter=D", "-z"])?;
    let deleted: BTreeSet<_> = deleted.split('\0').filter(|p| !p.is_empty()).collect();
    let mut content_names = BTreeSet::new();
    for node in cat.nodes.values() {
        if let Some(parent) = &node.parent {
            content_names.insert((parent.clone(), node.name.to_lowercase()));
        }
    }
    for entry in git(root, &["ls-files", "--stage", "-z"])?
        .split('\0')
        .filter(|s| !s.is_empty())
    {
        let (fields, path) = entry.split_once('\t').context("无法解析 Git 索引")?;
        if !managed(path) {
            continue;
        }
        if deleted.contains(path) {
            continue;
        }
        let mode = fields.split_whitespace().next().unwrap_or_default();
        ensure!(
            mode == "100644" || mode == "100755",
            "受管文件不能为 Git 子模块或符号链接：{path}"
        );
        for part in path.split('/') {
            valid_name(part)?;
        }
        if path.starts_with("content/") {
            ensure!(
                path.split('/').count() == 3,
                "正文子目录必须登记节点：{path}"
            );
            let mut parts = path.split('/');
            parts.next();
            let id = parts.next().unwrap();
            let name = parts.next().unwrap();
            let node = cat
                .nodes
                .get(id)
                .context("Git 中的正文缺少节点登记（可能尚未检出正文）")?;
            ensure!(
                node.kind != NodeKind::Repository,
                "代码仓库索引不能携带正文：{path}"
            );
            ensure!(
                content_names.insert((id.to_owned(), name.to_lowercase())),
                "Git 中同级文件/文件夹名称冲突：{path}"
            );
        }
    }
    check_content(root, &cat)?;
    Ok(cat)
}
fn managed(path: &str) -> bool {
    [".schedule-hub.json", ".gitignore", "README.md"].contains(&path)
        || ["catalog/", "content/", "planning/", "legacy/"]
            .iter()
            .any(|p| path.starts_with(p))
}
pub fn commit(root: &Path, message: &str) -> Result<String> {
    check(root)?;
    let staged = git(root, &["diff", "--cached", "--name-only", "-z"])?;
    ensure!(
        staged.split('\0').filter(|s| !s.is_empty()).all(managed),
        "存在非托管的已暂存文件；不会一并提交，请先处理暂存区"
    );
    let tracked = git(root, &["ls-files", "-z"])?;
    for part in [
        ".schedule-hub.json",
        ".gitignore",
        "README.md",
        "catalog",
        "content",
        "planning",
        "legacy",
    ] {
        if root.join(part).exists()
            || tracked
                .split('\0')
                .any(|p| p == part || p.starts_with(&format!("{part}/")))
        {
            git(root, &["add", "--all", "--force", "--sparse", "--", part])?;
        }
    }
    if !git(root, &["diff", "--cached", "--name-only"])?.is_empty() {
        git(
            root,
            &[
                "-c",
                "user.name=Schedule",
                "-c",
                "user.email=schedule@localhost",
                "commit",
                "-m",
                message,
            ],
        )?;
    }
    git(root, &["rev-parse", "HEAD"]).context("工作空间还没有提交")
}
fn current_branch(root: &Path) -> Result<String> {
    let branch = git(root, &["branch", "--show-current"])?;
    ensure!(
        !branch.is_empty(),
        "当前为 detached HEAD；请先切换到工作分支"
    );
    Ok(branch)
}
fn fetch(root: &Path, branch: &str) -> Result<bool> {
    ensure!(
        workspace::hub_remote(root)?.is_some(),
        "请先 hub remote 配置同步仓库地址"
    );
    if git(root, &["ls-remote", "--heads", "origin", branch])?.is_empty() {
        return Ok(false);
    }
    git(
        root,
        &[
            "fetch",
            "origin",
            &format!("refs/heads/{branch}:refs/remotes/origin/{branch}"),
        ],
    )?;
    Ok(true)
}
fn hash(s: &State) -> Result<String> {
    workspace::state_hash(s)
}
pub fn push(state: &State, local: &mut LocalSettings, agents_only: bool) -> Result<String> {
    let root = local.hub()?.to_owned();
    let lock = HubLock::open(&root)?;
    check(&root)?;
    let branch = current_branch(&root)?;
    if fetch(&root, &branch)? {
        ensure!(
            git(
                &root,
                &[
                    "merge-base",
                    "--is-ancestor",
                    &format!("origin/{branch}"),
                    "HEAD"
                ]
            )
            .is_ok(),
            "远端已更新；先 hub pull。不会强制推送"
        );
    }
    if !agents_only {
        let previous = read_plan(&root)?;
        let current_hash = hash(state)?;
        let hub_hash = hash(&previous)?;
        ensure!(
            hub_hash == current_hash || local.base_state_hash.as_ref() == Some(&hub_hash),
            "中央规划副本已有未导入变更；先 hub pull 或显式 import 规划文件，不能覆盖"
        );
        let changes = Changes::from([(
            PathBuf::from("planning/state.json"),
            Some(serde_json::to_vec_pretty(state)?),
        )]);
        lock.apply(&changes)?;
    }
    let head = commit(&root, "sync: update agent workspace")?;
    git(
        &root,
        &["push", "-u", "origin", &format!("HEAD:refs/heads/{branch}")],
    )?;
    if !agents_only {
        local.base_state_hash = Some(hash(state)?);
    }
    Ok(head)
}
fn plan_at(root: &Path, reference: &str) -> Result<State> {
    decode_state(git(root, &["show", &format!("{reference}:planning/state.json")])?.as_bytes())
}
fn import_plan(
    current: &State,
    result: &State,
    base: &Option<String>,
    replace: bool,
) -> Result<State> {
    let current_hash = hash(current)?;
    let incoming_hash = hash(result)?;
    let empty = current_hash == hash(&State::default())?;
    ensure!(
        replace
            || empty
            || base.as_ref() == Some(&current_hash)
            || current_hash == incoming_hash
            || base.as_ref() == Some(&incoming_hash),
        "本机与中央规划账本都已改变；可先 hub pull --agents-only 只同步项目。完整替换须先 export 备份，再 hub pull --replace"
    );
    if !replace && base.as_ref() == Some(&incoming_hash) && !empty {
        Ok(current.clone())
    } else {
        Ok(result.clone())
    }
}
pub fn pull(
    state: &State,
    local: &mut LocalSettings,
    agents_only: bool,
    replace: bool,
    device: &Device,
) -> Result<State> {
    let root = local.hub()?.to_owned();
    let _lock = HubLock::open(&root)?;
    check(&root)?;
    ensure!(
        git(&root, &["status", "--porcelain"])?.is_empty(),
        "工作空间有未提交内容；先 hub diff / hub commit，再拉取"
    );
    let branch = current_branch(&root)?;
    ensure!(
        fetch(&root, &branch)?,
        "远端没有此分支；请先从原设备 hub push"
    );
    let reference = format!("origin/{branch}");
    let head = git(&root, &["rev-parse", "HEAD"])?;
    let ancestor = git(&root, &["merge-base", "HEAD", &reference])
        .context("两个工作空间没有共同历史，不会自动拼接")?;
    let base = plan_at(&root, &ancestor)?;
    let left = plan_at(&root, "HEAD")?;
    let right = plan_at(&root, &reference)?;
    ensure!(
        hash(&left)? == hash(&base)?
            || hash(&right)? == hash(&base)?
            || hash(&left)? == hash(&right)?,
        "两个 Git 分支分别修改了规划账本；不能文本合并额度，请在分支中明确保留哪份账本后再合并"
    );
    let ahead = git(&root, &["merge-base", "--is-ancestor", &reference, "HEAD"]).is_ok();
    let incoming = if ahead {
        read_plan(&root)?
    } else {
        let temp = tempfile::tempdir()?;
        let worktree = temp.path().join("verify");
        let worktree_arg = path_string(&worktree)?;
        git(
            &root,
            &[
                "worktree",
                "add",
                "--detach",
                "--no-checkout",
                &worktree_arg,
                "HEAD",
            ],
        )?;
        let result = (|| -> Result<State> {
            // Validate complete metadata without eagerly downloading every body in a partial clone.
            git(
                &worktree,
                &["sparse-checkout", "set", "--cone", "catalog", "planning"],
            )?;
            git(&worktree, &["read-tree", "--reset", "-u", "HEAD"])?;
            git(
                &worktree,
                &[
                    "-c",
                    "user.name=Schedule",
                    "-c",
                    "user.email=schedule@localhost",
                    "merge",
                    "--no-commit",
                    "--no-ff",
                    &reference,
                ],
            )
            .context(
                "Git 合并存在冲突；原工作树未修改。可在 hub 内 git merge 后手工解决，再 hub check",
            )?;
            check(&worktree)?;
            read_plan(&worktree)
        })();
        let cleanup = git(&root, &["worktree", "remove", "--force", &worktree_arg]);
        cleanup.context("临时校验工作树清理失败")?;
        result?
    };
    let next = if agents_only {
        state.clone()
    } else {
        import_plan(state, &incoming, &local.base_state_hash, replace)?
    };
    if !ahead {
        ensure!(
            git(&root, &["rev-parse", "HEAD"])? == head,
            "校验期间分支被外部修改，请重试"
        );
        git(
            &root,
            &[
                "-c",
                "user.name=Schedule",
                "-c",
                "user.email=schedule@localhost",
                "merge",
                "--no-edit",
                &reference,
            ],
        )?;
    }
    if !agents_only {
        local.base_state_hash = Some(hash(&incoming)?);
    }
    if let Some(selection) = &device.selection {
        select(&root, &Catalog::load(&root)?, Some(selection))?;
    }
    Ok(next)
}
pub fn select(root: &Path, cat: &Catalog, selected: Option<&[String]>) -> Result<()> {
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "选择正文前先提交工作空间改动；不会移除未提交文件"
    );
    if let Some(selected) = selected {
        let mut agents = BTreeSet::new();
        let mut nodes = BTreeSet::new();
        for id in selected {
            if cat.nodes.contains_key(id) {
                nodes.extend(cat.descendants(id));
                continue;
            }
            // A selected folder may have been explicitly deleted on another device.
            // Retaining its local selection must not prevent pulling that deletion.
            if !cat.agents.contains_key(id) {
                continue;
            }
            let mut id = id.as_str();
            let mut seen = BTreeSet::new();
            while let Some(a) = cat.agents.get(id) {
                ensure!(seen.insert(id), "合并映射循环");
                if let Some(next) = &a.merged_into {
                    id = next;
                } else {
                    break;
                }
            }
            ensure!(cat.agents.contains_key(id), "选择的项目不存在：{id}");
            agents.insert(id);
        }
        let mut paths = vec!["catalog".to_owned(), "planning".to_owned()];
        paths.extend(
            cat.nodes
                .values()
                .filter(|n| agents.contains(n.agent.as_str()) || nodes.contains(&n.id))
                .map(|n| format!("content/{}", n.id)),
        );
        git_input(
            root,
            &["sparse-checkout", "set", "--cone", "--stdin"],
            paths.join("\n").as_bytes(),
            false,
        )?;
    } else {
        git(root, &["sparse-checkout", "disable"])?;
    }
    Ok(())
}

/// A second v0.2 device follows the existing central migration instead of creating
/// its own random identities. Ordinary old-format pulls still use the legacy path.
pub fn pull_upgrade(
    state: &State,
    local: &mut LocalSettings,
    agents_only: bool,
    replace: bool,
) -> Result<Option<State>> {
    let root = local.hub()?.to_owned();
    let _lock = HubLock::open(&root)?;
    let branch = current_branch(&root)?;
    if !fetch(&root, &branch)? {
        return Ok(None);
    }
    let reference = format!("origin/{branch}");
    let marker: serde_json::Value = serde_json::from_str(&git(
        &root,
        &["show", &format!("{reference}:.schedule-hub.json")],
    )?)?;
    if marker["version"] != 2 {
        return Ok(None);
    }
    ensure!(marker["application"] == "schedule", "远端工作空间标识无效");
    ensure!(
        git(&root, &["status", "--porcelain"])?.is_empty(),
        "升级拉取前请提交旧工作空间改动"
    );
    ensure!(
        git(&root, &["merge-base", "--is-ancestor", "HEAD", &reference]).is_ok(),
        "旧工作空间与中央迁移分叉；请保留此副本并在新目录 hub clone，避免重复生成身份"
    );
    let incoming = plan_at(&root, &reference)?;
    let next = if agents_only {
        state.clone()
    } else {
        import_plan(state, &incoming, &local.base_state_hash, replace)?
    };
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("verify");
    let path_arg = path_string(&path)?;
    git(
        &root,
        &[
            "worktree",
            "add",
            "--detach",
            "--no-checkout",
            &path_arg,
            &reference,
        ],
    )?;
    let result = (|| -> Result<()> {
        git(
            &path,
            &["sparse-checkout", "set", "--cone", "catalog", "planning"],
        )?;
        git(&path, &["read-tree", "--reset", "-u", "HEAD"])?;
        check(&path)?;
        Ok(())
    })();
    git(&root, &["worktree", "remove", "--force", &path_arg])?;
    result?;
    git(&root, &["merge", "--ff-only", &reference])?;
    if !agents_only {
        local.base_state_hash = Some(hash(&incoming)?);
    }
    Ok(Some(next))
}
pub fn migrate(
    root: &Path,
    state: &State,
    local: &LocalSettings,
    device: &Device,
    apply: bool,
) -> Result<serde_json::Value> {
    if format(root)? == 2 {
        return Ok(json!({"already_migrated":true}));
    }
    let lock = HubLock::open(root)?;
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "迁移前请提交旧 hub 中的改动；原文件将保留在 legacy/v02"
    );
    let synced = decode_state(&fs::read(root.join("workspace/state.json"))?)?;
    ensure!(
        hash(&synced)? == hash(state)?,
        "本机规划与旧 hub 不一致；先用旧 hub push/pull 对齐再迁移"
    );
    let mut cat = Catalog::default();
    device.register(&mut cat);
    let mut changes = Changes::new();
    let tracked = git(root, &["ls-files", "-z"])?;
    for path in tracked.split('\0').filter(|p| !p.is_empty()) {
        let old = safe_path(root, Path::new(path))?;
        changes.insert(PathBuf::from("legacy/v02").join(path), Some(fs::read(old)?));
        if ![".gitignore", "README.md", ".schedule-hub.json"].contains(&path) {
            changes.insert(PathBuf::from(path), None);
        }
    }
    let container = if state.projects.is_empty() {
        None
    } else {
        Some(cat.add_agent("v0.2 待整理资料".into(), None)?)
    };
    for p in &state.projects {
        let repo_id = new_id("repo")?;
        let locations = local
            .paths
            .get(&p.name)
            .map(|path| {
                path_string(path).map(|path| {
                    vec![Location {
                        machine: device.id.clone(),
                        path,
                    }]
                })
            })
            .transpose()?
            .unwrap_or_default();
        let bundle = format!("legacy/v02/projects/{}/project.bundle", p.name);
        let legacy = if root
            .join(format!("projects/{}/project.bundle", p.name))
            .exists()
        {
            bundle
        } else {
            format!("legacy/v02/projects/{}/project.json", p.name)
        };
        cat.repos.insert(
            repo_id.clone(),
            Repository {
                id: repo_id.clone(),
                name: p.name.clone(),
                remote: p.remote.clone(),
                locations,
                legacy: Some(legacy),
            },
        );
        let agent = container.as_ref().unwrap();
        let folder = cat.add_node(agent, None, p.name.clone(), NodeKind::Folder, None)?;
        cat.add_node(
            agent,
            Some(folder.clone()),
            "代码仓库".into(),
            NodeKind::Repository,
            Some(repo_id),
        )?;
        for (file, fallback) in [("CONTEXT.md", &p.context), ("MEMORY.md", &p.memory)] {
            let bytes = fs::read(root.join("projects").join(&p.name).join(file))
                .unwrap_or_else(|_| fallback.as_bytes().to_vec());
            changes.insert(crate::agent_files::content_path(&folder, file), Some(bytes));
        }
    }
    changes.extend(cat.changes(&Catalog::default())?);
    changes.insert(
        PathBuf::from("planning/state.json"),
        Some(serde_json::to_vec_pretty(state)?),
    );
    changes.insert(
        PathBuf::from(".schedule-hub.json"),
        Some(serde_json::to_vec_pretty(
            &json!({"version":2,"application":"schedule","workspace_id":new_id("workspace")?}),
        )?),
    );
    changes.insert(PathBuf::from("README.md"),Some("# Schedule v0.3 工作空间\n\n旧版文件原样保存在 legacy/v02。catalog 与 content 是可直接编辑的 agent 工作空间；planning 独立保存逻辑任务与用量。\n".as_bytes().to_vec()));
    let result = json!({"applied":apply,"repositories":cat.repos.len(),"agents":cat.agents.len(),"backup":"legacy/v02","planning_unchanged":true});
    if apply {
        lock.apply(&changes)?;
        check(root)?;
    }
    Ok(result)
}
