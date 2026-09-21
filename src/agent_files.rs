use crate::{
    agents::*,
    workspace::{fingerprint, git},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const DEFAULT_IGNORE: &str = ".git/\ntarget/\nnode_modules/\n.venv/\nvenv/\n__pycache__/\n.DS_Store\nThumbs.db\n.env\n.env.*\n*.pem\n*.key\n.tc-export.json\n";
const MAX_FILE: u64 = 90 * 1024 * 1024;
const MAX_IMPORT: usize = 512 * 1024 * 1024;

pub fn git_input(dir: &Path, args: &[&str], input: &[u8], allow_one: bool) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.excludesFile=/dev/null",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Drain output concurrently: large NUL-separated input/output must not deadlock pipes.
    let mut stdin = child.stdin.take().unwrap();
    let data = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&data));
    let out = child.wait_with_output()?;
    let written = writer
        .join()
        .map_err(|_| anyhow::anyhow!("Git 输入线程失败"))?;
    ensure!(
        out.status.success() || (allow_one && out.status.code() == Some(1)),
        "Git 操作失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    written?;
    Ok(String::from_utf8(out.stdout)?)
}
#[derive(Default)]
struct Scan {
    directories: BTreeSet<String>,
    repositories: BTreeMap<String, PathBuf>,
    files: BTreeMap<String, Vec<u8>>,
    skipped: Vec<String>,
}
fn rel_name(path: &Path) -> Result<String> {
    Ok(path.to_str().context("路径不是 UTF-8")?.replace('\\', "/"))
}
fn default_skip(name: &str, dir: bool) -> bool {
    (dir && ["target", "node_modules", ".venv", "venv", "__pycache__"].contains(&name))
        || [".DS_Store", "Thumbs.db", ".env", ".tc-export.json"].contains(&name)
        || name.starts_with(".env.")
        || name.ends_with(".key")
        || name.ends_with(".pem")
}
fn walk(
    source: &Path,
    relative: &Path,
    scan: &mut Scan,
    ignore_root: &Path,
    depth: usize,
    excludes: &[String],
) -> Result<()> {
    ensure!(
        depth <= 64
            && scan.files.len() + scan.directories.len() + scan.repositories.len() <= 100_000,
        "目录过深或条目超过 100000；请缩小登记范围"
    );
    let path = source.join(relative);
    let rel = rel_name(relative)?;
    if path.join(".git").exists() {
        git(&path, &["rev-parse", "--show-toplevel"])
            .context("发现 .git 标记但无法读取仓库；不会复制其内容")?;
        scan.repositories.insert(rel, path);
        return Ok(());
    }
    ensure!(
        !(path.join("HEAD").is_file()
            && path.join("objects").is_dir()
            && git(&path, &["rev-parse", "--is-bare-repository"]).is_ok_and(|v| v == "true")),
        "裸 Git 仓库不能当成普通资料导入：{}",
        path.display()
    );
    scan.directories.insert(rel.clone());
    let mut rules = String::new();
    for name in [".gitignore", ".tcignore"] {
        let p = path.join(name);
        if p.exists() && !is_link(&fs::symlink_metadata(&p)?) {
            rules.push_str(
                &fs::read_to_string(&p)
                    .with_context(|| format!("无法读取排除规则：{}", p.display()))?,
            );
            rules.push('\n');
        }
    }
    if relative.as_os_str().is_empty() {
        rules.push_str(DEFAULT_IGNORE);
        for rule in excludes {
            ensure!(!rule.contains(['\0', '\n', '\r']), "排除规则必须为一行");
            rules.push_str(rule);
            rules.push('\n');
        }
    }
    if !rules.is_empty() {
        fs::create_dir_all(ignore_root.join(relative))?;
        fs::write(ignore_root.join(relative).join(".gitignore"), rules)?;
    }
    let mut entries: Vec<_> = fs::read_dir(&path)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    let mut candidates = String::new();
    for entry in &entries {
        candidates.push_str(&rel_name(&relative.join(entry.file_name()))?);
        if entry.file_type()?.is_dir() {
            candidates.push('/');
        }
        candidates.push('\0');
    }
    let ignored = git_input(
        ignore_root,
        &["check-ignore", "--no-index", "-z", "--stdin"],
        candidates.as_bytes(),
        true,
    )?;
    let ignored: BTreeSet<_> = ignored
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_end_matches('/'))
        .collect();
    let mut names = BTreeSet::new();
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("文件名不是 UTF-8"))?;
        let child_rel = relative.join(&name);
        let child_name = rel_name(&child_rel)?;
        let meta = fs::symlink_metadata(entry.path())?;
        if is_link(&meta)
            || default_skip(&name, meta.is_dir())
            || ignored.contains(child_name.as_str())
        {
            scan.skipped.push(child_name);
            continue;
        }
        valid_name(&name)?;
        ensure!(
            names.insert(name.to_lowercase()),
            "同级文件大小写冲突：{}",
            entry.path().display()
        );
        if meta.is_dir() {
            walk(source, &child_rel, scan, ignore_root, depth + 1, excludes)?;
        } else {
            ensure!(meta.is_file(), "不支持特殊文件：{}", entry.path().display());
            // Read payloads only after ignore rules have been applied.
            scan.files.insert(child_name, Vec::new());
        }
    }
    Ok(())
}
fn scan(source: &Path, excludes: &[String]) -> Result<Scan> {
    ensure!(
        source.is_dir(),
        "来源目录不可访问；不会推断为删除：{}",
        source.display()
    );
    ensure!(
        !is_link(&fs::symlink_metadata(source)?),
        "来源不能为符号链接/目录联接"
    );
    let tmp = tempfile::tempdir()?;
    git(tmp.path(), &["init", "--initial-branch=main"])?;
    let mut scan = Scan::default();
    walk(source, Path::new(""), &mut scan, tmp.path(), 0, excludes)?;
    let mut rules = fs::read_to_string(tmp.path().join(".gitignore")).unwrap_or_default();
    rules.push('\n');
    rules.push_str(DEFAULT_IGNORE);
    for rule in excludes {
        ensure!(!rule.contains(['\0', '\n', '\r']), "排除规则必须为一行");
        rules.push_str(rule);
        rules.push('\n');
    }
    fs::write(tmp.path().join(".gitignore"), rules)?;
    let names: Vec<_> = scan
        .files
        .keys()
        .cloned()
        .chain(
            scan.repositories
                .keys()
                .chain(scan.directories.iter())
                .filter(|x| !x.is_empty())
                .map(|p| format!("{p}/")),
        )
        .collect();
    let input = names.iter().map(|x| format!("{x}\0")).collect::<String>();
    let ignored = git_input(
        tmp.path(),
        &["check-ignore", "--no-index", "-z", "--stdin"],
        input.as_bytes(),
        true,
    )?;
    let ignored: BTreeSet<_> = ignored
        .split('\0')
        .filter(|x| !x.is_empty())
        .map(|p| p.trim_end_matches('/').to_owned())
        .collect();
    let excluded = |path: &str| {
        ignored
            .iter()
            .any(|p| path == p || path.starts_with(&format!("{p}/")))
    };
    scan.files.retain(|p, _| !excluded(p));
    scan.repositories.retain(|p, _| !excluded(p));
    scan.directories.retain(|p| !excluded(p));
    scan.skipped.extend(ignored);
    let mut total = 0;
    for (rel, bytes) in &mut scan.files {
        let path = safe_path(source, Path::new(rel))?;
        ensure!(
            path.metadata()?.len() < MAX_FILE,
            "文件超过 90 MiB，请排除或单独管理：{}",
            path.display()
        );
        *bytes = fs::read(path)?;
        total += bytes.len();
        ensure!(total <= MAX_IMPORT, "单次收集超过 512 MiB，请分批登记");
    }
    Ok(scan)
}
#[derive(Serialize)]
pub struct ImportSummary {
    pub root: String,
    pub files: usize,
    pub bytes: usize,
    pub repositories: usize,
    pub skipped: Vec<String>,
    pub deleted: usize,
}
pub struct Collection {
    pub catalog: Catalog,
    pub changes: Changes,
    pub binding: SourceBinding,
    pub summary: ImportSummary,
}
fn parent_of(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(String::new(), |(p, _)| p.to_owned())
}
fn file_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}
pub fn content_path(node: &str, file: &str) -> PathBuf {
    PathBuf::from("content").join(node).join(file)
}

