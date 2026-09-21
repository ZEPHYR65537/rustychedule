use crate::{
    cli::{Response, ok, preference_edges, response},
    domain::*,
    ledger::*,
    store::Store,
    ui,
    workspace::{self, LocalSettings, ProjectMode},
};
use anyhow::{Context, Result, ensure};
use chrono::{Duration, Local};
use clap::{Args, Subcommand};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Debug, Subcommand)]
pub enum SubscriptionCommand {
    /// 配置订阅及估计周 token 容量；不记录费用
    Set {
        model: String,
        #[arg(long)]
        renewal_day: Option<u32>,
        #[arg(long)]
        weekly_tokens: u64,
        #[arg(long, default_value = "subscription")]
        plan: String,
        #[arg(long)]
        next_reset: Option<String>,
        #[arg(long, default_value = "用户手动估计")]
        note: String,
    },
    /// 更新估计周容量；原始百分比观测保留，当前可用量随新估计重算
    Estimate {
        model: String,
        tokens: u64,
        #[arg(long, default_value = "用户修正估计")]
        note: String,
    },
    Active {
        model: String,
        #[arg(action=clap::ArgAction::Set)]
        enabled: bool,
    },
    List,
}
#[derive(Debug, Subcommand)]
pub enum ApiCommand {
    /// 累计购买/允许使用的 token 上界；不是剩余余额
    Set {
        model: String,
        #[arg(long)]
        limit: u64,
    },
    /// 增加允许使用的累计 token 上限（只记账，不会实际购买）
    TopUp {
        model: String,
        tokens: u64,
    },
    List,
}
#[derive(Debug, Subcommand)]
pub enum PolicyCommand {
    /// 设置全局模型偏序；任务设置的偏序会整体覆盖它
    Models {
        #[arg(long, required_unless_present = "clear", conflicts_with = "clear")]
        prefer: Vec<String>,
        #[arg(long)]
        clear: bool,
    },
    /// subscription-first / model-first / subscription-only
    Funding {
        #[arg(value_enum)]
        policy: FundingPolicy,
    },
    Show,
}
#[derive(Debug, Subcommand)]
pub enum WorkCommand {
    Log(WorkLog),
    List {
        #[arg(long)]
        task: Option<u64>,
    },
    /// 复盘：时间、实际 token、完成内容、收获及下一步
    Summary {
        task: u64,
    },
    /// 撤销复盘及它创建的用量流水，不回退手工进度
    Void {
        id: u64,
    },
}
#[derive(Debug, Args)]
pub struct WorkLog {
    pub task: u64,
    #[arg(long, default_value_t = 0)]
    pub minutes: u32,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, value_enum, default_value = "subscription")]
    pub source: Funding,
    #[arg(long, default_value_t = 0)]
    pub input: u64,
    #[arg(long, default_value_t = 0)]
    pub output: u64,
    #[arg(long, default_value_t = 0)]
    pub cached: u64,
    #[arg(long, default_value = "")]
    pub done: String,
    #[arg(long, default_value = "")]
    pub learned: String,
    #[arg(long, default_value = "")]
    pub next: String,
    #[arg(long)]
    pub progress: Option<f64>,
    #[arg(long)]
    pub at: Option<String>,
}
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// 注册原 Git 仓库；有可携带远程地址时默认 reference，否则 snapshot
    Register {
        name: String,
        path: PathBuf,
        #[arg(long, value_enum)]
        mode: Option<ProjectMode>,
        #[arg(long)]
        remote: Option<String>,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// 把同步来的项目关联到本机已有目录，不修改代码
    Bind { name: String, path: PathBuf },
    /// 从原代码仓库或工作空间快照检出到新目录
    Checkout {
        name: String,
        #[arg(long)]
        to: PathBuf,
    },
    /// 所有注册项目的分支、未提交更改、关联任务
    Status,
    /// 显示单项目的 Git 改动摘要
    Diff { name: String },
    /// 项目上下文和长期记忆（替换指定字段）
    Note {
        name: String,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        memory: Option<String>,
        #[arg(long, conflicts_with = "context")]
        context_file: Option<PathBuf>,
        #[arg(long, conflicts_with = "memory")]
        memory_file: Option<PathBuf>,
    },
    /// 为没有原远程仓库的项目制作已提交代码快照
    Snapshot { name: String },
    /// 项目、任务、记忆和复盘索引
    Show { name: String },
}
#[derive(Debug, Subcommand)]
pub enum HubCommand {
    /// 初始化专用工作空间；--github 只生成仓库地址，需已创建私有仓库
    Init {
        path: PathBuf,
        #[arg(long, conflicts_with = "github")]
        remote: Option<String>,
        #[arg(long)]
        github: Option<String>,
        #[arg(long, default_value = "schedule-workspace")]
        repo: String,
    },
    /// 换机器：克隆既有工作空间；之后 hub pull 导入任务
    Clone {
        remote: String,
        path: PathBuf,
    },
    Remote {
        url: String,
    },
    /// 展示本地同步摘要，不访问网络
    Status,
    /// 生成本地结构化索引，提交并推送工作空间（显式联网）
    Push,
    /// 下载状态并安全导入；双端变更拒绝覆盖，--replace 显式接受远端
    Pull {
        #[arg(long)]
        replace: bool,
    },
}

