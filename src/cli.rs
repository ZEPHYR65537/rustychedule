use crate::{
    domain::*,
    planner,
    store::{Store, atomic_write},
    ui,
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{Duration, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "tongchou",
    version,
    about = "统筹：任务优先级 × 独立模型额度 × token 预算",
    after_help = "不带子命令显示总览。所有记录保存在本机，不会调用模型或兑换真实重置卡。\n快速体验：tongchou --data .demo init --demo\n帮助示例：tongchou task add --help"
)]
pub struct Cli {
    /// 数据目录（默认使用系统用户数据目录）
    #[arg(long, global = true, env = "TONGCHOU_DATA")]
    pub data: Option<PathBuf>,
    /// 输出机器可读 JSON
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// 初始化；--demo 仅向空目录写入示例
    Init {
        #[arg(long)]
        demo: bool,
        #[arg(long, default_value = "CNY")]
        currency: String,
    },
    /// 总览：任务、每个模型独立额度、重置机会
    Dashboard,
    /// 重要性 × 紧迫性四象限
    Matrix,
    /// 添加、编辑、检索和推进任务
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// 模型配置（每个模型独立管理额度）
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// 模型的额度窗口：固定周期、滚动窗口、手动重置
    Quota {
        #[command(subcommand)]
        command: QuotaCommand,
    },
    /// 给任务分配模型 token 预算
    Budget {
        #[command(subcommand)]
        command: BudgetCommand,
    },
    /// 手动记录、校准及撤销用量
    Usage {
        #[command(subcommand)]
        command: UsageCommand,
    },
    /// Reset 卡库存；不用于随机 Tibo 刷新
    Credit {
        #[command(subcommand)]
        command: CreditCommand,
    },
    /// 登记已发生的刷新（不会操作供应商账户）
    Reset(ResetArgs),
    /// 规划本次工作，--commit 保存 token 预留
    Plan {
        #[arg(long, default_value_t = 240)]
        minutes: u32,
        #[arg(long, default_value_t = 10.0)]
        reserve_percent: f64,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        commit: bool,
        /// 释放本次范围内未用预算并重新规划，支持额度耗尽后的降级
        #[arg(long)]
        rebalance: bool,
    },
    /// 按模型显示近期用量、费用和每日趋势
    Report {
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
    /// 完整导出 JSON 备份
    Export {
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// 导入并校验备份；替换现有数据需 --replace
    Import {
        path: PathBuf,
        #[arg(long)]
        replace: bool,
    },
    /// 校验数据并显示数据位置
    Doctor,
}
#[derive(Subcommand, Debug)]
pub enum TaskCommand {
    Add(TaskAdd),
    Edit(TaskEdit),
    /// 按优先级列出任务，默认只列未结束任务
    List {
        #[arg(long)]
        all: bool,
        #[arg(long, value_enum)]
        status: Option<Status>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        search: Option<String>,
    },
    Show {
        id: u64,
    },
    /// 状态：todo / doing / blocked / done / cancelled；结束时释放预算
    Status {
        id: u64,
        #[arg(value_enum)]
        status: Status,
        #[arg(long)]
        note: Option<String>,
    },
}
#[derive(Args, Debug)]
pub struct TaskAdd {
    pub title: String,
    #[arg(short = 'i', long, default_value_t = 3)]
    pub importance: u8,
    #[arg(short = 'u', long, default_value_t = 3)]
    pub urgency: u8,
    /// 截止日期；纯日期按本地当天 23:59:59
    #[arg(long)]
    pub due: Option<String>,
    /// 本次剩余工作分钟数
    #[arg(long, default_value_t = 30)]
    pub minutes: u32,
    /// 全任务预计输入 token（包括缓存输入）
    #[arg(long, default_value_t = 0)]
    pub input: u64,
    #[arg(long, default_value_t = 0)]
    pub output: u64,
    #[arg(long, default_value_t = 1)]
    pub capability: u8,
    #[arg(long, default_value = "")]
    pub project: String,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub depends: Vec<u64>,
    /// 允许的模型集合；省略表示所有模型
    #[arg(long, value_delimiter = ',')]
    pub models: Vec<String>,
    /// 偏序边，可重复：--prefer 'A>B' --prefer 'A>C'
    #[arg(long)]
    pub prefer: Vec<String>,
    /// 相对基准 token 估计的模型倍率，可重复：--factor fast=1.5
    #[arg(long)]
    pub factor: Vec<String>,
    #[arg(long, default_value_t = 0.0)]
    pub progress: f64,
    /// 必须让同一个模型完成剩余工作，禁止自动分段
    #[arg(long)]
    pub no_split: bool,
    #[arg(long, default_value = "")]
    pub note: String,
}
#[derive(Args, Debug)]
pub struct TaskEdit {
    pub id: u64,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub importance: Option<u8>,
    #[arg(long)]
    pub urgency: Option<u8>,
    #[arg(long, conflicts_with = "clear_due")]
    pub due: Option<String>,
    #[arg(long)]
    pub clear_due: bool,
    #[arg(long)]
    pub minutes: Option<u32>,
    #[arg(long)]
    pub input: Option<u64>,
    #[arg(long)]
    pub output: Option<u64>,
    #[arg(long)]
    pub capability: Option<u8>,
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub tags: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',', conflicts_with = "clear_depends")]
    pub depends: Option<Vec<u64>>,
    #[arg(long)]
    pub clear_depends: bool,
    #[arg(long, value_delimiter = ',', conflicts_with = "all_models")]
    pub models: Option<Vec<String>>,
    #[arg(long)]
    pub all_models: bool,
    /// 替换全部偏序边
    #[arg(long, conflicts_with = "clear_preferences")]
    pub prefer: Vec<String>,
    #[arg(long)]
    pub clear_preferences: bool,
    #[arg(long)]
    pub note: Option<String>,
    #[arg(long)]
    pub progress: Option<f64>,
    /// 替换所有模型倍率；未指定的模型按 1.0
    #[arg(long, conflicts_with = "clear_factors")]
    pub factor: Vec<String>,
    #[arg(long)]
    pub clear_factors: bool,
    #[arg(long,action=clap::ArgAction::Set)]
    pub splittable: Option<bool>,
}
#[derive(Subcommand, Debug)]
pub enum ModelCommand {
    Add(ModelAdd),
    Edit(ModelEdit),
    List,
}
#[derive(Args, Debug)]
pub struct ModelAdd {
    pub name: String,
    #[arg(long, default_value = "custom")]
    pub provider: String,
    #[arg(long, default_value_t = 3)]
    pub capability: u8,
    /// 每百万输入 token 的价格，使用数据文件的统一货币
    #[arg(long, default_value_t = 0.0)]
    pub input_price: f64,
    #[arg(long, default_value_t = 0.0)]
    pub output_price: f64,
    #[arg(long, default_value_t = 0.0)]
    pub cached_price: f64,
    /// 额度单位标签；默认为 token，可用 units/points 等
    #[arg(long, default_value = "token")]
    pub unit: String,
    /// 每个输入 token 折算为多少本模型额度单位
    #[arg(long, default_value_t = 1.0)]
    pub input_weight: f64,
    #[arg(long, default_value_t = 1.0)]
    pub output_weight: f64,
}
#[derive(Args, Debug)]
pub struct ModelEdit {
    pub name: String,
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub capability: Option<u8>,
    #[arg(long)]
    pub input_price: Option<f64>,
    #[arg(long)]
    pub output_price: Option<f64>,
    #[arg(long)]
    pub cached_price: Option<f64>,
    /// 显式 true/false；停用后保留历史
    #[arg(long,action=clap::ArgAction::Set)]
    pub enabled: Option<bool>,
    #[arg(long)]
    pub input_weight: Option<f64>,
    #[arg(long)]
    pub output_weight: Option<f64>,
}
#[derive(Subcommand, Debug)]
pub enum QuotaCommand {
    /// 为模型添加一个额度限制窗口
    Add {
        model: String,
        name: String,
        #[arg(long)]
        limit: f64,
        #[arg(long, value_enum, default_value = "fixed")]
        kind: WindowKind,
        #[arg(long)]
        period: Option<String>,
        /// 固定周期下一次自然重置的准确时间
        #[arg(long)]
        next_reset: Option<String>,
    },
    /// 调整限额，不修改历史用量
    Limit {
        model: String,
        window: String,
        limit: f64,
    },
    List {
        #[arg(long)]
        model: Option<String>,
    },
}
#[derive(Subcommand, Debug)]
pub enum BudgetCommand {
    /// 设置该任务/模型的剩余预算，替换旧预留
    Set {
        task: u64,
        model: String,
        #[arg(long, default_value_t = 0)]
        input: u64,
        #[arg(long, default_value_t = 0)]
        output: u64,
        #[arg(long)]
        force: bool,
    },
    List {
        #[arg(long)]
        task: Option<u64>,
    },
    Release {
        task: u64,
        #[arg(long)]
        model: Option<String>,
    },
}
#[derive(Subcommand, Debug)]
pub enum UsageCommand {
    Log {
        model: String,
        #[arg(long)]
        task: Option<u64>,
        #[arg(long, default_value_t = 0)]
        input: u64,
        #[arg(long, default_value_t = 0)]
        output: u64,
        /// 缓存输入是 input 的子集，不重复计入总 token
        #[arg(long, default_value_t = 0)]
        cached: u64,
        /// 覆盖本条消耗的模型额度单位；不影响原始 token 数
        #[arg(long)]
        units: Option<f64>,
        #[arg(long)]
        cost: Option<f64>,
        #[arg(long)]
        at: Option<String>,
        /// 同时更新任务的绝对完成百分比；不根据 token 消耗自动推断
        #[arg(long, requires = "task")]
        progress: Option<f64>,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// 用供应商面板的已用数/百分比校准一个窗口
    Reconcile {
        model: String,
        window: String,
        #[arg(long, required_unless_present = "percent", conflicts_with = "percent")]
        used: Option<f64>,
        #[arg(long)]
        percent: Option<f64>,
        #[arg(long, default_value = "")]
        note: String,
    },
    List {
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
    /// 撤销错误的用量或校准记录；保留审计记录
    Void { id: u64 },
}
#[derive(Subcommand, Debug)]
pub enum CreditCommand {
    Add {
        #[arg(long, default_value_t = 1)]
        count: u32,
        #[arg(long, value_enum, default_value = "card")]
        kind: CreditKind,
        /// 适用模型；不填表示任何模型（仍分别重置）
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        windows: Vec<String>,
        #[arg(long)]
        expires: Option<String>,
        #[arg(long, default_value = "")]
        note: String,
    },
    List,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ResetSource {
    Tibo,
    Manual,
}
#[derive(Args, Debug)]
pub struct ResetArgs {
    /// 分别刷新这些模型，不合并它们的额度
    #[arg(long, value_delimiter = ',', required = true)]
    pub models: Vec<String>,
    /// 不填表示选定模型的所有窗口
    #[arg(long, value_delimiter = ',')]
    pub windows: Vec<String>,
    /// 登记一次已使用的 reset 卡（一次操作扣一次库存）
    #[arg(long, conflicts_with = "source")]
    pub credit: Option<u64>,
    /// 无卡来源：随机 Tibo 自动刷新或其他手动记录
    #[arg(long, value_enum, required_unless_present = "credit")]
    pub source: Option<ResetSource>,
    /// 同时把固定周期下一次自然重置改为“现在 + 周期”
    #[arg(long)]
    pub restart_clock: bool,
    #[arg(long, default_value = "")]
    pub note: String,
}

pub struct Response {
    pub value: Value,
    pub text: String,
    pub changed: bool,
}
fn response(value: Value, text: String, changed: bool) -> Response {
    Response {
        value,
        text,
        changed,
    }
}
fn ok(message: String, id: Option<u64>) -> Response {
    response(json!({"message":message,"id":id}), message, true)
}
fn preference_edges(values: &[String]) -> Result<Vec<[String; 2]>> {
    values
        .iter()
        .map(|s| {
            let (a, b) = s
                .split_once('>')
                .context("偏好格式须为 '模型A>模型B'，请用引号包住")?;
            ensure!(!b.contains('>'), "每项只写一条边；链式偏好请重复 --prefer");
            Ok([a.trim().into(), b.trim().into()])
        })
        .collect()
}
fn is_empty(s: &State) -> bool {
    s.tasks.is_empty()
        && s.models.is_empty()
        && s.events.is_empty()
        && s.credits.is_empty()
        && s.pools.is_empty()
        && s.budgets.is_empty()
}
fn factors(values: &[String]) -> Result<BTreeMap<String, f64>> {
    values
        .iter()
        .map(|s| {
            let (m, f) = s.split_once('=').context("倍率格式为 model=1.5")?;
            Ok((m.into(), f.parse().context("倍率必须是数字")?))
        })
        .collect()
}

pub fn execute(command: Command, s: &mut State, store: &Store, now: Time) -> Result<Response> {
    match command {
        Command::Init { demo, currency } => {
            ensure!(
                is_empty(s),
                "数据非空，init 不会覆盖现有数据；体验示例请指定新的 --data 目录"
            );
            s.currency = currency;
            if demo {
                *s = demo_state(now, &s.currency)?;
            }
            Ok(ok(
                format!(
                    "已初始化 {}{}",
                    store.dir.display(),
                    if demo {
                        "（示例数据，所有价格和额度均为虚构）"
                    } else {
                        ""
                    }
                ),
                None,
            ))
        }
        Command::Dashboard => Ok(response(
            json!({"at":now,"tasks":s.tasks,"models":s.models,"quotas":s.quotas(now),"credits":s.credits}),
            ui::dashboard(s, now),
            false,
        )),
        Command::Matrix => Ok(response(
            json!({"at":now,"quadrants":(0..4).map(|q|s.tasks.iter().filter(|t|t.status.active() && t.quadrant(now)==q).collect::<Vec<_>>()).collect::<Vec<_>>()}),
            ui::matrix(s, now),
            false,
        )),
        Command::Task { command } => task_command(command, s, now),
        Command::Model { command } => model_command(command, s),
        Command::Quota { command } => quota_command(command, s, now),
        Command::Budget { command } => match command {
            BudgetCommand::Set {
                task,
                model,
                input,
                output,
                force,
            } => {
                let id = s.budget_set(task, &model, input, output, now, force)?;
                Ok(ok(
                    format!(
                        "已预留预算 #{id}：任务 #{task} / {model}，输入 {input}、输出 {output} token"
                    ),
                    Some(id),
                ))
            }
            BudgetCommand::Release { task, model } => {
                s.task(task)?;
                if let Some(m) = &model {
                    s.model(m)?;
                }
                for b in &mut s.budgets {
                    if b.task == task && model.as_ref().is_none_or(|m| m == &b.model) {
                        b.released = true;
                    }
                }
                Ok(ok("已释放预留；历史记录保留".into(), None))
            }
            BudgetCommand::List { task } => {
                if let Some(t) = task {
                    s.task(t)?;
                }
                let rows: Vec<_> = s
                    .budgets
                    .iter()
                    .filter(|b| !b.released && task.is_none_or(|t| t == b.task))
                    .map(|b| {
                        let (i, o) = s.remaining_budget(b, now);
                        json!({"id":b.id,"task":b.task,"model":b.model,"input":i,"output":o})
                    })
                    .collect();
                Ok(response(json!(rows), ui::budget_rows(&rows), false))
            }
        },
        Command::Usage { command } => usage_command(command, s, now),
        Command::Credit { command } => match command {
            CreditCommand::Add {
                count,
                kind,
                models,
                windows,
                expires,
                note,
            } => {
                ensure!(count > 0, "重置机会次数须大于 0");
                for m in &models {
                    s.model(m)?;
                }
                let expires = expires.map(|x| parse_time(&x, true)).transpose()?;
                ensure!(expires.is_none_or(|t| t > now), "有效期必须在未来");
                let id = s.id();
                s.credits.push(Credit {
                    id,
                    kind,
                    count,
                    pools: models,
                    windows,
                    expires,
                    note,
                });
                Ok(ok(
                    format!("已登记 reset 卡/重置机会 #{id}，{count} 次"),
                    Some(id),
                ))
            }
            CreditCommand::List => Ok(response(json!(s.credits), ui::credits(s, now), false)),
        },
        Command::Reset(args) => {
            let mut targets = BTreeMap::new();
            for model in &args.models {
                s.model(model)?;
                let pool = s.pool(model)?;
                targets.insert(
                    model.clone(),
                    if args.windows.is_empty() {
                        pool.windows.iter().map(|w| w.name.clone()).collect()
                    } else {
                        args.windows.clone()
                    },
                );
            }
            let source = match args.source {
                Some(ResetSource::Tibo) => "tibo",
                _ => "manual",
            };
            let id = s.reset(
                targets,
                args.credit,
                args.restart_clock,
                source,
                args.note,
                now,
            )?;
            Ok(ok(
                format!("已记录刷新 #{id}。各模型分别清零已用额度，任务预留仍然保留。"),
                Some(id),
            ))
        }
        Command::Plan {
            minutes,
            reserve_percent,
            project,
            commit,
            rebalance,
        } => {
            let mut working = s.clone();
            if rebalance {
                planner::release_scope(&mut working, project.as_deref());
            }
            let p = planner::build(&working, now, minutes, reserve_percent, project.as_deref())?;
            if commit {
                planner::commit(&mut working, &p)?;
                *s = working;
            }
            let mut text = ui::plan(&p, &s.currency, commit);
            if rebalance {
                text.push_str(
                    "\n本次已重算范围内的所有未用预算；暂缓任务的旧预留也会在提交后释放。\n",
                );
            }
            Ok(response(
                json!({"committed":commit,"rebalanced":rebalance,"plan":p}),
                text,
                commit,
            ))
        }
        Command::Report { days } => {
            ensure!((1..=3660).contains(&days), "统计天数须为 1–3660");
            let v = ui::report_data(s, now, days);
            Ok(response(v.clone(), ui::report(&v, &s.currency), false))
        }
        Command::Export { path, force } => {
            ensure!(
                force || !path.exists(),
                "目标已存在；如需覆盖请使用 --force"
            );
            atomic_write(&path, &serde_json::to_vec_pretty(s)?)?;
            Ok(response(
                json!({"path":path}),
                format!("已导出完整备份到 {}", path.display()),
                false,
            ))
        }
        Command::Import { path, replace } => {
            ensure!(
                replace || is_empty(s),
                "现有数据非空；完整替换请使用 --replace"
            );
            *s = store.import(&path)?;
            Ok(ok(
                "备份已通过校验并导入；可用旧数据备份为 state.json.bak，损坏原件保留为 state.json.corrupt".into(),
                None,
            ))
        }
        Command::Doctor => {
            s.validate()?;
            Ok(response(
                json!({"valid":true,"version":s.version,"data":store.dir,"tasks":s.tasks.len(),"events":s.events.len()}),
                format!(
                    "数据校验通过 · 格式 v{}\n数据目录：{}\n任务 {} 个；模型 {} 个；流水 {} 条。",
                    s.version,
                    store.dir.display(),
                    s.tasks.len(),
                    s.models.len(),
                    s.events.len()
                ),
                false,
            ))
        }
    }
}

fn task_command(command: TaskCommand, s: &mut State, now: Time) -> Result<Response> {
    match command {
        TaskCommand::Add(a) => {
            let due = a.due.map(|d| parse_time(&d, true)).transpose()?;
            let preferences = preference_edges(&a.prefer)?;
            let id = s.id();
            s.tasks.push(Task {
                id,
                title: a.title,
                importance: a.importance,
                urgency: a.urgency,
                due,
                minutes: a.minutes,
                input: a.input,
                output: a.output,
                capability: a.capability,
                status: Status::Todo,
                project: a.project,
                tags: a.tags,
                depends: a.depends,
                allowed_models: a.models,
                preferences,
                progress: a.progress,
                factors: factors(&a.factor)?,
                splittable: !a.no_split,
                note: a.note,
                created: now,
            });
            Ok(ok(format!("已添加任务 #{id}"), Some(id)))
        }
        TaskCommand::Edit(a) => {
            let t = s
                .tasks
                .iter_mut()
                .find(|t| t.id == a.id)
                .context("任务不存在")?;
            if let Some(v) = a.title {
                t.title = v;
            }
            if let Some(v) = a.importance {
                t.importance = v;
            }
            if let Some(v) = a.urgency {
                t.urgency = v;
            }
            if let Some(v) = a.due {
                t.due = Some(parse_time(&v, true)?);
            }
            if a.clear_due {
                t.due = None;
            }
            if let Some(v) = a.minutes {
                t.minutes = v;
            }
            if let Some(v) = a.input {
                t.input = v;
            }
            if let Some(v) = a.output {
                t.output = v;
            }
            if let Some(v) = a.capability {
                t.capability = v;
            }
            if let Some(v) = a.project {
                t.project = v;
            }
            if let Some(v) = a.tags {
                t.tags = v;
            }
            if let Some(v) = a.depends {
                t.depends = v;
            }
            if a.clear_depends {
                t.depends.clear();
            }
            if let Some(v) = a.models {
                t.allowed_models = v;
            }
            if a.all_models {
                t.allowed_models.clear();
            }
            if !a.prefer.is_empty() {
                t.preferences = preference_edges(&a.prefer)?;
            }
            if a.clear_preferences {
                t.preferences.clear();
            }
            if let Some(v) = a.progress {
                t.progress = v;
            }
            if !a.factor.is_empty() {
                t.factors = factors(&a.factor)?;
            }
            if a.clear_factors {
                t.factors.clear();
            }
            if let Some(v) = a.splittable {
                t.splittable = v;
            }
            if let Some(v) = a.note {
                t.note = v;
            }
            Ok(ok(format!("已更新任务 #{}", a.id), Some(a.id)))
        }
        TaskCommand::Status { id, status, note } => {
            let t = s
                .tasks
                .iter_mut()
                .find(|t| t.id == id)
                .context("任务不存在")?;
            t.status = status;
            if let Some(n) = note {
                t.note = n;
            }
            if status == Status::Done {
                t.progress = 100.0;
            }
            if !status.active() {
                for b in &mut s.budgets {
                    if b.task == id {
                        b.released = true;
                    }
                }
            }
            Ok(ok(format!("任务 #{id} → {}", status.label()), Some(id)))
        }
        TaskCommand::List {
            all,
            status,
            project,
            tag,
            search,
        } => {
            let mut tasks: Vec<_> = s
                .tasks
                .iter()
                .filter(|t| {
                    (all || status.is_some() || t.status.active())
                        && status.is_none_or(|v| t.status == v)
                        && project.as_ref().is_none_or(|v| v == &t.project)
                        && tag.as_ref().is_none_or(|v| t.tags.contains(v))
                        && search.as_ref().is_none_or(|v| {
                            format!("{} {}", t.title, t.note)
                                .to_lowercase()
                                .contains(&v.to_lowercase())
                        })
                })
                .collect();
            tasks.sort_by_key(|t| (std::cmp::Reverse(t.score(now)), t.id));
            Ok(response(json!(tasks), ui::task_table(&tasks, now), false))
        }
        TaskCommand::Show { id } => {
            let t = s.task(id)?;
            let (i, o, c) = s.task_usage(id, now);
            Ok(response(
                json!({"task":t,"actual_input":i,"actual_output":o,"actual_cost":c}),
                ui::task_detail(s, t, now),
                false,
            ))
        }
    }
}
fn model_command(command: ModelCommand, s: &mut State) -> Result<Response> {
    match command {
        ModelCommand::Add(a) => {
            ensure!(s.models.iter().all(|m| m.name != a.name), "模型名称已存在");
            s.pools.push(Pool {
                name: a.name.clone(),
                unit: a.unit,
                windows: vec![],
            });
            s.models.push(Model {
                name: a.name.clone(),
                provider: a.provider,
                capability: a.capability,
                enabled: true,
                input_price: a.input_price,
                output_price: a.output_price,
                cached_price: a.cached_price,
                bindings: vec![Binding {
                    pool: a.name.clone(),
                    input_weight: a.input_weight,
                    output_weight: a.output_weight,
                }],
            });
            Ok(ok(
                format!(
                    "已添加模型 {}；请用 quota add 配置其独立额度窗口（未设置时视为不受限）",
                    a.name
                ),
                None,
            ))
        }
        ModelCommand::Edit(a) => {
            let m = s
                .models
                .iter_mut()
                .find(|m| m.name == a.name)
                .context("模型不存在")?;
            if let Some(v) = a.provider {
                m.provider = v;
            }
            if let Some(v) = a.capability {
                m.capability = v;
            }
            if let Some(v) = a.input_price {
                m.input_price = v;
            }
            if let Some(v) = a.output_price {
                m.output_price = v;
            }
            if let Some(v) = a.cached_price {
                m.cached_price = v;
            }
            if let Some(v) = a.enabled {
                m.enabled = v;
            }
            if let Some(v) = a.input_weight {
                m.bindings[0].input_weight = v;
            }
            if let Some(v) = a.output_weight {
                m.bindings[0].output_weight = v;
            }
            Ok(ok(
                "模型已更新；历史用量及当时费用不变，当前预算按新权重折算".into(),
                None,
            ))
        }
        ModelCommand::List => Ok(response(json!(s.models), ui::models(s), false)),
    }
}
fn quota_command(command: QuotaCommand, s: &mut State, now: Time) -> Result<Response> {
    match command {
        QuotaCommand::Add {
            model,
            name,
            limit,
            kind,
            period,
            next_reset,
        } => {
            s.model(&model)?;
            let seconds = if kind == WindowKind::Manual {
                ensure!(
                    period.is_none() && next_reset.is_none(),
                    "manual 窗口不接受周期或下一次重置时间"
                );
                0
            } else {
                parse_duration(
                    period
                        .as_deref()
                        .context("fixed/rolling 窗口必须设置 --period，例如 5h 或 7d")?,
                )?
            };
            ensure!(
                kind == WindowKind::Fixed || next_reset.is_none(),
                "只有 fixed 窗口接受 --next-reset"
            );
            let anchor = next_reset
                .map(|x| parse_time(&x, false))
                .transpose()?
                .map_or(now, |d| d - Duration::seconds(seconds));
            let pool = s.pools.iter_mut().find(|p| p.name == model).unwrap();
            ensure!(
                pool.windows.iter().all(|w| w.name != name),
                "窗口名称已存在"
            );
            pool.windows.push(Window {
                name: name.clone(),
                kind,
                limit,
                seconds,
                anchor,
            });
            Ok(ok(format!("已为 {model} 添加独立窗口 {name}"), None))
        }
        QuotaCommand::Limit {
            model,
            window,
            limit,
        } => {
            positive(limit)?;
            let w = s
                .pools
                .iter_mut()
                .find(|p| p.name == model)
                .context("模型不存在")?
                .windows
                .iter_mut()
                .find(|w| w.name == window)
                .context("窗口不存在")?;
            w.limit = limit;
            Ok(ok(format!("{model}/{window} 限额已改为 {limit}"), None))
        }
        QuotaCommand::List { model } => {
            if let Some(m) = &model {
                s.model(m)?;
            }
            let quotas: Vec<_> = s
                .quotas(now)
                .into_iter()
                .filter(|q| model.as_ref().is_none_or(|m| m == &q.pool))
                .collect();
            Ok(response(
                json!(quotas),
                ui::quota_table(&quotas, now),
                false,
            ))
        }
    }
}
fn usage_command(command: UsageCommand, s: &mut State, now: Time) -> Result<Response> {
    match command {
        UsageCommand::Log {
            model,
            task,
            input,
            output,
            cached,
            units,
            cost,
            at,
            progress,
            note,
        } => {
            tokens(input, output)?;
            ensure!(cached <= input, "缓存输入不能超过总输入");
            if let Some(t) = task {
                s.task(t)?;
            }
            let m = s.model(&model)?;
            let amount = units.unwrap_or_else(|| m.bindings[0].units(input, output));
            nonnegative(amount)?;
            let cost = cost.unwrap_or_else(|| m.cost(input, output, cached));
            nonnegative(cost)?;
            ensure!(
                input > 0 || output > 0 || amount > 0.0 || cost > 0.0,
                "空用量记录无须保存"
            );
            let at = at
                .map(|x| parse_time(&x, false))
                .transpose()?
                .unwrap_or(now);
            ensure!(at <= now, "不能记录未来用量");
            let id = s.id();
            s.events.push(Event {
                id,
                at,
                note,
                voided: false,
                kind: EventKind::Usage {
                    task,
                    model: model.clone(),
                    input,
                    output,
                    cached,
                    cost,
                    units: BTreeMap::from([(model.clone(), amount)]),
                },
            });
            if let Some(p) = progress {
                let id = task.context("更新进度需要 --task")?;
                s.tasks.iter_mut().find(|t| t.id == id).unwrap().progress = p;
            }
            let warnings: Vec<_> = s
                .quotas(now)
                .into_iter()
                .filter(|q| q.pool == model && q.used + q.reserved > q.limit)
                .map(|q| format!("{}/{} 用量+预留已超限", q.pool, q.window))
                .collect();
            Ok(response(
                json!({"id":id,"cost":cost,"warnings":warnings}),
                format!(
                    "已记用量 #{id}：{model}，输入 {input} / 输出 {output}，费用 {cost:.4} {}{}",
                    s.currency,
                    if warnings.is_empty() {
                        String::new()
                    } else {
                        format!("\n注意：{}", warnings.join("；"))
                    }
                ),
                true,
            ))
        }
        UsageCommand::Reconcile {
            model,
            window,
            used,
            percent,
            note,
        } => {
            let w = s
                .pool(&model)?
                .windows
                .iter()
                .find(|w| w.name == window)
                .context("窗口不存在")?;
            let used = if let Some(p) = percent {
                ensure!(
                    p.is_finite() && (0.0..=100.0).contains(&p),
                    "百分比须在 0–100 内"
                );
                w.limit * p / 100.0
            } else {
                used.context("请指定 --used 或 --percent")?
            };
            nonnegative(used)?;
            let rolling = w.kind == WindowKind::Rolling;
            let id = s.id();
            s.events.push(Event {
                id,
                at: now,
                note,
                voided: false,
                kind: EventKind::Snapshot {
                    pool: model,
                    window,
                    used,
                },
            });
            Ok(ok(
                format!(
                    "已校准窗口已用额度为 {used:.2}{}",
                    if rolling {
                        "；滚动窗口按整批在一个周期后到期作保守估计"
                    } else {
                        ""
                    }
                ),
                Some(id),
            ))
        }
        UsageCommand::List { model, last } => {
            if let Some(m) = &model {
                s.model(m)?;
            }
            let events: Vec<_> = s
                .events
                .iter()
                .rev()
                .filter(|e| {
                    model.as_ref().is_none_or(|m| match &e.kind {
                        EventKind::Usage { model, .. } => m == model,
                        EventKind::Snapshot { pool, .. } => m == pool,
                        EventKind::Reset { targets, .. } => targets.contains_key(m),
                    })
                })
                .take(last)
                .collect();
            Ok(response(json!(events), ui::events(&events), false))
        }
        UsageCommand::Void { id } => {
            let e = s
                .events
                .iter_mut()
                .find(|e| e.id == id)
                .context("流水不存在")?;
            ensure!(
                !matches!(e.kind, EventKind::Reset { .. }),
                "不能撤销刷新流水；请用 usage reconcile 校准实际用量"
            );
            ensure!(!e.voided, "该流水已撤销");
            e.voided = true;
            Ok(ok(
                format!("已撤销流水 #{id}，额度、预算和统计会重新计算；手动登记的任务进度保持不变"),
                Some(id),
            ))
        }
    }
}

pub fn demo_state(now: Time, currency: &str) -> Result<State> {
    let mut s = State {
        currency: currency.into(),
        ..State::default()
    };
    for (name, cap, limit, ip, op) in [
        ("deep", 5, 120_000.0, 12.0, 48.0),
        ("fast", 3, 240_000.0, 1.0, 4.0),
        ("local", 2, 500_000.0, 0.0, 0.0),
    ] {
        s.models.push(Model {
            name: name.into(),
            provider: "示例（非真实套餐）".into(),
            capability: cap,
            enabled: true,
            input_price: ip,
            output_price: op,
            cached_price: ip / 4.0,
            bindings: vec![Binding {
                pool: name.into(),
                input_weight: 1.0,
                output_weight: 1.0,
            }],
        });
        s.pools.push(Pool {
            name: name.into(),
            unit: "token".into(),
            windows: vec![
                Window {
                    name: "5h".into(),
                    kind: WindowKind::Fixed,
                    limit,
                    seconds: 18_000,
                    anchor: now - Duration::hours(2),
                },
                Window {
                    name: "week".into(),
                    kind: WindowKind::Fixed,
                    limit: limit * 5.0,
                    seconds: 604_800,
                    anchor: now - Duration::days(3),
                },
            ],
        });
    }
    for (title, i, u, minutes, input, output, cap, due) in [
        (
            "修复支付回调的重复写入",
            5,
            5,
            60,
            18000,
            6000,
            5,
            Some(now + Duration::hours(4)),
        ),
        (
            "设计下周研究实验",
            5,
            2,
            90,
            24000,
            10000,
            3,
            Some(now + Duration::days(8)),
        ),
        (
            "整理访谈笔记",
            3,
            4,
            30,
            12000,
            3000,
            2,
            Some(now + Duration::days(2)),
        ),
        (
            "给实验设计补充对照组",
            4,
            2,
            45,
            10000,
            4000,
            3,
            Some(now + Duration::days(9)),
        ),
        ("清理旧资料目录", 2, 1, 20, 0, 0, 1, None),
    ] {
        let id = s.id();
        s.tasks.push(Task {
            id,
            title: title.into(),
            importance: i,
            urgency: u,
            due,
            minutes,
            input,
            output,
            capability: cap,
            status: Status::Todo,
            project: "示例项目".into(),
            tags: vec!["demo".into()],
            depends: if id == 4 { vec![2] } else { vec![] },
            allowed_models: vec![],
            preferences: vec![
                ["deep".into(), "fast".into()],
                ["fast".into(), "local".into()],
            ],
            progress: 0.0,
            factors: BTreeMap::from([("fast".into(), 1.25), ("local".into(), 1.8)]),
            splittable: true,
            note: "演示数据，可另选目录开始正式记录".into(),
            created: now,
        });
    }
    let id = s.id();
    s.events.push(Event {
        id,
        at: now - Duration::minutes(30),
        note: "之前的实际用量".into(),
        voided: false,
        kind: EventKind::Usage {
            task: None,
            model: "deep".into(),
            input: 60_000,
            output: 24_000,
            cached: 0,
            cost: 1.872,
            units: BTreeMap::from([("deep".into(), 84_000.0)]),
        },
    });
    let id = s.id();
    s.credits.push(Credit {
        id,
        kind: CreditKind::Card,
        count: 2,
        pools: vec!["deep".into()],
        windows: vec![],
        expires: Some(now + Duration::days(5)),
        note: "示例 reset 卡".into(),
    });
    s.validate()?;
    Ok(s)
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let json_output = cli.json;
    let result = (|| -> Result<Response> {
        let store = Store::open(cli.data.unwrap_or_else(crate::store::default_dir))?;
        // Import is the explicit recovery path and must work even if the live file is corrupt.
        let command = cli.command.unwrap_or(Command::Dashboard);
        let mut state = if matches!(command, Command::Import { replace: true, .. }) {
            State::default()
        } else {
            store.load()?
        };
        let response = execute(command, &mut state, &store, Utc::now())?;
        if response.changed {
            store.save(&state)?;
        }
        Ok(response)
    })();
    match result {
        Ok(r) => {
            use std::io::Write;
            let output = if json_output {
                serde_json::to_string_pretty(&r.value)?
            } else {
                r.text
            };
            if let Err(e) = writeln!(std::io::stdout().lock(), "{output}") {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(e.into());
                }
            }
            Ok(())
        }
        Err(e) => {
            if json_output {
                eprintln!("{}", json!({"error":format!("{e:#}")}));
            } else {
                eprintln!("错误：{e:#}");
            }
            bail!("__reported__")
        }
    }
}