pub struct ImportOptions {
    pub agent: String,
    pub parent: Option<String>,
    pub name: String,
    pub excludes: Vec<String>,
}
pub fn import(
    cat: &Catalog,
    device: &Device,
    hub: &Path,
    source: &Path,
    options: ImportOptions,
) -> Result<Collection> {
    let ImportOptions {
        agent,
        parent,
        name,
        excludes,
    } = options;
    ensure!(
        !is_link(&fs::symlink_metadata(source)?),
        "来源不能为符号链接/目录联接"
    );
    let source = fs::canonicalize(source).context("来源目录不存在")?;
    let hub = fs::canonicalize(hub)?;
    ensure!(
        !source.starts_with(&hub) && !hub.starts_with(&source),
        "导入范围不能与 sync 仓库重叠"
    );
    let data = scan(&source, &excludes)?;
    let mut updated = cat.clone();
    device.register(&mut updated);
    if data.repositories.contains_key("") {
        let repo = register_repo(&mut updated, device, &source, None)?;
        let root = updated.add_node(&agent, parent, name, NodeKind::Repository, Some(repo))?;
        return Ok(Collection {
            changes: updated.changes(cat)?,
            catalog: updated,
            binding: SourceBinding::default(),
            summary: ImportSummary {
                root,
                files: 0,
                bytes: 0,
                repositories: 1,
                skipped: data.skipped,
                deleted: 0,
            },
        });
    }
    let root = updated.add_node(&agent, parent, name, NodeKind::Folder, None)?;
    let binding = SourceBinding {
        path: source,
        directories: BTreeMap::from([(String::new(), root.clone())]),
        baseline: BTreeMap::new(),
        excludes,
    };
    collect_scanned(cat, updated, device, &hub, binding, data, false)
}
pub fn collect(
    cat: &Catalog,
    device: &Device,
    hub: &Path,
    root: &str,
    prune: bool,
) -> Result<Collection> {
    let binding = device
        .sources
        .get(root)
        .context("本机无来源绑定；首次使用 agent import，换设备先 agent export")?
        .clone();
    ensure!(
        cat.nodes
            .get(root)
            .is_some_and(|n| n.kind == NodeKind::Folder),
        "来源对应的目录已被删除或不是文件夹"
    );
    let data = scan(&binding.path, &binding.excludes)?;
    ensure!(
        !data.repositories.contains_key(""),
        "来源现在是独立 Git 仓库；请显式改为仓库引用"
    );
    collect_scanned(cat, cat.clone(), device, hub, binding, data, prune)
}
fn collect_scanned(
    old: &Catalog,
    mut cat: Catalog,
    device: &Device,
    hub: &Path,
    mut binding: SourceBinding,
    data: Scan,
    prune: bool,
) -> Result<Collection> {
    let root = binding
        .directories
        .get("")
        .context("来源缺少根节点")?
        .clone();
    let agent = cat.nodes[&root].agent.clone();
    let mut changes = Changes::new();
    let mut deleted = 0;
    for rel in data.directories.iter().filter(|r| !r.is_empty()) {
        let parent = binding
            .directories
            .get(&parent_of(rel))
            .context("缺少来源父目录")?
            .clone();
        if let Some(id) = binding.directories.get(rel) {
            ensure!(
                cat.nodes
                    .get(id)
                    .is_some_and(|n| n.parent.as_ref() == Some(&parent)
                        && n.name == file_of(rel)
                        && n.kind == NodeKind::Folder),
                "来源与中央目录结构均有调整：{rel}；先导出新的目录再继续工作"
            );
        } else {
            let id = cat.add_node(
                &agent,
                Some(parent),
                file_of(rel).into(),
                NodeKind::Folder,
                None,
            )?;
            binding.directories.insert(rel.clone(), id);
        }
    }
    for (rel, path) in &data.repositories {
        let parent = binding
            .directories
            .get(&parent_of(rel))
            .context("仓库缺少父目录")?
            .clone();
        let repo = register_repo(&mut cat, device, path, None)?;
        if let Some(id) = binding.directories.get(rel) {
            ensure!(
                cat.nodes.get(id).is_some_and(
                    |n| n.kind == NodeKind::Repository && n.repository.as_ref() == Some(&repo)
                ),
                "来源的仓库边界有变化：{rel}；请显式整理节点"
            );
        } else {
            let id = cat.add_node(
                &agent,
                Some(parent),
                file_of(rel).into(),
                NodeKind::Repository,
                Some(repo),
            )?;
            binding.directories.insert(rel.clone(), id);
        }
    }
    for (rel, bytes) in &data.files {
        let node = binding
            .directories
            .get(&parent_of(rel))
            .context("文件缺少父目录")?;
        ensure!(cat.nodes.contains_key(node), "文件对应节点已删除：{rel}");
        let dest = content_path(node, file_of(rel));
        let current = bytes_hash(&safe_path(hub, &dest)?)?;
        let incoming = fingerprint(bytes);
        let base = binding.baseline.get(rel);
        if base != Some(&incoming) {
            ensure!(
                current.as_ref() == base || current.as_ref() == Some(&incoming),
                "文件双方都有修改：{rel}；中央和来源均保留，请手动合并"
            );
            changes.insert(dest, Some(bytes.clone()));
        } else if current.is_none() && is_tracked(hub, &dest)? {
            anyhow::bail!("正文未选择/下载：{rel}；请先 hub select");
        }
        binding.baseline.insert(rel.clone(), incoming);
    }
    if prune {
        for (rel, base) in binding.baseline.clone() {
            if data.files.contains_key(&rel)
                || data
                    .skipped
                    .iter()
                    .any(|p| rel == *p || rel.starts_with(&format!("{p}/")))
            {
                continue;
            }
            let node = binding
                .directories
                .get(&parent_of(&rel))
                .context("删除文件缺少目录")?;
            let dest = content_path(node, file_of(&rel));
            let current = bytes_hash(&safe_path(hub, &dest)?)?;
            ensure!(
                current.as_ref() == Some(&base),
                "来源删除与中央修改冲突：{rel}；不会删除正文"
            );
            changes.insert(dest, None);
            binding.baseline.remove(&rel);
            deleted += 1;
        }
    }
    cat.validate()?;
    changes.extend(cat.changes(old)?);
    let summary = ImportSummary {
        root,
        files: data.files.len(),
        bytes: data.files.values().map(Vec::len).sum(),
        repositories: data.repositories.len(),
        skipped: data.skipped,
        deleted,
    };
    Ok(Collection {
        catalog: cat,
        changes,
        binding,
        summary,
    })
}
pub fn is_tracked(hub: &Path, rel: &Path) -> Result<bool> {
    Ok(!git(hub, &["ls-files", "--", &rel_name(rel)?])?.is_empty())
}
pub fn files_for(hub: &Path, id: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = vec![];
    let rel = PathBuf::from("content").join(id);
    let dir = safe_path(hub, &rel)?;
    // Index knows tracked but sparsely absent files. Never turn those into deletions.
    let tracked = git(hub, &["ls-files", "-z", "--", &rel_name(&rel)?])?;
    for path in tracked.split('\0').filter(|s| !s.is_empty()) {
        ensure!(
            safe_path(hub, Path::new(path))?.exists(),
            "正文未下载或工作树已删除：{path}；请先 hub select / 查看 hub diff"
        );
    }
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("文件名不是 UTF-8"))?;
            valid_name(&name)?;
            let path = safe_path(hub, &rel.join(&name))?;
            ensure!(
                path.is_file(),
                "正文目录只放直属文件；子目录请用 agent folder/import 登记：{}",
                path.display()
            );
            files.push((name, fs::read(path)?));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}