pub fn subscription(c: SubscriptionCommand, s: &mut State, now: Time) -> Result<Response> {
    match c {
        SubscriptionCommand::Set {
            model,
            renewal_day,
            weekly_tokens,
            plan,
            next_reset,
            note,
        } => {
            s.model(&model)?;
            tokens(weekly_tokens, 0)?;
            let binding = &s.model(&model)?.bindings[0];
            ensure!(
                s.pool(&model)?.unit == "token"
                    && binding.input_weight == 1.0
                    && binding.output_weight == 1.0,
                "此模型使用自定义额度单位/权重；请保留 quota 配置，或另建使用原始 token 的模型，避免重解释历史用量"
            );
            ensure!(weekly_tokens > 0, "估计周额度须大于 0");
            ensure!(
                renewal_day.is_none_or(|d| (1..=31).contains(&d)),
                "续期日须为 1–31"
            );
            let explicit_anchor = next_reset
                .map(|x| parse_time(&x, false).map(|t| t - Duration::days(7)))
                .transpose()?;
            let pool = s.pools.iter_mut().find(|p| p.name == model).unwrap();
            pool.unit = "token".into();
            if let Some(w) = pool.windows.iter_mut().find(|w| w.name == "week") {
                w.limit = weekly_tokens as f64;
                w.kind = WindowKind::Fixed;
                w.seconds = 604800;
                if let Some(a) = explicit_anchor {
                    w.anchor = a;
                }
            } else {
                pool.windows.push(Window {
                    name: "week".into(),
                    kind: WindowKind::Fixed,
                    limit: weekly_tokens as f64,
                    seconds: 604800,
                    anchor: explicit_anchor.unwrap_or(now),
                });
            }
            let m = s.models.iter_mut().find(|m| m.name == model).unwrap();
            m.bindings[0].input_weight = 1.0;
            m.bindings[0].output_weight = 1.0;
            s.accounts.entry(model.clone()).or_default().subscription = Some(Subscription {
                plan,
                renewal_day,
                active: true,
                estimate_note: note,
            });
            Ok(ok(
                format!(
                    "已配置 {model}：周容量约 {weekly_tokens} token。月续期与周重置分开计算；不记录费用。"
                ),
                None,
            ))
        }
        SubscriptionCommand::Estimate {
            model,
            tokens: amount,
            note,
        } => {
            tokens(amount, 0)?;
            ensure!(amount > 0, "估计容量须大于 0");
            let sub = s
                .accounts
                .get_mut(&model)
                .and_then(|b| b.subscription.as_mut())
                .context("请先 subscription set")?;
            sub.estimate_note = note;
            s.pools
                .iter_mut()
                .find(|p| p.name == model)
                .unwrap()
                .windows
                .iter_mut()
                .find(|w| w.name == "week")
                .unwrap()
                .limit = amount as f64;
            Ok(ok(
                format!(
                    "{model} 周容量估计已改为 {amount}；原始百分比观测保留，当前估计已重新换算"
                ),
                None,
            ))
        }
        SubscriptionCommand::Active { model, enabled } => {
            s.accounts
                .get_mut(&model)
                .and_then(|b| b.subscription.as_mut())
                .context("订阅不存在")?
                .active = enabled;
            Ok(ok(format!("{model} 订阅启用状态：{enabled}"), None))
        }
        SubscriptionCommand::List => {
            let rows:Vec<_>=s.accounts.iter().filter_map(|(m,b)|b.subscription.as_ref().map(|sub|json!({"model":m,"subscription":sub,"next_renewal":sub.next_renewal(now.with_timezone(&Local).date_naive()),"estimated_weekly_tokens":s.pool(m).ok().and_then(|p|p.windows.iter().find(|w|w.name=="week")).map(|w|w.limit)}))).collect();
            Ok(response(json!(rows), accounts_table(s, now), false))
        }
    }
}
pub fn accounts_table(s: &State, now: Time) -> String {
    ui::table(
        &["模型", "订阅", "下次续期", "周 token 估计", "状态"],
        s.accounts
            .iter()
            .filter_map(|(m, b)| {
                b.subscription.as_ref().map(|sub| {
                    vec![
                        m.clone(),
                        sub.plan.clone(),
                        sub.next_renewal(now.with_timezone(&Local).date_naive())
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "未设置".into()),
                        format!(
                            "~{:.0}",
                            s.pool(m)
                                .unwrap()
                                .windows
                                .iter()
                                .find(|w| w.name == "week")
                                .unwrap()
                                .limit
                        ),
                        if sub.active { "启用" } else { "停用" }.into(),
                    ]
                })
            })
            .collect(),
        &[20, 20, 12, 16, 6],
    )
}
pub fn api(c: ApiCommand, s: &mut State, now: Time) -> Result<Response> {
    match c {
        ApiCommand::Set { model, limit } => {
            s.model(&model)?;
            tokens(limit, 0)?;
            s.accounts.entry(model.clone()).or_default().api_token_limit = limit;
            if limit == 0 {
                for b in &mut s.budgets {
                    if b.model == model && b.funding == Funding::Api {
                        b.released = true;
                    }
                }
            }
            Ok(ok(
                format!(
                    "{model} API 累计 token 上界设为 {limit}；0 会禁用 API 并释放未用 API 预留；订阅重置不会影响它"
                ),
                None,
            ))
        }
        ApiCommand::TopUp {
            model,
            tokens: amount,
        } => {
            s.model(&model)?;
            let b = s.accounts.entry(model.clone()).or_default();
            b.api_token_limit = b
                .api_token_limit
                .checked_add(amount)
                .context("token 数量溢出")?;
            tokens(b.api_token_limit, 0)?;
            Ok(ok(
                format!("{model} API 库存上界增加 {amount} token（仅本地记账）"),
                None,
            ))
        }
        ApiCommand::List => {
            let q: Vec<_> = s
                .models
                .iter()
                .filter(|m| s.accounts.contains_key(&m.name))
                .map(|m| s.api_quota(&m.name, now))
                .collect();
            Ok(response(json!(q), ui::quota_table(&q, now), false))
        }
    }
}
pub fn policy(c: PolicyCommand, s: &mut State) -> Result<Response> {
    match c {
        PolicyCommand::Models { prefer, clear } => {
            s.model_preferences = if clear {
                vec![]
            } else {
                preference_edges(&prefer)?
            };
            Ok(ok(
                "全局模型偏序已更新；任务已有偏序时继续使用任务规则".into(),
                None,
            ))
        }
        PolicyCommand::Funding { policy } => {
            s.funding_policy = policy;
            Ok(ok(
                format!("额度来源策略 → {policy:?}；已有预算需 plan --rebalance 才重新分配"),
                None,
            ))
        }
        PolicyCommand::Show => Ok(response(
            json!({"models":s.model_preferences,"funding":s.funding_policy}),
            format!(
                "额度来源策略：{:?}\n全局模型偏序：{}\n任务优先级：重要性、紧迫性、截止日期、依赖继承。",
                s.funding_policy,
                s.model_preferences
                    .iter()
                    .map(|[a, b]| format!("{a}>{b}"))
                    .collect::<Vec<_>>()
                    .join("；")
            ),
            false,
        )),
    }
}
pub fn work(c: WorkCommand, s: &mut State, now: Time) -> Result<Response> {
    match c {
        WorkCommand::Log(a) => {
            s.task(a.task)?;
            tokens(a.input, a.output)?;
            ensure!(a.cached <= a.input, "缓存输入不能超过总输入");
            let at =
                a.at.map(|x| parse_time(&x, false))
                    .transpose()?
                    .unwrap_or(now);
            ensure!(at <= now, "不能记录未来工作");
            let usage_event = if let Some(model) = a.model {
                s.model(&model)?;
                if a.source == Funding::Api {
                    ensure!(
                        s.accounts.contains_key(&model),
                        "请先用 api set 配置 API token 上界（历史用量可如实记录为超限）"
                    );
                }
                let id = s.id();
                let amount = s.funding_units(&model, a.source, a.input, a.output);
                s.events.push(Event {
                    id,
                    at,
                    note: a.done.clone(),
                    voided: false,
                    funding: a.source,
                    observed_percent: None,
                    kind: EventKind::Usage {
                        task: Some(a.task),
                        model: model.clone(),
                        input: a.input,
                        output: a.output,
                        cached: a.cached,
                        units: BTreeMap::from([(model, amount)]),
                    },
                });
                Some(id)
            } else {
                ensure!(a.input == 0 && a.output == 0, "记录 token 时需要 --model");
                None
            };
            if let Some(progress) = a.progress {
                s.tasks
                    .iter_mut()
                    .find(|t| t.id == a.task)
                    .unwrap()
                    .progress = progress;
            }
            let id = s.id();
            s.sessions.push(WorkSession {
                id,
                task: a.task,
                at,
                minutes: a.minutes,
                usage_event,
                completed: a.done,
                learned: a.learned,
                next_steps: a.next,
                voided: false,
            });
            Ok(ok(
                format!(
                    "已记录复盘 #{id}：{} 分钟；用量事件 {:?}。任务进度仅按显式 --progress 更新。",
                    a.minutes, usage_event
                ),
                Some(id),
            ))
        }
        WorkCommand::List { task } => {
            if let Some(id) = task {
                s.task(id)?;
            }
            let rows: Vec<_> = s
                .sessions
                .iter()
                .filter(|r| task.is_none_or(|t| t == r.task))
                .collect();
            let text = ui::table(
                &["ID", "任务", "时间", "分钟", "完成内容", "收获"],
                rows.iter()
                    .map(|r| {
                        vec![
                            format!("#{}{}", r.id, if r.voided { "×" } else { "" }),
                            r.task.to_string(),
                            ui::fmt_time(r.at),
                            r.minutes.to_string(),
                            r.completed.clone(),
                            r.learned.clone(),
                        ]
                    })
                    .collect(),
                &[10, 8, 12, 8, 35, 35],
            );
            Ok(response(json!(rows), text, false))
        }
        WorkCommand::Summary { task } => {
            let t = s.task(task)?;
            let rows: Vec<_> = s
                .sessions
                .iter()
                .filter(|r| r.task == task && !r.voided)
                .collect();
            let minutes: u64 = rows.iter().map(|r| u64::from(r.minutes)).sum();
            let (i, o) = s.task_usage(task, now);
            let mut text = format!(
                "#{} {}\n累计工作 {} 分钟；实际输入 {} / 输出 {} token\n",
                t.id, t.title, minutes, i, o
            );
            for r in &rows {
                text.push_str(&format!(
                    "\n{} · {} 分钟\n完成：{}\n收获：{}\n下一步：{}\n",
                    ui::fmt_time(r.at),
                    r.minutes,
                    r.completed,
                    r.learned,
                    r.next_steps
                ));
            }
            Ok(response(
                json!({"task":t,"minutes":minutes,"input":i,"output":o,"sessions":rows}),
                text,
                false,
            ))
        }
        WorkCommand::Void { id } => {
            let r = s
                .sessions
                .iter_mut()
                .find(|r| r.id == id)
                .context("复盘记录不存在")?;
            ensure!(!r.voided, "已撤销");
            r.voided = true;
            if let Some(e) = r.usage_event {
                s.events.iter_mut().find(|x| x.id == e).unwrap().voided = true;
            }
            Ok(ok(
                format!("已撤销复盘 #{id} 及关联用量；手工进度不变"),
                Some(id),
            ))
        }
    }
}
pub fn project(c: ProjectCommand, s: &mut State, store: &Store) -> Result<Response> {
    let mut local = LocalSettings::load(store)?;
    match c {
        ProjectCommand::Register {
            name,
            path,
            mode,
            remote,
            description,
        } => {
            workspace::register(
                s,
                &mut local,
                name.clone(),
                &path,
                mode,
                remote,
                description,
            )?;
            s.validate()?;
            local.save(store)?;
            Ok(ok(
                format!(
                    "已注册 {name}；本机路径只保存在 local.json。使用 project status 汇总查看。"
                ),
                None,
            ))
        }
        ProjectCommand::Bind { name, path } => {
            ensure!(s.projects.iter().any(|p| p.name == name), "项目未注册");
            let root = workspace::git(&path, &["rev-parse", "--show-toplevel"])?;
            local.paths.insert(name, fs::canonicalize(root)?);
            local.save(store)?;
            Ok(response(
                json!({"bound":true}),
                "已绑定本机路径；未修改项目代码".into(),
                false,
            ))
        }
        ProjectCommand::Checkout { name, to } => {
            workspace::checkout(s, &mut local, &name, &to)?;
            local.save(store)?;
            Ok(response(
                json!({"path":to}),
                format!("已检出并绑定 {}", to.display()),
                false,
            ))
        }
        ProjectCommand::Status => {
            let statuses = workspace::statuses(s, &local);
            let text = ui::table(
                &[
                    "项目",
                    "模式",
                    "分支",
                    "HEAD",
                    "改动文件",
                    "活动任务",
                    "提示",
                ],
                statuses
                    .iter()
                    .map(|p| {
                        vec![
                            p.name.clone(),
                            format!("{:?}", p.mode),
                            p.branch.clone(),
                            p.head.clone(),
                            p.changes.len().to_string(),
                            p.tasks.to_string(),
                            p.error.clone().unwrap_or_default(),
                        ]
                    })
                    .collect(),
                &[20, 10, 20, 12, 10, 10, 45],
            );
            Ok(response(json!(statuses), text, false))
        }
        ProjectCommand::Diff { name } => {
            let path = local.paths.get(&name).context("本机未绑定项目")?;
            let status =
                workspace::git(path, &["-c", "core.quotepath=false", "status", "--short"])?;
            let unstaged = workspace::git(path, &["diff", "--stat"])?;
            let staged = workspace::git(path, &["diff", "--cached", "--stat"])?;
            Ok(response(
                json!({"status":status,"unstaged":unstaged,"staged":staged}),
                format!("{name}\n{status}\n未暂存：\n{unstaged}\n已暂存：\n{staged}"),
                false,
            ))
        }
        ProjectCommand::Note {
            name,
            context,
            memory,
            context_file,
            memory_file,
        } => {
            let p = s
                .projects
                .iter_mut()
                .find(|p| p.name == name)
                .context("项目未注册")?;
            if let Some(v) = context {
                p.context = v;
            }
            if let Some(v) = memory {
                p.memory = v;
            }
            if let Some(f) = context_file {
                p.context = fs::read_to_string(f)?;
            }
            if let Some(f) = memory_file {
                p.memory = fs::read_to_string(f)?;
            }
            Ok(ok(
                "项目上下文/长期记忆已更新；hub push 后同步".into(),
                None,
            ))
        }
        ProjectCommand::Snapshot { name } => {
            let snap = workspace::snapshot(s, &local, &name)?;
            Ok(response(
                json!(snap),
                format!(
                    "已生成项目快照：{} bytes，提交 {}。使用 hub push 上传。",
                    snap.bytes, snap.commit
                ),
                false,
            ))
        }
        ProjectCommand::Show { name } => {
            let p = s
                .projects
                .iter()
                .find(|p| p.name == name)
                .context("项目未注册")?;
            let tasks: Vec<_> = s.tasks.iter().filter(|t| t.project == name).collect();
            Ok(response(
                json!({"project":p,"tasks":tasks}),
                format!(
                    "{} · {:?}\n原代码仓库：{}\n上下文：\n{}\n长期记忆：\n{}\n关联任务：{}",
                    p.name,
                    p.mode,
                    p.remote.as_deref().unwrap_or("无，使用快照"),
                    p.context,
                    p.memory,
                    tasks
                        .iter()
                        .map(|t| format!("#{} {}", t.id, t.title))
                        .collect::<Vec<_>>()
                        .join("；")
                ),
                false,
            ))
        }
    }
}
pub fn hub(c: HubCommand, s: &mut State, store: &Store) -> Result<Response> {
    let mut local = LocalSettings::load(store)?;
    match c {
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
                    "GitHub 账号格式无效"
                );
                workspace::slug(&repo)?;
                Some(format!("https://github.com/{owner}/{repo}.git"))
            } else {
                remote
            };
            let path = workspace::init_hub(&path, remote.as_deref())?;
            local.hub = Some(path.clone());
            local.base_state_hash = None;
            local.save(store)?;
            Ok(response(
                json!({"path":path,"remote":remote}),
                format!(
                    "工作空间已初始化：{}。账号认证复用 Git；远程仓库需预先创建为私有。hub push 才会上传。",
                    path.display()
                ),
                false,
            ))
        }
        HubCommand::Clone { remote, path } => {
            let path = workspace::clone_hub(&remote, &path)?;
            local.hub = Some(path.clone());
            local.base_state_hash = None;
            local.save(store)?;
            Ok(response(
                json!({"path":path}),
                "已连接工作空间；运行 hub pull 导入任务，再 project bind/checkout 关联本机代码"
                    .into(),
                false,
            ))
        }
        HubCommand::Remote { url } => {
            let h = local.hub()?;
            workspace::check_hub(h)?;
            let command = if workspace::hub_remote(h)?.is_some() {
                "set-url"
            } else {
                "add"
            };
            workspace::git(h, &["remote", command, "origin", &url])?;
            Ok(response(
                json!({"remote":url}),
                "工作空间远程已配置".into(),
                false,
            ))
        }
        HubCommand::Status => {
            let h = local.hub()?;
            workspace::check_hub(h)?;
            let changed = local.base_state_hash.as_ref() != Some(&workspace::state_hash(s)?);
            let changes = workspace::git(h, &["status", "--short"])?;
            Ok(response(
                json!({"hub":h,"local_changed":changed,"git_changes":changes,"remote":workspace::hub_remote(h)?}),
                format!(
                    "工作空间：{}\n本机任务数据{}\n镜像文件更改：\n{}\n此命令未访问网络。",
                    h.display(),
                    if changed {
                        "有未同步更改"
                    } else {
                        "与上次同步一致"
                    },
                    changes
                ),
                false,
            ))
        }
        HubCommand::Push => {
            let commit = workspace::push(s, &mut local)?;
            local.save(store)?;
            Ok(response(
                json!({"commit":commit}),
                format!("任务、额度、复盘、项目索引和记忆已同步：{commit}"),
                false,
            ))
        }
        HubCommand::Pull { replace } => {
            let remote = workspace::pull(s, &mut local, replace)?;
            store.save(&remote)?;
            *s = remote;
            local.save(store)?;
            Ok(response(
                json!({"pulled":true}),
                "同步完成；本机路径保持独立。project status 查看需要绑定的项目。".into(),
                false,
            ))
        }
    }
}
