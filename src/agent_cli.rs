use crate::{
    agent_files::{self, content_path, files_for},
    agents::*,
    cli::{Response, response},
    domain::State,
    hub,
    store::Store,
    ui,
    workspace::{LocalSettings, fingerprint, git, portable_remote},
};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub enum MachineCommand {
    /// 为当前设备设置可共享别名；不改变稳定机器 ID
    Alias {
        name: String,
    },
    List,
}
#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// 创建 agent 工作容器（不会创建平台服务端项目）
    Create {
        name: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        source: Option<String>,
    },
    List {
        #[arg(long)]
        all: bool,
    },
    /// 按用户熟悉的名称展示项目→文件夹→仓库/聊天
    Tree {
        agent: Option<String>,
    },
    Show {
        agent: String,
    },
    /// 为既有同步身份增加平台容器来源
    Bind {
        agent: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        source: String,
    },
    Folder {
        agent: String,
        name: String,
        #[arg(long)]
        parent: Option<String>,
    },
    /// 预览目录纳管；--apply 才保存，独立 Git 仓库只登记索引
    Import {
        agent: String,
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        apply: bool,
    },
    /// 从本机已绑定来源增量收集；双边修改停止，缺失不会默认为删除
    Collect {
        node: String,
        #[arg(long)]
        apply: bool,
        #[arg(long, requires = "apply")]
        prune: bool,
    },
    /// 导出原目录结构到新目录；--bind 可将其作为本机后续 collect 来源
    Export {
        agent: String,
        #[arg(long)]
        to: PathBuf,
        #[arg(long)]
        bind: bool,
    },
    /// 查看节点正文在中央工作副本的位置
    Path {
        node: String,
    },
    Rename {
        agent: String,
        name: String,
    },
    RenameNode {
        node: String,
        name: String,
    },
    Move {
        node: String,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        parent: Option<String>,
    },
    /// 将文件夹子树拆为新 agent 项目，保留节点身份和正文
    Split {
        node: String,
        #[arg(long)]
        name: String,
    },
    /// 默认预览；合并保留来源分区，不拼接指令、不操作平台服务端
    Merge {
        from: String,
        into: String,
        #[arg(long)]
        apply: bool,
    },
    Archive {
        agent: String,
        #[arg(long)]
        restore: bool,
    },
    /// 删除中央节点子树；默认预览，原目录与原代码仓库不变
    Remove {
        node: String,
        #[arg(long)]
        apply: bool,
    },
    /// 对项目或节点登记逻辑任务多对多关联，不改动任务/额度
    Link {
        target: String,
        #[arg(long, required = true)]
        task: Vec<u64>,
        #[arg(long)]
        remove: bool,
    },
    /// 增加聊天来源与显式交接摘要；不读取隐藏云端记忆
    Chat {
        agent: String,
        title: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long, default_value = "manual")]
        provider: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        summary: Option<PathBuf>,
    },
    /// 在文件夹中引用已登记代码仓库
    Attach {
        agent: String,
        repo: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// 本地正文搜索；未选择正文会单独报告
    Find {
        query: String,
        #[arg(long)]
        agent: Option<String>,
    },
}
#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    /// 本地仓库记录机器位置；--remote 可只登记远程仓库
    Add {
        name: String,
        path: Option<PathBuf>,
        #[arg(long)]
        remote: Option<String>,
    },
    /// 绑定当前机器的既有仓库，不复制代码
    Bind {
        repo: String,
        path: PathBuf,
    },
    Remote {
        repo: String,
        url: String,
    },
    Status {
        repo: Option<String>,
    },
    Show {
        repo: String,
    },
    /// 按需从原远程克隆代码；无 remote 时显示来源机器
    Checkout {
        repo: String,
        #[arg(long)]
        to: PathBuf,
    },
}
fn connected(store: &Store) -> Result<PathBuf> {
    let local = LocalSettings::load(store)?;
    let root = local.hub()?.to_owned();
    hub::require_v2(&root)?;
    Ok(root)
}
fn parent(cat: &Catalog, value: Option<String>) -> Result<Option<String>> {
    value.map(|p| cat.node_id(&p)).transpose()
}
fn answer(value: serde_json::Value, text: impl Into<String>) -> Response {
    response(value, text.into(), false)
}
fn saved(id: &str, text: &str) -> Response {
    answer(
        json!({"id":id,"message":text}),
        format!("{text}\nID：{id}\n使用 hub diff 审阅，hub commit 保存版本，hub push 上传。"),
    )
}
fn save(lock: &HubLock, old: &Catalog, cat: &Catalog, extra: Changes) -> Result<()> {
    let mut changes = cat.changes(old)?;
    changes.extend(extra);
    // Validate projected file/child name collisions before touching the worktree.
    for n in cat.nodes.values() {
        let mut names: BTreeSet<_> = cat
            .nodes
            .values()
            .filter(|c| c.parent.as_deref() == Some(&n.id))
            .map(|c| c.name.to_lowercase())
            .collect();
        let dir = lock.root.join("content").join(&n.id);
        if dir.exists() {
            for file in fs::read_dir(&dir)? {
                let file = file?;
                let name = file.file_name().to_string_lossy().into_owned();
                if changes.get(&content_path(&n.id, &name)) == Some(&None) {
                    continue;
                }
                ensure!(
                    names.insert(name.to_lowercase()),
                    "文件/子目录名称冲突：{name}"
                );
            }
        }
        for (path, bytes) in &changes {
            if bytes.is_some() && path.parent() == Some(Path::new("content").join(&n.id).as_path())
            {
                let name = path.file_name().unwrap().to_string_lossy();
                if !dir.join(name.as_ref()).exists() {
                    ensure!(
                        names.insert(name.to_lowercase()),
                        "文件/子目录名称冲突：{name}"
                    );
                }
            }
        }
    }
    lock.apply(&changes)
}
pub fn machine(command: MachineCommand, store: &Store) -> Result<Response> {
    let mut device = Device::load(store)?;
    let local = LocalSettings::load(store)?;
    if let MachineCommand::Alias { name } = command {
        valid_name(&name)?;
        device.alias = Some(name.clone());
        if let Some(root) = local
            .hub
            .as_ref()
            .filter(|r| hub::format(r).ok() == Some(2))
        {
            let lock = HubLock::open(root)?;
            let old = hub::check(root)?;
            let mut cat = old.clone();
            device.register(&mut cat);
            cat.machines.get_mut(&device.id).unwrap().alias = name.clone();
            save(&lock, &old, &cat, Changes::new())?;
        }
        device.save(store)?;
        return Ok(answer(
            json!({"id":device.id,"alias":name}),
            format!(
                "当前设备：{name}\n机器 ID：{}；改别名不会改变仓库引用。",
                device.id
            ),
        ));
    }
    let root = connected(store)?;
    let _lock = HubLock::open(&root)?;
    let cat = hub::check(&root)?;
    Ok(answer(
        json!({"current":device.id,"machines":cat.machines}),
        ui::table(
            &["机器", "ID", "本机"],
            cat.machines
                .values()
                .map(|m| {
                    vec![
                        m.alias.clone(),
                        m.id.clone(),
                        if m.id == device.id { "是" } else { "" }.into(),
                    ]
                })
                .collect(),
            &[20, 42, 5],
        ),
    ))
}
pub fn agent(command: AgentCommand, state: &State, store: &Store) -> Result<Response> {
    let root = connected(store)?;
    let lock = HubLock::open(&root)?;
    let old = hub::check(&root)?;
    let mut cat = old.clone();
    let mut device = Device::load(store)?;
    match command {
        AgentCommand::Create {
            name,
            provider,
            source,
        } => {
            ensure!(
                source.is_none() || provider.is_some(),
                "--source 需要 --provider"
            );
            let binding = provider.map(|provider| Binding {
                provider,
                source: source.unwrap_or_default(),
            });
            let id = cat.add_agent(name, binding)?;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "已创建 agent 项目；可独立于逻辑任务使用"))
        }
        AgentCommand::List { all } => {
            let agents: Vec<_> = cat.agents.values().filter(|a| all || !a.archived).collect();
            let text = ui::table(
                &["Agent 项目", "ID", "逻辑任务", "状态"],
                agents
                    .iter()
                    .map(|a| {
                        vec![
                            a.name.clone(),
                            a.id.clone(),
                            a.tasks
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                            if a.archived { "归档" } else { "活动" }.into(),
                        ]
                    })
                    .collect(),
                &[24, 42, 20, 6],
            );
            Ok(answer(json!(agents), text))
        }
        AgentCommand::Tree { agent } => {
            let filter = agent.map(|q| cat.agent_id(&q)).transpose()?;
            let mut text = String::new();
            for a in cat
                .agents
                .values()
                .filter(|a| filter.as_ref().map_or(!a.archived, |id| id == &a.id))
            {
                text.push_str(&format!(
                    "{} [{}]{}\n",
                    a.name,
                    a.id,
                    if a.archived { " · 已归档" } else { "" }
                ));
                render_tree(&cat, &root, &a.id, None, "", &mut text)?;
            }
            Ok(answer(
                json!({"agents":cat.agents,"nodes":cat.nodes,"repositories":cat.repos,"tree":text}),
                text,
            ))
        }
        AgentCommand::Show { agent } => {
            let id = cat.agent_id(&agent)?;
            let a = &cat.agents[&id];
            Ok(answer(json!(a), serde_json::to_string_pretty(a)?))
        }
        AgentCommand::Bind {
            agent,
            provider,
            source,
        } => {
            let id = cat.agent_id(&agent)?;
            let b = Binding { provider, source };
            if !cat.agents[&id].bindings.contains(&b) {
                cat.agents.get_mut(&id).unwrap().bindings.push(b);
            }
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "平台来源已关联；未操作云端项目"))
        }
        AgentCommand::Folder {
            agent,
            name,
            parent: p,
        } => {
            let a = cat.agent_id(&agent)?;
            let p = parent(&cat, p)?;
            let id = cat.add_node(&a, p, name, NodeKind::Folder, None)?;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "文件夹已创建"))
        }
        AgentCommand::Import {
            agent,
            path,
            name,
            parent: p,
            exclude,
            apply,
        } => {
            let a = cat.agent_id(&agent)?;
            let p = parent(&cat, p)?;
            let name = name
                .or_else(|| path.file_name().map(|x| x.to_string_lossy().into_owned()))
                .context("请用 --name 指定文件夹名")?;
            let import = agent_files::import(
                &cat,
                &device,
                &root,
                &path,
                agent_files::ImportOptions {
                    agent: a,
                    parent: p,
                    name,
                    excludes: exclude,
                },
            )?;
            if apply {
                save(&lock, &old, &import.catalog, import.changes)?;
                if !import.binding.path.as_os_str().is_empty() {
                    device
                        .sources
                        .insert(import.summary.root.clone(), import.binding);
                }
                device.save(store)?;
            }
            let text = format!(
                "{}：{} 个文件，{} bytes，{} 个仓库引用，跳过 {} 项\n根节点：{}\n{}",
                if apply { "已纳管" } else { "预览" },
                import.summary.files,
                import.summary.bytes,
                import.summary.repositories,
                import.summary.skipped.len(),
                import.summary.root,
                if apply {
                    "原目录未修改；后续 agent collect 按基准更新。"
                } else {
                    "加 --apply 保存；仓库只保存索引。"
                }
            );
            Ok(answer(
                json!({"applied":apply,"summary":import.summary}),
                text,
            ))
        }
        AgentCommand::Collect { node, apply, prune } => {
            let id = cat.node_id(&node)?;
            let result = agent_files::collect(&cat, &device, &root, &id, prune)?;
            if apply {
                save(&lock, &old, &result.catalog, result.changes)?;
                device.sources.insert(id.clone(), result.binding);
                device.save(store)?;
            }
            Ok(answer(
                json!({"applied":apply,"summary":result.summary}),
                format!(
                    "{}：{} 个来源文件，删除 {} 个文件；{}",
                    if apply { "已收集" } else { "预览" },
                    result.summary.files,
                    result.summary.deleted,
                    if apply {
                        "中央修改已保留，使用 hub diff 审阅。"
                    } else {
                        "加 --apply 保存；默认不传播来源删除。"
                    }
                ),
            ))
        }
        AgentCommand::Export { agent, to, bind } => {
            let id = cat.agent_id(&agent)?;
            let result = agent_files::export(&cat, &root, &id, &to)?;
            if bind {
                bind_export(&cat, &root, &id, &to, &mut device)?;
                device.save(store)?;
            }
            Ok(answer(
                json!(result),
                format!(
                    "已恢复到 {}：{} 个正文文件、{} 个仓库入口；代码按需 repo checkout。{}",
                    to.display(),
                    result.files,
                    result.repositories.len(),
                    if bind {
                        "已绑定导出目录，后续可 agent collect。"
                    } else {
                        "这是导出副本；需要后续收集可导出时使用 --bind。"
                    }
                ),
            ))
        }
        AgentCommand::Path { node } => {
            let id = cat.node_id(&node)?;
            let p = root.join("content").join(&id);
            Ok(answer(
                json!({"id":id,"path":p,"logical_path":cat.node_path(&id)}),
                format!(
                    "{}\n逻辑位置：{}\n此处可编辑直属文件；恢复完整工作目录请 agent export。",
                    p.display(),
                    cat.node_path(&id).display()
                ),
            ))
        }
        AgentCommand::Rename { agent, name } => {
            let id = cat.agent_id(&agent)?;
            cat.agents.get_mut(&id).unwrap().name = name;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "项目名称已更新，身份与正文不变"))
        }
        AgentCommand::RenameNode { node, name } => {
            let id = cat.node_id(&node)?;
            cat.nodes.get_mut(&id).unwrap().name = name;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "节点名称已更新"))
        }
        AgentCommand::Move {
            node,
            agent,
            parent: p,
        } => {
            let id = cat.node_id(&node)?;
            let p = parent(&cat, p)?;
            let a = agent
                .map(|a| cat.agent_id(&a))
                .transpose()?
                .unwrap_or_else(|| {
                    p.as_ref()
                        .map(|p| cat.nodes[p].agent.clone())
                        .unwrap_or_else(|| cat.nodes[&id].agent.clone())
                });
            cat.move_node(&id, &a, p)?;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "节点子树已移动；逻辑任务与用量未修改"))
        }
        AgentCommand::Split { node, name } => {
            let id = cat.node_id(&node)?;
            let a = cat.add_agent(name, None)?;
            cat.move_node(&id, &a, None)?;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&a, "已拆分为新 agent 项目，保留正文和节点身份"))
        }
        AgentCommand::Merge { from, into, apply } => {
            let from = cat.agent_id(&from)?;
            let to = cat.agent_id(&into)?;
            let group = cat.merge(&from, &to)?;
            if apply {
                save(&lock, &old, &cat, Changes::new())?;
            }
            Ok(answer(
                json!({"applied":apply,"from":from,"into":to,"group":group}),
                format!(
                    "{}：来源内容保留在目标项目的独立分组；来源项目归档并记录去向。{}",
                    if apply { "已合并" } else { "合并预览" },
                    if apply {
                        "逻辑任务与额度不变。"
                    } else {
                        "加 --apply 保存。"
                    }
                ),
            ))
        }
        AgentCommand::Archive { agent, restore } => {
            let id = cat.agent_id(&agent)?;
            ensure!(
                !restore || cat.agents[&id].merged_into.is_none(),
                "此项目已合并；恢复合并前结构请使用 Git 恢复对应提交"
            );
            cat.agents.get_mut(&id).unwrap().archived = !restore;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(
                &id,
                if restore {
                    "已恢复项目"
                } else {
                    "已归档，正文保留"
                },
            ))
        }
        AgentCommand::Remove { node, apply } => {
            let id = cat.node_id(&node)?;
            let subtree = cat.descendants(&id);
            let mut changes = Changes::new();
            for n in &subtree {
                for (name, _) in files_for(&root, n)? {
                    changes.insert(content_path(n, &name), None);
                }
                cat.nodes.remove(n);
            }
            if apply {
                ensure!(
                    git(&root, &["status", "--porcelain"])?.is_empty(),
                    "删除前先 hub commit，确保可由 Git 恢复"
                );
                save(&lock, &old, &cat, changes)?;
            }
            Ok(answer(
                json!({"applied":apply,"removed_nodes":subtree}),
                format!(
                    "{} {} 个中央节点；原目录、原代码仓库及逻辑任务不变。{}",
                    if apply { "已移除" } else { "将移除" },
                    subtree.len(),
                    if apply {
                        ""
                    } else {
                        "先提交当前版本，再加 --apply。"
                    }
                ),
            ))
        }
        AgentCommand::Link {
            target,
            task,
            remove,
        } => {
            if !remove {
                for id in &task {
                    state.task(*id)?;
                }
            }
            let (id, tasks) = if let Ok(id) = cat.agent_id(&target) {
                let tasks = &mut cat.agents.get_mut(&id).unwrap().tasks;
                (id, tasks)
            } else {
                let id = cat.node_id(&target)?;
                let tasks = &mut cat.nodes.get_mut(&id).unwrap().tasks;
                (id, tasks)
            };
            for t in task {
                if remove {
                    tasks.remove(&t);
                } else {
                    tasks.insert(t);
                }
            }
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "逻辑任务关联已更新；没有创建或重复计算用量"))
        }
        AgentCommand::Chat {
            agent,
            title,
            parent: p,
            provider,
            source,
            summary,
        } => {
            let a = cat.agent_id(&agent)?;
            let p = parent(&cat, p)?;
            let id = cat.add_node(&a, p, title, NodeKind::Conversation, None)?;
            cat.nodes.get_mut(&id).unwrap().source = Some(Binding { provider, source });
            let mut extra = Changes::new();
            if let Some(file) = summary {
                extra.insert(content_path(&id, "SUMMARY.md"), Some(fs::read(file)?));
            }
            save(&lock, &old, &cat, extra)?;
            Ok(saved(&id, "聊天来源与摘要已登记"))
        }
        AgentCommand::Attach {
            agent,
            repo,
            parent: p,
            name,
        } => {
            let a = cat.agent_id(&agent)?;
            let r = cat.repo_id(&repo)?;
            let p = parent(&cat, p)?;
            let name = name.unwrap_or_else(|| cat.repos[&r].name.clone());
            let id = cat.add_node(&a, p, name, NodeKind::Repository, Some(r))?;
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "已添加仓库引用；代码未复制"))
        }
        AgentCommand::Find { query, agent } => {
            ensure!(!query.is_empty(), "查询不能为空");
            let filter = agent.map(|a| cat.agent_id(&a)).transpose()?;
            let q = query.to_lowercase();
            let mut found = vec![];
            let mut missing = 0;
            for n in cat.nodes.values().filter(|n| {
                n.kind != NodeKind::Repository && filter.as_ref().is_none_or(|a| a == &n.agent)
            }) {
                let tracked = git(
                    &root,
                    &["ls-files", "-z", "--", &format!("content/{}", n.id)],
                )?;
                if tracked
                    .split('\0')
                    .filter(|p| !p.is_empty())
                    .any(|p| !root.join(p).exists())
                {
                    missing += 1;
                    continue;
                }
                for (name, bytes) in files_for(&root, &n.id)? {
                    if bytes.len() > 2 * 1024 * 1024 {
                        continue;
                    }
                    if let Ok(text) = String::from_utf8(bytes) {
                        if text.contains('\0') {
                            continue;
                        }
                        for (line, value) in text.lines().enumerate() {
                            if value.to_lowercase().contains(&q) {
                                found.push(json!({"agent":n.agent,"node":n.id,"file":name,"line":line+1,"text":value}));
                            }
                        }
                    }
                }
            }
            let text = found
                .iter()
                .map(|x| {
                    format!(
                        "{} / {}:{}  {}",
                        cat.agents[x["agent"].as_str().unwrap()].name,
                        x["file"].as_str().unwrap(),
                        x["line"],
                        x["text"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(answer(
                json!({"matches":found,"unavailable_nodes":missing}),
                format!(
                    "{text}\n{} 处匹配；{missing} 个节点正文未下载/已从工作树移除，未搜索。",
                    found.len()
                ),
            ))
        }
    }
}
fn render_tree(
    cat: &Catalog,
    hub: &Path,
    agent: &str,
    parent: Option<&str>,
    prefix: &str,
    out: &mut String,
) -> Result<()> {
    let mut children: Vec<_> = cat
        .nodes
        .values()
        .filter(|n| n.agent == agent && n.parent.as_deref() == parent)
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    for (i, n) in children.iter().enumerate() {
        let last = i + 1 == children.len();
        let label = match n.kind {
            NodeKind::Repository => {
                let r = &cat.repos[n.repository.as_ref().unwrap()];
                r.remote.clone().unwrap_or_else(|| {
                    format!(
                        "仅本地：{}",
                        r.locations
                            .iter()
                            .map(|l| format!("{} → {}", cat.machines[&l.machine].alias, l.path))
                            .collect::<Vec<_>>()
                            .join("；")
                    )
                })
            }
            NodeKind::Conversation => "聊天来源／摘要".into(),
            NodeKind::Folder => "文件夹".into(),
        };
        out.push_str(&format!(
            "{prefix}{} {} [{label}] #{}\n",
            if last { "└──" } else { "├──" },
            n.name,
            &n.id
        ));
        if n.kind != NodeKind::Repository {
            let path = hub.join("content").join(&n.id);
            let tracked = git(hub, &["ls-files", "-z", "--", &format!("content/{}", n.id)])?;
            let missing = tracked
                .split('\0')
                .filter(|p| !p.is_empty() && !hub.join(p).exists())
                .count();
            if missing > 0 {
                out.push_str(&format!(
                    "{prefix}{}    （{missing} 个正文不在本机工作树，见 hub status/select）\n",
                    if last { "    " } else { "│   " }
                ));
            }
            if path.exists() {
                let mut files: Vec<_> = fs::read_dir(path)?.collect::<std::io::Result<_>>()?;
                files.sort_by_key(|f| f.file_name());
                for f in files {
                    out.push_str(&format!(
                        "{prefix}{}    {}\n",
                        if last { "    " } else { "│   " },
                        f.file_name().to_string_lossy()
                    ));
                }
            }
        }
        render_tree(
            cat,
            hub,
            agent,
            Some(&n.id),
            &format!("{prefix}{}", if last { "    " } else { "│   " }),
            out,
        )?;
    }
    Ok(())
}
fn bind_export(
    cat: &Catalog,
    hub: &Path,
    agent: &str,
    to: &Path,
    device: &mut Device,
) -> Result<()> {
    for root in cat
        .nodes
        .values()
        .filter(|n| n.agent == agent && n.parent.is_none() && n.kind == NodeKind::Folder)
    {
        let root_path = cat.node_path(&root.id);
        let mut binding = SourceBinding {
            path: fs::canonicalize(to.join(&root_path))?,
            ..SourceBinding::default()
        };
        for id in cat.descendants(&root.id) {
            let relative = cat.node_path(&id).strip_prefix(&root_path)?.to_owned();
            let rel = relative.to_string_lossy().replace('\\', "/");
            binding.directories.insert(rel.clone(), id.clone());
            if cat.nodes[&id].kind != NodeKind::Repository {
                for (name, bytes) in files_for(hub, &id)? {
                    binding.baseline.insert(
                        if rel.is_empty() {
                            name
                        } else {
                            format!("{rel}/{name}")
                        },
                        fingerprint(&bytes),
                    );
                }
            }
        }
        device.sources.insert(root.id.clone(), binding);
    }
    Ok(())
}
pub fn repo(command: RepoCommand, store: &Store) -> Result<Response> {
    let root = connected(store)?;
    let lock = HubLock::open(&root)?;
    let old = hub::check(&root)?;
    let mut cat = old.clone();
    let device = Device::load(store)?;
    match command {
        RepoCommand::Add { name, path, remote } => {
            ensure!(
                path.is_some() || remote.is_some(),
                "需要本地路径或 --remote"
            );
            let id = if let Some(path) = path {
                let canonical = fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)?;
                ensure!(canonical != root, "不能把 sync 仓库注册为代码仓库");
                register_repo(&mut cat, &device, &path, Some(name))?
            } else {
                valid_name(&name)?;
                let id = new_id("repo")?;
                cat.repos.insert(
                    id.clone(),
                    Repository {
                        id: id.clone(),
                        name,
                        remote: remote.clone(),
                        locations: vec![],
                        legacy: None,
                    },
                );
                id
            };
            if let Some(remote) = remote {
                ensure!(portable_remote(&remote), "远程地址不可包含凭据");
                cat.repos.get_mut(&id).unwrap().remote = Some(remote);
            }
            save(&lock, &old, &cat, Changes::new())?;
            device.save(store)?;
            Ok(saved(&id, "仓库已登记；本地位置按机器记录，未复制代码"))
        }
        RepoCommand::Bind { repo, path } => {
            let id = cat.repo_id(&repo)?;
            ensure!(
                fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)?
                    != fs::canonicalize(&root)?,
                "不能把 sync 仓库绑定为代码仓库"
            );
            bind_repo(&mut cat, &device, &id, &path)?;
            save(&lock, &old, &cat, Changes::new())?;
            device.save(store)?;
            Ok(saved(&id, "已绑定本机仓库位置"))
        }
        RepoCommand::Remote { repo, url } => {
            ensure!(portable_remote(&url), "需要无凭据 HTTPS/git@ 地址");
            let id = cat.repo_id(&repo)?;
            cat.repos.get_mut(&id).unwrap().remote = Some(url);
            save(&lock, &old, &cat, Changes::new())?;
            Ok(saved(&id, "仓库远程索引已更新；未修改原仓库 Git 配置"))
        }
        RepoCommand::Show { repo } => {
            let id = cat.repo_id(&repo)?;
            Ok(answer(
                json!(cat.repos[&id]),
                serde_json::to_string_pretty(&cat.repos[&id])?,
            ))
        }
        RepoCommand::Checkout { repo, to } => {
            let id = cat.repo_id(&repo)?;
            let r = &cat.repos[&id];
            ensure!(!to.exists(), "只检出到新目录，避免覆盖现有文件");
            let remote = if let Some(url) = &r.remote {
                url.clone()
            } else if let Some(bundle) = r.legacy.as_ref().filter(|p| p.ends_with(".bundle")) {
                path_string(&safe_path(&root, Path::new(bundle))?)?
            } else {
                anyhow::bail!(
                    "此仓库只有本地索引，不能恢复代码。来源：{}",
                    r.locations
                        .iter()
                        .map(|l| format!("{} → {}", cat.machines[&l.machine].alias, l.path))
                        .collect::<Vec<_>>()
                        .join("；")
                );
            };
            let parent = to
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::create_dir_all(parent)?;
            git(
                parent,
                &[
                    "clone",
                    "--",
                    &remote,
                    to.file_name()
                        .and_then(|x| x.to_str())
                        .context("目标目录名无效")?,
                ],
            )?;
            bind_repo(&mut cat, &device, &id, &to)?;
            save(&lock, &old, &cat, Changes::new())?;
            device.save(store)?;
            Ok(answer(
                json!({"id":id,"path":to}),
                format!("已从原仓库检出并绑定：{}", to.display()),
            ))
        }
        RepoCommand::Status { repo } => {
            let filter = repo.map(|r| cat.repo_id(&r)).transpose()?;
            let mut rows = vec![];
            let mut values = vec![];
            for r in cat
                .repos
                .values()
                .filter(|r| filter.as_ref().is_none_or(|id| id == &r.id))
            {
                let locations: Vec<_> = r
                    .locations
                    .iter()
                    .filter(|l| l.machine == device.id)
                    .collect();
                if locations.is_empty() {
                    rows.push(vec![
                        r.name.clone(),
                        r.id.clone(),
                        r.remote
                            .clone()
                            .unwrap_or_else(|| "仅本地，代码未备份".into()),
                        "本机未绑定".into(),
                    ]);
                    values.push(json!({"repository":r,"local":false}));
                }
                for loc in locations {
                    let path = Path::new(&loc.path);
                    let status = git(path, &["-c", "core.quotepath=false", "status", "--short"]);
                    let head = git(path, &["rev-parse", "--short", "HEAD"]).ok();
                    let branch = git(path, &["branch", "--show-current"]).ok();
                    let counts = git(
                        path,
                        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
                    )
                    .ok();
                    let text = match &status {
                        Ok(s) => format!(
                            "{}，{} 项改动{}",
                            branch.as_deref().unwrap_or(""),
                            s.lines().count(),
                            counts
                                .as_ref()
                                .map(|c| format!("，上游落后/领先 {c}"))
                                .unwrap_or_default()
                        ),
                        Err(e) => format!("不可访问：{e}"),
                    };
                    rows.push(vec![r.name.clone(), r.id.clone(), loc.path.clone(), text]);
                    values.push(json!({"repository":r.id,"path":loc.path,"head":head,"branch":branch,"changes":status.as_ref().ok(),"error":status.as_ref().err().map(ToString::to_string),"behind_ahead":counts}));
                }
            }
            Ok(answer(
                json!(values),
                ui::table(
                    &["仓库", "ID", "位置", "本机状态（未联网）"],
                    rows,
                    &[20, 42, 50, 48],
                ),
            ))
        }
    }
}
fn bind_repo(cat: &mut Catalog, device: &Device, id: &str, path: &Path) -> Result<()> {
    let root = fs::canonicalize(git(path, &["rev-parse", "--show-toplevel"])?)?;
    device.register(cat);
    let loc = Location {
        machine: device.id.clone(),
        path: path_string(&root)?,
    };
    let r = cat.repos.get_mut(id).context("仓库不存在")?;
    if let (Some(expected), Ok(found)) = (&r.remote, git(&root, &["remote", "get-url", "origin"])) {
        ensure!(
            remote_key(expected) == remote_key(&found),
            "本地 origin 与索引不同；请核对仓库身份，必要时先 repo remote"
        );
    }
    if !r.locations.contains(&loc) {
        r.locations.push(loc);
    }
    Ok(())
}
fn remote_key(value: &str) -> String {
    let value = value
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .replace("git@github.com:", "https://github.com/");
    if value
        .to_ascii_lowercase()
        .starts_with("https://github.com/")
    {
        value.to_lowercase()
    } else {
        value
    }
}

