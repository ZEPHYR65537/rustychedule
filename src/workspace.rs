use crate::{
    domain::State,
    store::{Store, atomic_write, decode_state},
};
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectMode {
    Reference,
    Snapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub mode: ProjectMode,
    pub remote: Option<String>,
    pub description: String,
    pub context: String,
    pub memory: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LocalSettings {
    pub hub: Option<PathBuf>,
    pub paths: BTreeMap<String, PathBuf>,
    pub base_state_hash: Option<String>,
}
impl LocalSettings {
    pub fn load(store: &Store) -> Result<Self> {
        let p = store.dir.join("local.json");
        if p.exists() {
            Ok(serde_json::from_slice(&fs::read(p)?)?)
        } else {
            Ok(Self::default())
        }
    }
    pub fn save(&self, store: &Store) -> Result<()> {
        atomic_write(
            &store.dir.join("local.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
    pub fn hub(&self) -> Result<&Path> {
        self.hub
            .as_deref()
            .context("请先用 hub init 或 hub clone 连接工作空间仓库")
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ProjectStatus {
    pub name: String,
    pub mode: ProjectMode,
    pub path: Option<PathBuf>,
    pub branch: String,
    pub head: String,
    pub changes: Vec<String>,
    pub error: Option<String>,
    pub tasks: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub commit: String,
    pub branch: String,
    pub bundle: String,
    pub bytes: u64,
    pub sha256: String,
}

pub fn slug(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "项目名称须为 1–64 个小写字母、数字、横线或下划线"
    );
    ensure!(
        ![
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"
        ]
        .contains(&s),
        "项目名称是系统保留名称"
    );
    Ok(())
}
pub fn portable_remote(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix("https://") {
        return !rest.split('/').next().unwrap_or("").is_empty()
            && !s.chars().any(char::is_whitespace)
            && !rest.split('/').next().unwrap_or("").contains('@')
            && !s.contains('?')
            && !s.contains('#');
    }
    s.starts_with("git@") && s.contains(':') && !s.chars().any(char::is_whitespace)
}
pub fn validate_projects(projects: &[Project]) -> Result<()> {
    let mut names = BTreeSet::new();
    for p in projects {
        slug(&p.name)?;
        ensure!(names.insert(&p.name), "项目重名");
        if let Some(r) = &p.remote {
            ensure!(
                portable_remote(r),
                "同步的仓库地址必须是无凭据的 HTTPS 或 git@ SSH 地址"
            );
        }
        ensure!(
            p.mode != ProjectMode::Reference || p.remote.is_some(),
            "引用模式需要原代码仓库地址"
        );
    }
    Ok(())
}

/// Argument vectors only: repository names, paths and URLs are never evaluated by a shell.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("未找到 Git，请先安装 Git 并配置账号认证")?;
    ensure!(
        out.status.success(),
        "Git 操作失败：{}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim_end_matches(['\r', '\n'])
        .into())
}
pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
// Git's clone URL parser treats Windows extended-length paths as remote URLs.
fn git_path(path: &Path) -> Result<String> {
    let path = path.to_str().context("Git 路径不是有效 UTF-8")?;
    #[cfg(windows)]
    {
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            return Ok(format!("//{}", unc.replace('\\', "/")));
        }
        Ok(path
            .strip_prefix(r"\\?\")
            .unwrap_or(path)
            .replace('\\', "/"))
    }
    #[cfg(not(windows))]
    Ok(path.into())
}
pub fn state_hash(state: &State) -> Result<String> {
    Ok(fingerprint(&serde_json::to_vec(state)?))
}
pub fn register(
    s: &mut State,
    local: &mut LocalSettings,
    name: String,
    path: &Path,
    mode: Option<ProjectMode>,
    remote: Option<String>,
    description: String,
) -> Result<()> {
    slug(&name)?;
    ensure!(
        s.projects.iter().all(|p| p.name != name),
        "项目已注册；换机器请用 project bind"
    );
    let root = git(path, &["rev-parse", "--show-toplevel"])?;
    let root = fs::canonicalize(root)?;
    let detected = git(&root, &["remote", "get-url", "origin"])
        .ok()
        .filter(|r| portable_remote(r));
    let remote = remote.or(detected);
    let mode = mode.unwrap_or(if remote.is_some() {
        ProjectMode::Reference
    } else {
        ProjectMode::Snapshot
    });
    let p = Project {
        name: name.clone(),
        mode,
        remote,
        description,
        context: String::new(),
        memory: String::new(),
    };
    validate_projects(std::slice::from_ref(&p))?;
    local.paths.insert(name, root);
    s.projects.push(p);
    Ok(())
}
pub fn statuses(s: &State, local: &LocalSettings) -> Vec<ProjectStatus> {
    s.projects
        .iter()
        .map(|p| {
            let path = local.paths.get(&p.name).cloned();
            let mut status = ProjectStatus {
                name: p.name.clone(),
                mode: p.mode,
                path: path.clone(),
                branch: String::new(),
                head: String::new(),
                changes: vec![],
                error: None,
                tasks: s
                    .tasks
                    .iter()
                    .filter(|t| t.project == p.name && t.status.active())
                    .count(),
            };
            if let Some(path) = path {
                let check = (|| -> Result<()> {
                    status.branch = git(&path, &["branch", "--show-current"])?;
                    status.head = git(&path, &["rev-parse", "--short", "HEAD"])?;
                    status.changes =
                        git(&path, &["-c", "core.quotepath=false", "status", "--short"])?
                            .lines()
                            .map(str::to_owned)
                            .collect();
                    Ok(())
                })();
                if let Err(e) = check {
                    status.error = Some(e.to_string());
                }
            } else {
                status.error = Some("本机未绑定；project bind 或 project checkout".into());
            }
            status
        })
        .collect()
}
pub fn init_hub(path: &Path, remote: Option<&str>) -> Result<PathBuf> {
    ensure!(
        !path.exists() || fs::read_dir(path)?.next().is_none(),
        "hub init 需要一个空目录"
    );
    fs::create_dir_all(path)?;
    let path = fs::canonicalize(path)?;
    git(&path, &["init", "--initial-branch=main"])?;
    atomic_write(
        &path.join(".schedule-hub.json"),
        b"{\"version\":1,\"application\":\"schedule\"}\n",
    )?;
    atomic_write(
        &path.join(".gitignore"),
        b"local.json\n.env\n.env.*\n*.key\n*.pem\ntarget/\n",
    )?;
    if let Some(r) = remote {
        git(&path, &["remote", "add", "origin", r])?;
    }
    Ok(path)
}
pub fn clone_hub(remote: &str, path: &Path) -> Result<PathBuf> {
    ensure!(!path.exists(), "目标已存在；请指定新目录");
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    git(
        parent,
        &[
            "clone",
            "--",
            remote,
            path.file_name()
                .and_then(|x| x.to_str())
                .context("目标目录名无效")?,
        ],
    )?;
    let path = fs::canonicalize(path)?;
    check_hub(&path)?;
    Ok(path)
}
pub fn check_hub(path: &Path) -> Result<()> {
    safe_path(path, Path::new(".schedule-hub.json"))?;
    let marker: serde_json::Value = serde_json::from_slice(
        &fs::read(path.join(".schedule-hub.json"))
            .context("不是 Schedule 工作空间仓库（缺少标识文件）")?,
    )?;
    ensure!(
        marker["version"] == 1 && marker["application"] == "schedule",
        "不支持的工作空间格式"
    );
    let root = fs::canonicalize(git(path, &["rev-parse", "--show-toplevel"])?)?;
    ensure!(
        root == fs::canonicalize(path)?,
        "hub 必须位于专用 Git 仓库根目录"
    );
    Ok(())
}
fn safe_path(hub: &Path, relative: &Path) -> Result<()> {
    let mut current = hub.to_path_buf();
    for component in relative.components() {
        ensure!(
            matches!(component, std::path::Component::Normal(_)),
            "工作空间路径不合法"
        );
        current.push(component);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            ensure!(
                !meta.file_type().is_symlink(),
                "工作空间受管路径不可为符号链接：{}",
                current.display()
            );
        }
    }
    Ok(())
}
pub fn snapshot(s: &State, local: &LocalSettings, name: &str) -> Result<Snapshot> {
    let p = s
        .projects
        .iter()
        .find(|p| p.name == name)
        .context("项目未注册")?;
    ensure!(
        p.mode == ProjectMode::Snapshot,
        "此项目是引用模式，代码保留在原仓库；无需上传副本"
    );
    let source = local.paths.get(name).context("本机未绑定项目")?;
    let hub = local.hub()?;
    check_hub(hub)?;
    ensure!(
        fs::canonicalize(source)? != fs::canonicalize(hub)?,
        "不能把工作空间本身注册为代码快照"
    );
    ensure!(
        git(source, &["status", "--porcelain"])?.is_empty(),
        "项目有未提交改动；请先在原项目审阅并提交，快照只包含已提交内容"
    );
    ensure!(
        !git(source, &["ls-files", "--stage"])?
            .lines()
            .any(|l| l.starts_with("160000 ")),
        "项目含子模块；请分别注册子模块或使用引用模式"
    );
    let attributes = git(source, &["ls-files"])?;
    ensure!(
        !attributes
            .lines()
            .any(|p| p.ends_with(".env") || p.ends_with(".pem") || p.ends_with(".key")),
        "检测到常见敏感文件名，请先审阅项目；快照包括提交历史"
    );
    // Only committed history of HEAD is bundled. Source branches/index/worktree stay untouched.
    let head = git(source, &["rev-parse", "HEAD"])?;
    let branch = git(source, &["branch", "--show-current"])?;
    let dir = hub.join("projects").join(name);
    safe_path(
        hub,
        &PathBuf::from("projects").join(name).join("project.bundle"),
    )?;
    safe_path(
        hub,
        &PathBuf::from("projects").join(name).join("snapshot.json"),
    )?;
    fs::create_dir_all(&dir)?;
    let temporary = tempfile::tempdir()?;
    let bundle = temporary.path().join("project.bundle");
    git(
        source,
        &[
            "bundle",
            "create",
            bundle.to_str().context("快照路径无效")?,
            "HEAD",
        ],
    )?;
    let bytes = fs::read(&bundle)?;
    ensure!(
        bytes.len() < 90 * 1024 * 1024,
        "Git 快照超过 90 MiB，请使用原代码仓库引用模式"
    );
    let relative = format!("projects/{name}/project.bundle");
    atomic_write(&hub.join(&relative), &bytes)?;
    let snapshot = Snapshot {
        commit: head,
        branch,
        bundle: relative,
        bytes: bytes.len() as u64,
        sha256: fingerprint(&bytes),
    };
    atomic_write(
        &dir.join("snapshot.json"),
        &serde_json::to_vec_pretty(&snapshot)?,
    )?;
    Ok(snapshot)
}
pub fn materialize(s: &State, hub: &Path) -> Result<()> {
    check_hub(hub)?;
    s.validate()?;
    for path in managed_paths(s, hub) {
        safe_path(hub, Path::new(&path))?;
    }
    atomic_write(
        &hub.join("workspace/state.json"),
        &serde_json::to_vec_pretty(s)?,
    )?;
    let mut index = String::from(
        "# Schedule 工作空间\n\n此仓库保存任务、模型额度、复盘和记忆。请使用 schedule CLI 修改并同步。\n本机路径、API 密钥和认证信息不在本仓库内。\n\n## 项目\n\n",
    );
    for p in &s.projects {
        let dir = hub.join("projects").join(&p.name);
        atomic_write(&dir.join("project.json"), &serde_json::to_vec_pretty(p)?)?;
        atomic_write(&dir.join("CONTEXT.md"), p.context.as_bytes())?;
        atomic_write(&dir.join("MEMORY.md"), p.memory.as_bytes())?;
        index.push_str(&format!(
            "- [{}](projects/{}/project.json) · {:?}\n",
            p.name, p.name, p.mode
        ));
    }
    index.push_str("\n## 任务\n\n");
    for t in &s.tasks {
        let dir = hub.join("tasks").join(t.id.to_string());
        atomic_write(&dir.join("task.json"), &serde_json::to_vec_pretty(t)?)?;
        let sessions: Vec<_> = s.sessions.iter().filter(|x| x.task == t.id).collect();
        atomic_write(
            &dir.join("sessions.json"),
            &serde_json::to_vec_pretty(&sessions)?,
        )?;
        atomic_write(&dir.join("CONTEXT.md"), t.note.as_bytes())?;
        index.push_str(&format!(
            "- [#{} {}](tasks/{}/task.json) · {} · {:.0}%\n",
            t.id,
            t.title,
            t.id,
            t.status.label(),
            t.progress
        ));
    }
    atomic_write(&hub.join("README.md"), index.as_bytes())?;
    Ok(())
}
fn managed_paths(s: &State, hub: &Path) -> Vec<String> {
    let mut paths = vec![
        ".schedule-hub.json".into(),
        ".gitignore".into(),
        "README.md".into(),
        "workspace/state.json".into(),
    ];
    for p in &s.projects {
        for file in ["project.json", "CONTEXT.md", "MEMORY.md"] {
            paths.push(format!("projects/{}/{file}", p.name));
        }
        if p.mode == ProjectMode::Snapshot {
            for file in ["snapshot.json", "project.bundle"] {
                let path = format!("projects/{}/{file}", p.name);
                if hub.join(&path).exists() {
                    paths.push(path);
                }
            }
        }
    }
    for t in &s.tasks {
        for file in ["task.json", "sessions.json", "CONTEXT.md"] {
            paths.push(format!("tasks/{}/{file}", t.id));
        }
    }
    paths
}
pub fn hub_remote(hub: &Path) -> Result<Option<String>> {
    Ok(git(hub, &["remote", "get-url", "origin"]).ok())
}
fn fetch(hub: &Path) -> Result<bool> {
    let refs = git(hub, &["ls-remote", "--heads", "origin", "main"])?;
    if refs.is_empty() {
        return Ok(false);
    }
    git(hub, &["fetch", "origin", "main"])?;
    Ok(true)
}
pub fn push(s: &State, local: &mut LocalSettings) -> Result<String> {
    let hub = local.hub()?.to_owned();
    check_hub(&hub)?;
    ensure!(
        hub_remote(&hub)?.is_some(),
        "尚未设置工作空间远程地址；使用 hub remote"
    );
    if fetch(&hub)? {
        let remote_state = git(&hub, &["show", "origin/main:workspace/state.json"])
            .context("远程不是有效的 Schedule 工作空间")?;
        let remote_state = decode_state(remote_state.as_bytes())?;
        let remote_hash = state_hash(&remote_state)?;
        ensure!(
            local.base_state_hash.as_ref() == Some(&remote_hash) || state_hash(s)? == remote_hash,
            "远端状态已更新或本机尚未拉取，请先 hub pull；不会覆盖另一台机器的数据"
        );
        ensure!(
            git(
                &hub,
                &["merge-base", "--is-ancestor", "origin/main", "HEAD"]
            )
            .is_ok(),
            "工作空间 Git 分支已分叉，请先处理分叉；不会强制推送"
        );
    }
    materialize(s, &hub)?;
    let paths = managed_paths(s, &hub);
    let staged = git(&hub, &["diff", "--cached", "--name-only"])?;
    ensure!(
        staged.lines().all(|p| paths.iter().any(|x| x == p)),
        "工作空间有不属于 CLI 的已暂存文件，请先处理；不会把它们一并上传"
    );
    for chunk in paths.chunks(100) {
        let mut args = vec!["add", "--"];
        args.extend(chunk.iter().map(String::as_str));
        git(&hub, &args)?;
    }
    if !git(&hub, &["diff", "--cached", "--name-only"])?.is_empty() {
        git(
            &hub,
            &[
                "-c",
                "user.name=Schedule",
                "-c",
                "user.email=schedule@localhost",
                "commit",
                "-m",
                "sync: update task workspace",
            ],
        )?;
    }
    git(&hub, &["push", "-u", "origin", "main"])?;
    local.base_state_hash = Some(state_hash(s)?);
    git(&hub, &["rev-parse", "HEAD"])
}
pub fn pull(s: &State, local: &mut LocalSettings, replace: bool) -> Result<State> {
    let hub = local.hub()?.to_owned();
    check_hub(&hub)?;
    ensure!(fetch(&hub)?, "远程还没有 main 分支，请先从主设备 hub push");
    let remote_text = git(&hub, &["show", "origin/main:workspace/state.json"])?;
    let remote_state = decode_state(remote_text.as_bytes())?;
    let current_hash = state_hash(s)?;
    let remote_hash = state_hash(&remote_state)?;
    let empty = current_hash == state_hash(&State::default())?;
    let local_clean = local.base_state_hash.as_ref() == Some(&current_hash);
    let remote_unchanged = local.base_state_hash.as_ref() == Some(&remote_hash);
    ensure!(
        replace || empty || local_clean || current_hash == remote_hash || remote_unchanged,
        "本机和远端都有变更，停止以避免丢失数据。先 export 备份；明确接受远端时用 hub pull --replace"
    );
    ensure!(
        git(&hub, &["status", "--porcelain"])?.is_empty(),
        "工作空间镜像有未提交改动（可能有未推送快照），请先处理；不会丢弃本地文件"
    );
    git(&hub, &["merge", "--ff-only", "origin/main"])?;
    local.base_state_hash = Some(remote_hash);
    if remote_unchanged && !replace && !local_clean {
        Ok(s.clone())
    } else {
        Ok(remote_state)
    }
}
pub fn checkout(s: &State, local: &mut LocalSettings, name: &str, to: &Path) -> Result<()> {
    let p = s
        .projects
        .iter()
        .find(|p| p.name == name)
        .context("项目未注册")?;
    ensure!(
        !to.exists(),
        "checkout 只允许写入新的目录，避免覆盖已有项目"
    );
    let parent = to
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let destination = to
        .file_name()
        .and_then(|x| x.to_str())
        .context("目标目录无效")?;
    match p.mode {
        ProjectMode::Reference => {
            git(
                parent,
                &[
                    "clone",
                    "--",
                    p.remote.as_deref().context("原仓库地址缺失")?,
                    destination,
                ],
            )?;
        }
        ProjectMode::Snapshot => {
            let hub = local.hub()?;
            check_hub(hub)?;
            safe_path(
                hub,
                &PathBuf::from("projects").join(name).join("snapshot.json"),
            )?;
            safe_path(
                hub,
                &PathBuf::from("projects").join(name).join("project.bundle"),
            )?;
            let snapshot: Snapshot = serde_json::from_slice(&fs::read(
                hub.join("projects").join(name).join("snapshot.json"),
            )?)?;
            let expected = format!("projects/{name}/project.bundle");
            ensure!(snapshot.bundle == expected, "快照路径不合法");
            let bundle = hub.join(expected);
            ensure!(
                fingerprint(&fs::read(&bundle)?) == snapshot.sha256,
                "快照内容校验失败"
            );
            git(parent, &["clone", "--", &git_path(&bundle)?, destination])?;
        }
    }
    local.paths.insert(name.into(), fs::canonicalize(to)?);
    Ok(())
}
