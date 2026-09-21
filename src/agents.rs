//! Git-native agent containers. These records never mutate the planning ledger.
use crate::{
    store::{Store, atomic_write},
    workspace::{fingerprint, git, portable_remote},
};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub fn new_id(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("生成身份失败：{e}"))?;
    Ok(format!(
        "{prefix}-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
pub fn valid_id(id: &str) -> Result<()> {
    ensure!(
        id.len() >= 3
            && id.len() <= 80
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "无效 ID：{id}"
    );
    Ok(())
}
/// Portable single path component, even when validating Windows paths on Unix.
pub fn valid_name(name: &str) -> Result<()> {
    ensure!(
        !name.eq_ignore_ascii_case(".git"),
        "Git 元数据名称不能用作托管文件或节点"
    );
    ensure!(
        !name.is_empty()
            && name.len() <= 240
            && name != "."
            && name != ".."
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
        "名称不能跨目录或包含不可携带字符：{name:?}"
    );
    let stem = name.split('.').next().unwrap().to_ascii_lowercase();
    ensure!(
        ![
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"
        ]
        .contains(&stem.as_str()),
        "系统保留名称：{name}"
    );
    Ok(())
}
pub fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
pub fn safe_path(root: &Path, rel: &Path) -> Result<PathBuf> {
    let mut path = root.to_owned();
    for part in rel.components() {
        let std::path::Component::Normal(name) = part else {
            bail!("不允许越界路径：{}", rel.display())
        };
        valid_name(name.to_str().context("路径不是 UTF-8")?)?;
        path.push(name);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            ensure!(!is_link(&meta), "不跨符号链接/目录联接：{}", path.display());
        }
    }
    Ok(path)
}
pub fn path_string(path: &Path) -> Result<String> {
    let s = path.to_str().context("路径不是 UTF-8")?;
    #[cfg(windows)]
    {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return Ok(format!("//{}", rest.replace('\\', "/")));
        }
        Ok(s.strip_prefix(r"\\?\").unwrap_or(s).replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        Ok(s.to_owned())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub tasks: BTreeSet<u64>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub merged_into: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub provider: String,
    pub source: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Folder,
    Repository,
    Conversation,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub agent: String,
    pub parent: Option<String>,
    pub name: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub origin_agent: Option<String>,
    #[serde(default)]
    pub source: Option<Binding>,
    #[serde(default)]
    pub tasks: BTreeSet<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub id: String,
    pub alias: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub machine: String,
    pub path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub remote: Option<String>,
    #[serde(default)]
    pub locations: Vec<Location>,
    #[serde(default)]
    pub legacy: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub agents: BTreeMap<String, Agent>,
    pub nodes: BTreeMap<String, Node>,
    pub repos: BTreeMap<String, Repository>,
    pub machines: BTreeMap<String, Machine>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceBinding {
    pub path: PathBuf,
    pub directories: BTreeMap<String, String>,
    pub baseline: BTreeMap<String, String>,
    #[serde(default)]
    pub excludes: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceBinding>,
    /// None = all content; Some(empty) = metadata only.
    #[serde(default)]
    pub selection: Option<Vec<String>>,
}
impl Device {
    pub fn load(store: &Store) -> Result<Self> {
        let path = store.dir.join("device.json");
        if path.exists() {
            let d: Self = serde_json::from_slice(&fs::read(path)?)?;
            valid_id(&d.id)?;
            Ok(d)
        } else {
            Ok(Self {
                id: new_id("machine")?,
                ..Self::default()
            })
        }
    }
    pub fn save(&self, store: &Store) -> Result<()> {
        atomic_write(
            &store.dir.join("device.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
    pub fn register(&self, cat: &mut Catalog) {
        cat.machines
            .entry(self.id.clone())
            .or_insert_with(|| Machine {
                id: self.id.clone(),
                alias: self
                    .alias
                    .clone()
                    .unwrap_or_else(|| format!("device-{}", &self.id[self.id.len() - 8..])),
            });
    }
}
fn read_records<T: for<'de> Deserialize<'de>>(
    root: &Path,
    dir: &str,
) -> Result<BTreeMap<String, T>> {
    let path = safe_path(root, Path::new(dir))?;
    let mut records = BTreeMap::new();
    if !path.exists() {
        return Ok(records);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        ensure!(
            name.ends_with(".json"),
            "目录说明中存在未知文件：{dir}/{name}"
        );
        let key = name.trim_end_matches(".json").to_owned();
        valid_id(&key)?;
        let p = safe_path(root, &Path::new(dir).join(&name))?;
        ensure!(p.is_file(), "目录说明必须是文件");
        let value = serde_json::from_slice(&fs::read(&p)?)
            .with_context(|| format!("无效目录说明：{}", p.display()))?;
        records.insert(key, value);
    }
    Ok(records)
}
impl Catalog {
    pub fn load(root: &Path) -> Result<Self> {
        let cat = Self {
            agents: read_records(root, "catalog/agents")?,
            nodes: read_records(root, "catalog/nodes")?,
            repos: read_records(root, "catalog/repos")?,
            machines: read_records(root, "catalog/machines")?,
        };
        cat.validate()?;
        Ok(cat)
    }
    pub fn validate(&self) -> Result<()> {
        let mut names = BTreeSet::new();
        for (id, a) in &self.agents {
            valid_id(id)?;
            ensure!(id == &a.id, "agent 文件名与 ID 不一致");
            valid_name(&a.name)?;
            ensure!(
                a.archived || names.insert(a.name.to_lowercase()),
                "活动 agent 项目重名：{}",
                a.name
            );
            if let Some(target) = &a.merged_into {
                ensure!(
                    a.archived && target != id && self.agents.contains_key(target),
                    "无效合并去向：{id}"
                );
                let mut seen = BTreeSet::from([id]);
                let mut at = target;
                loop {
                    ensure!(seen.insert(at), "合并去向存在循环");
                    if let Some(next) = &self.agents[at].merged_into {
                        at = next;
                    } else {
                        break;
                    }
                }
            }
        }
        names.clear();
        for (id, m) in &self.machines {
            valid_id(id)?;
            valid_name(&m.alias)?;
            ensure!(
                id == &m.id && names.insert(m.alias.to_lowercase()),
                "机器身份或别名冲突：{}",
                m.alias
            );
        }
        for (id, r) in &self.repos {
            valid_id(id)?;
            valid_name(&r.name)?;
            ensure!(id == &r.id, "仓库 ID 不一致");
            ensure!(
                r.remote.as_ref().is_none_or(|r| portable_remote(r)),
                "仓库 remote 必须是无凭据的 HTTPS 或 git@ 地址"
            );
            ensure!(
                r.remote.is_some() || !r.locations.is_empty() || r.legacy.is_some(),
                "本地仓库缺少机器位置：{}",
                r.name
            );
            let mut positions = BTreeSet::new();
            for loc in &r.locations {
                ensure!(
                    self.machines.contains_key(&loc.machine),
                    "仓库引用了未知机器"
                );
                ensure!(
                    absolute_location(&loc.path) && positions.insert((&loc.machine, &loc.path)),
                    "无效或重复机器路径"
                );
            }
        }
        let mut siblings = BTreeSet::new();
        for (id, n) in &self.nodes {
            valid_id(id)?;
            ensure!(
                id == &n.id && self.agents.contains_key(&n.agent),
                "节点 ID 或项目引用无效：{id}"
            );
            valid_name(&n.name)?;
            ensure!(
                siblings.insert((&n.agent, &n.parent, n.name.to_lowercase())),
                "同级名称冲突：{}",
                n.name
            );
            ensure!(
                n.kind != NodeKind::Repository
                    || n.repository
                        .as_ref()
                        .is_some_and(|r| self.repos.contains_key(r)),
                "仓库节点缺少有效引用"
            );
            ensure!(
                n.kind == NodeKind::Repository || n.repository.is_none(),
                "普通节点不能携带仓库引用"
            );
            if let Some(origin) = &n.origin_agent {
                ensure!(self.agents.contains_key(origin), "节点缺少来源项目");
            }
            let mut seen = BTreeSet::from([id]);
            let mut parent = &n.parent;
            while let Some(p) = parent {
                ensure!(seen.len() <= 128, "文件夹层级超过 128");
                ensure!(seen.insert(p), "文件夹包含循环：{id}");
                let p = self.nodes.get(p).context("父节点不存在")?;
                ensure!(
                    p.agent == n.agent && p.kind == NodeKind::Folder,
                    "父节点必须为同一 agent 项目的文件夹"
                );
                parent = &p.parent;
            }
        }
        Ok(())
    }
    pub fn agent_id(&self, query: &str) -> Result<String> {
        resolve(query, self.agents.values().map(|v| (&v.id, &v.name)))
    }
    pub fn node_id(&self, query: &str) -> Result<String> {
        resolve(query, self.nodes.values().map(|v| (&v.id, &v.name)))
    }
    pub fn repo_id(&self, query: &str) -> Result<String> {
        resolve(query, self.repos.values().map(|v| (&v.id, &v.name)))
    }
    pub fn add_agent(&mut self, name: String, binding: Option<Binding>) -> Result<String> {
        valid_name(&name)?;
        let id = new_id("agent")?;
        self.agents.insert(
            id.clone(),
            Agent {
                id: id.clone(),
                name,
                description: String::new(),
                bindings: binding.into_iter().collect(),
                tasks: BTreeSet::new(),
                archived: false,
                merged_into: None,
            },
        );
        self.validate()?;
        Ok(id)
    }
    pub fn add_node(
        &mut self,
        agent: &str,
        parent: Option<String>,
        name: String,
        kind: NodeKind,
        repo: Option<String>,
    ) -> Result<String> {
        ensure!(
            !self.agents.get(agent).context("agent 项目不存在")?.archived,
            "请先恢复归档项目"
        );
        let id = new_id("node")?;
        self.nodes.insert(
            id.clone(),
            Node {
                id: id.clone(),
                agent: agent.into(),
                parent,
                name,
                kind,
                repository: repo,
                origin_agent: Some(agent.into()),
                source: None,
                tasks: BTreeSet::new(),
            },
        );
        self.validate()?;
        Ok(id)
    }
    pub fn descendants(&self, root: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::from([root.to_owned()]);
        loop {
            let before = found.len();
            for n in self.nodes.values() {
                if n.parent.as_ref().is_some_and(|p| found.contains(p)) {
                    found.insert(n.id.clone());
                }
            }
            if found.len() == before {
                break;
            }
        }
        found
    }
    pub fn move_node(&mut self, node: &str, agent: &str, parent: Option<String>) -> Result<()> {
        ensure!(
            !self.agents.get(agent).context("项目不存在")?.archived,
            "目标项目已归档"
        );
        ensure!(self.nodes.contains_key(node), "节点不存在");
        let subtree = self.descendants(node);
        ensure!(
            parent.as_ref().is_none_or(|p| !subtree.contains(p)),
            "不能把目录移入自身"
        );
        self.nodes.get_mut(node).unwrap().parent = parent;
        for id in subtree {
            self.nodes.get_mut(&id).unwrap().agent = agent.into();
        }
        self.validate()
    }
    pub fn merge(&mut self, from: &str, to: &str) -> Result<String> {
        ensure!(
            from != to && !self.agents[from].archived,
            "不能合并自身或归档项目"
        );
        let name = self.agents[from].name.clone();
        let group = self.add_node(to, None, name, NodeKind::Folder, None)?;
        self.nodes.get_mut(&group).unwrap().origin_agent = Some(from.into());
        let roots: Vec<_> = self
            .nodes
            .values()
            .filter(|n| n.agent == from && n.parent.is_none())
            .map(|n| n.id.clone())
            .collect();
        for root in roots {
            self.move_node(&root, to, Some(group.clone()))?;
        }
        let tasks = self.agents[from].tasks.clone();
        self.agents.get_mut(to).unwrap().tasks.extend(tasks);
        let old = self.agents.get_mut(from).unwrap();
        old.archived = true;
        old.merged_into = Some(to.into());
        self.validate()?;
        Ok(group)
    }
    pub fn node_path(&self, node: &str) -> PathBuf {
        let mut parts = vec![];
        let mut n = &self.nodes[node];
        loop {
            parts.push(n.name.as_str());
            if let Some(p) = &n.parent {
                n = &self.nodes[p];
            } else {
                break;
            }
        }
        parts.into_iter().rev().collect()
    }
    pub fn changes(&self, old: &Self) -> Result<Changes> {
        self.validate()?;
        let mut changes = Changes::new();
        diff_records("agents", &old.agents, &self.agents, &mut changes)?;
        diff_records("nodes", &old.nodes, &self.nodes, &mut changes)?;
        diff_records("repos", &old.repos, &self.repos, &mut changes)?;
        diff_records("machines", &old.machines, &self.machines, &mut changes)?;
        Ok(changes)
    }
}
fn resolve<'a>(
    query: &str,
    values: impl Iterator<Item = (&'a String, &'a String)>,
) -> Result<String> {
    let pairs: Vec<_> = values.collect();
    if let Some((id, _)) = pairs.iter().find(|(id, _)| id.as_str() == query) {
        return Ok((*id).clone());
    }
    let found: Vec<_> = pairs
        .iter()
        .filter(|(id, name)| name.as_str() == query || (query.len() >= 10 && id.starts_with(query)))
        .collect();
    ensure!(
        found.len() == 1,
        "名称/ID {query:?} {}，请使用完整 ID",
        if found.is_empty() {
            "不存在"
        } else {
            "不唯一"
        }
    );
    Ok(found[0].0.clone())
}
fn absolute_location(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with(r"\\")
        || (path.as_bytes().get(1) == Some(&b':')
            && matches!(path.as_bytes().get(2), Some(b'/' | b'\\')))
}
fn diff_records<T: Serialize>(
    kind: &str,
    old: &BTreeMap<String, T>,
    new: &BTreeMap<String, T>,
    changes: &mut Changes,
) -> Result<()> {
    for (id, value) in new {
        let bytes = serde_json::to_vec_pretty(value)?;
        if old
            .get(id)
            .map(serde_json::to_vec_pretty)
            .transpose()?
            .as_ref()
            != Some(&bytes)
        {
            changes.insert(
                PathBuf::from(format!("catalog/{kind}/{id}.json")),
                Some(bytes),
            );
        }
    }
    for id in old.keys().filter(|id| !new.contains_key(*id)) {
        changes.insert(PathBuf::from(format!("catalog/{kind}/{id}.json")), None);
    }
    Ok(())
}
pub type Changes = BTreeMap<PathBuf, Option<Vec<u8>>>;

/// Journal lives in the worktree-specific Git directory, outside versioned content.
/// A failed transaction restores all previous files; startup repairs interrupted work.
pub struct HubLock {
    pub root: PathBuf,
    git_dir: PathBuf,
    _lock: File,
}
#[derive(Serialize, Deserialize)]
struct Undo {
    path: PathBuf,
    backup: Option<String>,
}
impl HubLock {
    pub fn open(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root)?;
        let git_dir = PathBuf::from(git(&root, &["rev-parse", "--absolute-git-dir"])?);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(git_dir.join("schedule.lock"))?;
        FileExt::try_lock_exclusive(&lock).context("另一个 tc 正在操作此工作空间")?;
        let guard = Self {
            root,
            git_dir,
            _lock: lock,
        };
        guard.recover()?;
        Ok(guard)
    }
    fn recover(&self) -> Result<()> {
        let journal = self.git_dir.join("schedule-transaction");
        let manifest = journal.join("undo.json");
        if manifest.exists() {
            let entries: Vec<Undo> = serde_json::from_slice(&fs::read(manifest)?)?;
            for entry in entries {
                let path = safe_path(&self.root, &entry.path)?;
                match entry.backup {
                    Some(file) => {
                        valid_name(&file)?;
                        atomic_write(&path, &fs::read(journal.join(file))?)?;
                    }
                    None if path.is_file() => fs::remove_file(path)?,
                    None => (),
                }
            }
        }
        if journal.exists() {
            fs::remove_dir_all(journal)?;
        }
        Ok(())
    }
    pub fn apply(&self, changes: &Changes) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let journal = self.git_dir.join("schedule-transaction");
        fs::create_dir(&journal)?;
        let result = (|| -> Result<()> {
            let mut undo = vec![];
            for (i, rel) in changes.keys().enumerate() {
                let path = safe_path(&self.root, rel)?;
                ensure!(!path.is_dir(), "目标是目录：{}", path.display());
                let backup = if path.exists() {
                    let name = format!("{i}.bin");
                    atomic_write(&journal.join(&name), &fs::read(&path)?)?;
                    Some(name)
                } else {
                    None
                };
                undo.push(Undo {
                    path: rel.clone(),
                    backup,
                });
            }
            atomic_write(&journal.join("undo.json"), &serde_json::to_vec(&undo)?)?;
            for (rel, bytes) in changes {
                let path = safe_path(&self.root, rel)?;
                if let Some(bytes) = bytes {
                    atomic_write(&path, bytes)?;
                } else if path.exists() {
                    fs::remove_file(path)?;
                }
            }
            // Removing the manifest is the transaction commit point.
            fs::remove_file(journal.join("undo.json"))?;
            Ok(())
        })();
        if result.is_err() {
            self.recover()?;
        } else {
            fs::remove_dir_all(&journal)?;
        }
        result
    }
}

pub fn register_repo(
    cat: &mut Catalog,
    device: &Device,
    path: &Path,
    name: Option<String>,
) -> Result<String> {
    let root = fs::canonicalize(git(path, &["rev-parse", "--show-toplevel"])?)
        .context("仓库路径不存在")?;
    let location = Location {
        machine: device.id.clone(),
        path: path_string(&root)?,
    };
    if let Some(r) = cat.repos.values().find(|r| r.locations.contains(&location)) {
        return Ok(r.id.clone());
    }
    device.register(cat);
    let remote = git(&root, &["remote", "get-url", "origin"])
        .ok()
        .filter(|r| portable_remote(r));
    let id = new_id("repo")?;
    let name = name.unwrap_or_else(|| {
        root.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    valid_name(&name)?;
    cat.repos.insert(
        id.clone(),
        Repository {
            id: id.clone(),
            name,
            remote,
            locations: vec![location],
            legacy: None,
        },
    );
    Ok(id)
}
pub fn bytes_hash(path: &Path) -> Result<Option<String>> {
    if path.exists() {
        Ok(Some(fingerprint(&fs::read(path)?)))
    } else {
        Ok(None)
    }
}