pub fn hub_command(
    command: crate::commands::HubCommand,
    state: &mut State,
    store: &Store,
) -> Result<Response> {
    use crate::commands::HubCommand;
    let mut local = LocalSettings::load(store)?;
    match command {
        HubCommand::Init {
            path,
            remote,
            github,
            repo,
        } => {
            let remote = if let Some(owner) = github {
                ensure!(
                    !owner.is_empty()
                        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                    "GitHub 账号无效"
                );
                crate::workspace::slug(&repo)?;
                Some(format!("https://github.com/{owner}/{repo}.git"))
            } else {
                remote
            };
            let root = hub::init(&path, remote.as_deref(), state)?;
            let mut device = Device::load(store)?;
            hub::seed_legacy(&root, state, &local, &device)?;
            local.hub = Some(root.clone());
            local.base_state_hash = Some(crate::workspace::state_hash(state)?);
            local.save(store)?;
            device.selection = None;
            device.save(store)?;
            Ok(answer(
                json!({"path":root,"remote":remote,"format":2}),
                format!(
                    "已创建 v0.3 工作空间：{}\n可设置 machine alias、创建 agent 项目。hub push 才会上传；GitHub 仓库需预先创建。",
                    root.display()
                ),
            ))
        }
        HubCommand::Clone {
            remote,
            path,
            metadata_only,
        } => {
            let root = hub::clone(&remote, &path, metadata_only)?;
            local.hub = Some(root.clone());
            local.base_state_hash = None;
            local.save(store)?;
            let mut device = Device::load(store)?;
            device.selection = if metadata_only { Some(vec![]) } else { None };
            device.save(store)?;
            Ok(answer(
                json!({"path":root,"metadata_only":metadata_only}),
                "已连接 sync 仓库。agent list 查看容器；hub pull 导入规划，或 --agents-only 保持本机规划不变。设置本机 machine alias 后按需选择正文。",
            ))
        }
        HubCommand::Migrate { apply } => {
            let root = local.hub()?.to_owned();
            let device = Device::load(store)?;
            let result = hub::migrate(&root, state, &local, &device, apply)?;
            if apply {
                device.save(store)?;
            }
            Ok(answer(
                result.clone(),
                format!(
                    "{}\n{}",
                    serde_json::to_string_pretty(&result)?,
                    if apply {
                        "使用 hub check / hub diff 审阅，再 hub commit / hub push。"
                    } else {
                        "加 --apply 执行；旧文件会保留在 legacy/v02。"
                    }
                ),
            ))
        }
        HubCommand::Remote { url } => {
            let root = connected(store)?;
            let _lock = HubLock::open(&root)?;
            let op = if crate::workspace::hub_remote(&root)?.is_some() {
                "set-url"
            } else {
                "add"
            };
            git(&root, &["remote", op, "origin", &url])?;
            Ok(answer(json!({"remote":url}), "同步仓库地址已更新"))
        }
        HubCommand::Status => {
            let root = connected(store)?;
            let _lock = HubLock::open(&root)?;
            let cat = hub::check(&root)?;
            let dirty = git(&root, &["-c", "core.quotepath=false", "status", "--short"])?;
            let changed =
                local.base_state_hash.as_ref() != Some(&crate::workspace::state_hash(state)?);
            let device = Device::load(store)?;
            Ok(answer(
                json!({"hub":root,"format":2,"agents":cat.agents.len(),"repositories":cat.repos.len(),"planning_changed":changed,"selection":device.selection,"git_changes":dirty}),
                format!(
                    "工作空间：{}\nAgent 项目 {}；仓库索引 {}\n本机规划{}；正文选择：{}\n{}\n未访问网络。",
                    root.display(),
                    cat.agents.len(),
                    cat.repos.len(),
                    if changed {
                        "有未同步变更"
                    } else {
                        "与上次同步一致"
                    },
                    if device.selection.is_none() {
                        "全部"
                    } else {
                        "按需"
                    },
                    dirty
                ),
            ))
        }
        HubCommand::Check => {
            let root = connected(store)?;
            let _lock = HubLock::open(&root)?;
            let cat = hub::check(&root)?;
            let tasks: BTreeSet<_> = state.tasks.iter().map(|t| t.id).collect();
            let missing: BTreeSet<_> = cat
                .agents
                .values()
                .flat_map(|a| a.tasks.iter())
                .chain(cat.nodes.values().flat_map(|n| n.tasks.iter()))
                .filter(|id| !tasks.contains(id))
                .copied()
                .collect();
            Ok(answer(
                json!({"valid":true,"agents":cat.agents.len(),"nodes":cat.nodes.len(),"repositories":cat.repos.len(),"tasks_not_in_local_ledger":missing}),
                format!(
                    "结构、正文和规划副本校验通过。Agent 项目 {}，节点 {}，仓库 {}。\n本机账本尚未包含的关联任务：{:?}（可单独 hub pull 导入规划）。",
                    cat.agents.len(),
                    cat.nodes.len(),
                    cat.repos.len(),
                    missing
                ),
            ))
        }
        HubCommand::Diff => {
            let root = connected(store)?;
            let _lock = HubLock::open(&root)?;
            let status = git(&root, &["-c", "core.quotepath=false", "status", "--short"])?;
            let diff = git(&root, &["diff", "--stat"])?;
            let staged = git(&root, &["diff", "--cached", "--stat"])?;
            Ok(answer(
                json!({"status":status,"unstaged":diff,"staged":staged}),
                format!("{status}\n未暂存：\n{diff}\n已暂存：\n{staged}"),
            ))
        }
        HubCommand::Commit { message } => {
            let root = connected(store)?;
            let _lock = HubLock::open(&root)?;
            let id = hub::commit(&root, &message)?;
            Ok(answer(
                json!({"commit":id}),
                format!("本地版本已保存：{id}；尚未上传。"),
            ))
        }
        HubCommand::Select { agents, all } => {
            let root = connected(store)?;
            let _lock = HubLock::open(&root)?;
            let cat = hub::check(&root)?;
            let mut device = Device::load(store)?;
            let selected = agents
                .iter()
                .map(|a| cat.agent_id(a).or_else(|_| cat.node_id(a)))
                .collect::<Result<Vec<_>>>()?;
            hub::select(&root, &cat, if all { None } else { Some(&selected) })?;
            device.selection = if all { None } else { Some(selected) };
            device.save(store)?;
            Ok(answer(
                json!({"selection":device.selection}),
                "本机正文选择已更新；没有删除中央文件。工作树选择不等于访问权限，也不保证减少已有 Git 对象。",
            ))
        }
        HubCommand::Push { agents_only } => {
            let id = hub::push(state, &mut local, agents_only)?;
            local.save(store)?;
            Ok(answer(
                json!({"commit":id,"agents_only":agents_only}),
                format!(
                    "同步分支已推送：{id}{}",
                    if agents_only {
                        "；未导出本机规划账本。"
                    } else {
                        "；包含规划副本。"
                    }
                ),
            ))
        }
        HubCommand::Pull {
            replace,
            agents_only,
        } => {
            let device = Device::load(store)?;
            let next = hub::pull(state, &mut local, agents_only, replace, &device)?;
            if !agents_only {
                store.save(&next)?;
                *state = next;
            }
            local.save(store)?;
            Ok(answer(
                json!({"pulled":true,"agents_only":agents_only}),
                if agents_only {
                    "agent 工作空间已更新；本机逻辑任务和用量未改变。"
                } else {
                    "工作空间和规划已同步；机器身份保持本机独立。"
                },
            ))
        }
    }
}