pub fn check_content(hub: &Path, cat: &Catalog) -> Result<()> {
    let content = safe_path(hub, Path::new("content"))?;
    if !content.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(content)? {
        let entry = entry?;
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("正文 ID 不是 UTF-8"))?;
        valid_id(&id)?;
        let dir = safe_path(hub, &PathBuf::from("content").join(&id))?;
        ensure!(dir.is_dir(), "content 下只能放节点目录");
        // Git does not track empty directories left behind by a node removal.
        if fs::read_dir(&dir)?.next().is_none() {
            continue;
        }
        let node = cat
            .nodes
            .get(&id)
            .context("正文目录没有对应节点；请登记或移出 content")?;
        let child_names: BTreeSet<_> = cat
            .nodes
            .values()
            .filter(|n| n.parent.as_deref() == Some(&id))
            .map(|n| n.name.to_lowercase())
            .collect();
        let mut seen = BTreeSet::new();
        for file in fs::read_dir(&dir)? {
            let file = file?;
            let name = file
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("文件名不是 UTF-8"))?;
            valid_name(&name)?;
            let path = safe_path(hub, &PathBuf::from("content").join(&id).join(&name))?;
            ensure!(
                node.kind != NodeKind::Repository && path.is_file() && name != ".git",
                "代码仓库不能存正文；普通子目录必须登记节点：{}",
                path.display()
            );
            ensure!(
                file.metadata()?.len() < MAX_FILE,
                "正文超过 90 MiB：{}",
                path.display()
            );
            ensure!(
                seen.insert(name.to_lowercase()) && !child_names.contains(&name.to_lowercase()),
                "同级文件/目录名称冲突：{name}"
            );
        }
    }
    Ok(())
}
#[derive(Serialize)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub files: usize,
    pub repositories: Vec<serde_json::Value>,
}
pub fn export(cat: &Catalog, hub: &Path, agent: &str, to: &Path) -> Result<ExportSummary> {
    ensure!(!to.exists(), "导出只写新目录，不覆盖已有工作");
    let parent = to
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = tempfile::tempdir_in(parent)?;
    let mut count = 0;
    let mut repos = vec![];
    for n in cat.nodes.values().filter(|n| n.agent == agent) {
        let relative = cat.node_path(&n.id);
        if n.kind == NodeKind::Repository {
            repos.push(serde_json::json!({"path":relative,"node":n.id,"repository":cat.repos[n.repository.as_ref().unwrap()]}));
            continue;
        }
        let dest = safe_path(tmp.path(), &relative)?;
        fs::create_dir_all(&dest)?;
        for (name, bytes) in files_for(hub, &n.id)? {
            fs::write(safe_path(tmp.path(), &relative.join(name))?, bytes)?;
            count += 1;
        }
    }
    let marker = serde_json::json!({"version":1,"agent":cat.agents[agent],"repositories":repos,"note":"代码只记录入口；正文是导出副本。原工作区的 .git 历史不在此目录。"});
    ensure!(
        !tmp.path().join(".tc-export.json").exists(),
        "保留文件名冲突：.tc-export.json"
    );
    fs::write(
        tmp.path().join(".tc-export.json"),
        serde_json::to_vec_pretty(&marker)?,
    )?;
    fs::rename(tmp.path(), to)?;
    Ok(ExportSummary {
        path: to.to_owned(),
        files: count,
        repositories: repos,
    })
}
