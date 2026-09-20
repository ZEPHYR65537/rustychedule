use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Time = DateTime<Utc>;
pub const MAX_TOKENS: u64 = 1_000_000_000_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Todo,
    Doing,
    Blocked,
    Done,
    Cancelled,
}
impl Status {
    pub fn active(self) -> bool {
        !matches!(self, Self::Done | Self::Cancelled)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "待办",
            Self::Doing => "进行中",
            Self::Blocked => "阻塞",
            Self::Done => "完成",
            Self::Cancelled => "取消",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub importance: u8,
    pub urgency: u8,
    pub due: Option<Time>,
    pub minutes: u32,
    pub input: u64,
    pub output: u64,
    pub capability: u8,
    pub status: Status,
    pub project: String,
    pub tags: Vec<String>,
    pub depends: Vec<u64>,
    pub allowed_models: Vec<String>,
    pub preferences: Vec<[String; 2]>,
    pub progress: f64,
    pub factors: BTreeMap<String, f64>,
    pub splittable: bool,
    pub note: String,
    pub created: Time,
}
impl Task {
    pub fn urgency_at(&self, now: Time) -> u8 {
        self.urgency.max(self.due.map_or(1, |due| {
            let remaining = due - now;
            if remaining <= Duration::hours(24) {
                5
            } else if remaining <= Duration::hours(72) {
                4
            } else if remaining <= Duration::hours(168) {
                3
            } else {
                1
            }
        }))
    }
    pub fn quadrant(&self, now: Time) -> usize {
        match (self.importance >= 4, self.urgency_at(now) >= 4) {
            (true, true) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 3,
        }
    }
    pub fn score(&self, now: Time) -> u32 {
        let overdue = self.due.is_some_and(|d| d < now);
        u32::from(self.importance) * 12
            + u32::from(self.urgency_at(now)) * 8
            + if overdue { 15 } else { 0 }
            + if self.status == Status::Doing { 5 } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WindowKind {
    Fixed,
    Rolling,
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Window {
    pub name: String,
    pub kind: WindowKind,
    pub limit: f64,
    pub seconds: i64,
    pub anchor: Time,
}
impl Window {
    pub fn bounds(&self, now: Time) -> (Option<Time>, Option<Time>) {
        match self.kind {
            WindowKind::Manual => (None, None),
            WindowKind::Rolling => (Some(now - Duration::seconds(self.seconds)), None),
            WindowKind::Fixed => {
                let delta = now.signed_duration_since(self.anchor).num_milliseconds();
                let cycles = delta.div_euclid(self.seconds * 1000);
                let start = self.anchor + Duration::seconds(cycles * self.seconds);
                (Some(start), Some(start + Duration::seconds(self.seconds)))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pool {
    pub name: String,
    pub unit: String,
    pub windows: Vec<Window>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub pool: String,
    pub input_weight: f64,
    pub output_weight: f64,
}
impl Binding {
    pub fn units(&self, input: u64, output: u64) -> f64 {
        input as f64 * self.input_weight + output as f64 * self.output_weight
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub provider: String,
    pub capability: u8,
    pub enabled: bool,
    pub input_price: f64,
    pub output_price: f64,
    pub cached_price: f64,
    pub bindings: Vec<Binding>,
}
impl Model {
    pub fn cost(&self, input: u64, output: u64, cached: u64) -> f64 {
        ((input - cached) as f64 * self.input_price
            + output as f64 * self.output_price
            + cached as f64 * self.cached_price)
            / 1_000_000.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budget {
    pub id: u64,
    pub task: u64,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub created: Time,
    pub released: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CreditKind {
    Card,
    Custom,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credit {
    pub id: u64,
    pub kind: CreditKind,
    pub count: u32,
    pub pools: Vec<String>,
    pub windows: Vec<String>,
    pub expires: Option<Time>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    pub at: Time,
    pub note: String,
    pub voided: bool,
    #[serde(flatten)]
    pub kind: EventKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum EventKind {
    Usage {
        task: Option<u64>,
        model: String,
        input: u64,
        output: u64,
        cached: u64,
        cost: f64,
        units: BTreeMap<String, f64>,
    },
    Snapshot {
        pool: String,
        window: String,
        used: f64,
    },
    Reset {
        targets: BTreeMap<String, Vec<String>>,
        source: String,
        credit: Option<u64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub next_id: u64,
    pub currency: String,
    pub tasks: Vec<Task>,
    pub models: Vec<Model>,
    pub pools: Vec<Pool>,
    pub budgets: Vec<Budget>,
    pub credits: Vec<Credit>,
    pub events: Vec<Event>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            currency: "CNY".into(),
            tasks: vec![],
            models: vec![],
            pools: vec![],
            budgets: vec![],
            credits: vec![],
            events: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QuotaView {
    pub pool: String,
    pub window: String,
    pub unit: String,
    pub kind: WindowKind,
    pub limit: f64,
    pub used: f64,
    pub reserved: f64,
    pub available: f64,
    pub next_reset: Option<Time>,
    pub approximate: bool,
}

impl State {
    pub fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn task(&self, id: u64) -> Result<&Task> {
        self.tasks
            .iter()
            .find(|t| t.id == id)
            .with_context(|| format!("任务 #{id} 不存在"))
    }
    pub fn model(&self, name: &str) -> Result<&Model> {
        self.models
            .iter()
            .find(|m| m.name == name)
            .with_context(|| format!("模型 {name} 不存在"))
    }
    pub fn pool(&self, name: &str) -> Result<&Pool> {
        self.pools
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("额度池 {name} 不存在"))
    }
    pub fn remaining_budget(&self, b: &Budget, now: Time) -> (u64, u64) {
        if b.released || self.task(b.task).is_ok_and(|t| !t.status.active()) {
            return (0, 0);
        }
        let (mut input, mut output) = (0u64, 0u64);
        for e in self
            .events
            .iter()
            .filter(|e| !e.voided && e.id > b.id && e.at >= b.created && e.at <= now)
        {
            if let EventKind::Usage {
                task: Some(task),
                model,
                input: i,
                output: o,
                ..
            } = &e.kind
            {
                if *task == b.task && *model == b.model {
                    input = input.saturating_add(*i);
                    output = output.saturating_add(*o);
                }
            }
        }
        (
            b.input.saturating_sub(input),
            b.output.saturating_sub(output),
        )
    }
    pub fn task_usage(&self, id: u64, now: Time) -> (u64, u64, f64) {
        let (mut i, mut o, mut cost) = (0u64, 0u64, 0.0);
        for e in self.events.iter().filter(|e| !e.voided && e.at <= now) {
            if let EventKind::Usage {
                task: Some(t),
                input,
                output,
                cost: c,
                ..
            } = &e.kind
            {
                if *t == id {
                    i = i.saturating_add(*input);
                    o = o.saturating_add(*output);
                    cost += c;
                }
            }
        }
        (i, o, cost)
    }
    pub fn reserved(&self, pool: &str, now: Time, exclude: Option<u64>) -> f64 {
        self.budgets
            .iter()
            .filter(|b| Some(b.task) != exclude)
            .map(|b| {
                let (i, o) = self.remaining_budget(b, now);
                self.model(&b.model)
                    .ok()
                    .and_then(|m| m.bindings.iter().find(|x| x.pool == pool))
                    .map_or(0.0, |x| x.units(i, o))
            })
            .sum()
    }
    pub fn quota(&self, pool: &Pool, w: &Window, now: Time, exclude: Option<u64>) -> QuotaView {
        let (start, mut next) = w.bounds(now);
        // Baselines are ordered by timestamp AND sequence, so records made in one second
        // remain deterministic. Natural fixed boundaries supersede older snapshots.
        let baseline = self
            .events
            .iter()
            .filter(|e| {
                !e.voided
                    && e.at <= now
                    && start.is_none_or(|s| {
                        if w.kind == WindowKind::Rolling {
                            e.at > s
                        } else {
                            e.at >= s
                        }
                    })
            })
            .filter_map(|e| match &e.kind {
                EventKind::Snapshot {
                    pool: p,
                    window,
                    used,
                } if p == &pool.name && window == &w.name => Some((e, *used)),
                EventKind::Reset { targets, .. }
                    if targets
                        .get(&pool.name)
                        .is_some_and(|ws| ws.contains(&w.name)) =>
                {
                    Some((e, 0.0))
                }
                _ => None,
            })
            .max_by_key(|(e, _)| (e.at, e.id));
        let mut used = baseline.map_or(0.0, |(_, v)| v);
        let approximate = w.kind == WindowKind::Rolling && baseline.is_some_and(|(_, v)| v > 0.0);
        if w.kind == WindowKind::Rolling {
            next = baseline
                .filter(|(_, v)| *v > 0.0)
                .map(|(e, _)| e.at + Duration::seconds(w.seconds));
        }
        for e in self.events.iter().filter(|e| !e.voided && e.at <= now) {
            let inside = start.is_none_or(|s| {
                if w.kind == WindowKind::Rolling {
                    e.at > s
                } else {
                    e.at >= s
                }
            });
            if !inside || baseline.is_some_and(|(b, _)| (e.at, e.id) <= (b.at, b.id)) {
                continue;
            }
            if let EventKind::Usage { units, .. } = &e.kind {
                used += units.get(&pool.name).copied().unwrap_or(0.0);
                if w.kind == WindowKind::Rolling && units.get(&pool.name).is_some_and(|x| *x > 0.0)
                {
                    let release = e.at + Duration::seconds(w.seconds);
                    next = Some(next.map_or(release, |n| n.min(release)));
                }
            }
        }
        let reserved = self.reserved(&pool.name, now, exclude);
        QuotaView {
            pool: pool.name.clone(),
            window: w.name.clone(),
            unit: pool.unit.clone(),
            kind: w.kind,
            limit: w.limit,
            used,
            reserved,
            available: (w.limit - used - reserved).max(0.0),
            next_reset: next,
            approximate,
        }
    }
    pub fn quotas(&self, now: Time) -> Vec<QuotaView> {
        self.pools
            .iter()
            .flat_map(|p| p.windows.iter().map(move |w| self.quota(p, w, now, None)))
            .collect()
    }
    pub fn budget_set(
        &mut self,
        task: u64,
        model: &str,
        input: u64,
        output: u64,
        now: Time,
        force: bool,
    ) -> Result<u64> {
        ensure!(self.task(task)?.status.active(), "任务已结束，请先重新打开");
        tokens(input, output)?;
        let m = self.model(model)?;
        ensure!(m.enabled, "模型已停用");
        ensure!(
            m.capability >= self.task(task)?.capability,
            "模型能力等级低于任务要求"
        );
        let allowed = &self.task(task)?.allowed_models;
        ensure!(
            allowed.is_empty() || allowed.iter().any(|n| n == model),
            "该模型不在任务允许的模型列表中"
        );
        // Replace only this task/model budget; retain other models' reservations.
        for binding in &m.bindings {
            let p = self.pool(&binding.pool)?;
            let old: f64 = self
                .budgets
                .iter()
                .filter(|b| b.task == task && b.model == model)
                .map(|b| {
                    let (i, o) = self.remaining_budget(b, now);
                    binding.units(i, o)
                })
                .sum();
            for w in &p.windows {
                let q = self.quota(p, w, now, None);
                ensure!(
                    force
                        || q.used + q.reserved - old + binding.units(input, output)
                            <= q.limit + 1e-8,
                    "额度不足：{}/{}；可用 {:.2}，申请 {:.2}。可减少预算或使用 --force 记录超配",
                    p.name,
                    w.name,
                    (q.limit - q.used - q.reserved + old).max(0.0),
                    binding.units(input, output)
                );
            }
        }
        for b in &mut self.budgets {
            if b.task == task && b.model == model {
                b.released = true;
            }
        }
        let id = self.id();
        self.budgets.push(Budget {
            id,
            task,
            model: model.into(),
            input,
            output,
            created: now,
            released: false,
        });
        Ok(id)
    }
    pub fn reset(
        &mut self,
        targets: BTreeMap<String, Vec<String>>,
        credit: Option<u64>,
        restart: bool,
        source: &str,
        note: String,
        now: Time,
    ) -> Result<u64> {
        ensure!(!targets.is_empty(), "请指定至少一个额度池");
        for (p, ws) in &targets {
            let pool = self.pool(p)?;
            ensure!(!ws.is_empty(), "额度池 {p} 没有可重置窗口");
            for name in ws {
                ensure!(
                    pool.windows.iter().any(|w| &w.name == name),
                    "窗口 {p}/{name} 不存在"
                );
            }
        }
        let source = if let Some(id) = credit {
            let c = self
                .credits
                .iter()
                .find(|c| c.id == id)
                .context("重置机会不存在")?;
            ensure!(c.count > 0, "重置机会已用完");
            ensure!(c.expires.is_none_or(|t| t > now), "重置机会已过期");
            for (p, ws) in &targets {
                ensure!(
                    c.pools.is_empty() || c.pools.contains(p),
                    "重置机会不适用于额度池 {p}"
                );
                ensure!(
                    c.windows.is_empty() || ws.iter().all(|w| c.windows.contains(w)),
                    "重置机会不适用于选定窗口"
                );
            }
            format!("{:?}", c.kind).to_lowercase()
        } else {
            ensure!(
                source == "tibo" || source == "manual",
                "无卡重置来源须为 tibo 或 manual"
            );
            source.into()
        };
        if let Some(id) = credit {
            self.credits.iter_mut().find(|c| c.id == id).unwrap().count -= 1;
        }
        if restart {
            for p in &mut self.pools {
                if let Some(ws) = targets.get(&p.name) {
                    for w in &mut p.windows {
                        if ws.contains(&w.name) && w.kind == WindowKind::Fixed {
                            w.anchor = now;
                        }
                    }
                }
            }
        }
        let id = self.id();
        self.events.push(Event {
            id,
            at: now,
            note,
            voided: false,
            kind: EventKind::Reset {
                targets,
                source,
                credit,
            },
        });
        Ok(id)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "不支持的数据版本 {}", self.version);
        ensure!(
            self.next_id > 0 && self.next_id < u64::MAX - 1,
            "数据编号空间已耗尽"
        );
        ensure!(!self.currency.trim().is_empty(), "货币单位不能为空");
        let mut ids = BTreeSet::new();
        for id in self
            .tasks
            .iter()
            .map(|x| x.id)
            .chain(self.budgets.iter().map(|x| x.id))
            .chain(self.credits.iter().map(|x| x.id))
            .chain(self.events.iter().map(|x| x.id))
        {
            ensure!(
                id > 0 && id < self.next_id && ids.insert(id),
                "数据编号重复或越界"
            );
        }
        let mut names = BTreeSet::new();
        for p in &self.pools {
            name(&p.name)?;
            ensure!(names.insert(&p.name), "额度池重名");
            ensure!(!p.unit.trim().is_empty(), "额度单位不能为空");
            let mut windows = BTreeSet::new();
            for w in &p.windows {
                name(&w.name)?;
                ensure!(windows.insert(&w.name), "窗口重名");
                positive(w.limit)?;
                ensure!(
                    w.kind == WindowKind::Manual || (1..=315_360_000).contains(&w.seconds),
                    "周期须在 1 秒到 10 年之间"
                );
            }
        }
        names.clear();
        let mut owners = BTreeSet::new();
        for m in &self.models {
            name(&m.name)?;
            ensure!(names.insert(&m.name), "模型重名");
            level(m.capability)?;
            for p in [m.input_price, m.output_price, m.cached_price] {
                nonnegative(p)?;
            }
            let mut bound = BTreeSet::new();
            ensure!(
                m.bindings.len() == 1 && m.bindings[0].pool == m.name,
                "每个模型须有且只有自己的独立额度池"
            );
            for b in &m.bindings {
                self.pool(&b.pool)?;
                ensure!(bound.insert(&b.pool), "模型重复绑定额度池");
                ensure!(owners.insert(&b.pool), "不能在模型之间共享额度池");
                nonnegative(b.input_weight)?;
                nonnegative(b.output_weight)?;
                ensure!(b.input_weight + b.output_weight > 0.0, "权重不能同时为零");
            }
        }
        ensure!(owners.len() == self.pools.len(), "存在未归属模型的额度池");
        for t in &self.tasks {
            ensure!(!t.title.trim().is_empty(), "任务标题不能为空");
            level(t.importance)?;
            level(t.urgency)?;
            level(t.capability)?;
            tokens(t.input, t.output)?;
            ensure!(t.minutes > 0, "任务时长须大于 0");
            let mut seen = BTreeSet::new();
            self.visit(t.id, &mut seen, &mut BTreeSet::new())?;
            for m in &t.allowed_models {
                self.model(m)?;
            }
            ensure!(
                t.progress.is_finite() && (0.0..=100.0).contains(&t.progress),
                "任务进度须为 0–100"
            );
            for (model, factor) in &t.factors {
                self.model(model)?;
                ensure!(
                    factor.is_finite() && (0.01..=100.0).contains(factor),
                    "token 倍率须为 0.01–100"
                );
                ensure!(
                    t.input as f64 * factor <= MAX_TOKENS as f64
                        && t.output as f64 * factor <= MAX_TOKENS as f64,
                    "倍率换算后 token 超出上限"
                );
            }
            for [a, b] in &t.preferences {
                self.model(a)?;
                self.model(b)?;
                ensure!(
                    t.allowed_models.is_empty()
                        || (t.allowed_models.contains(a) && t.allowed_models.contains(b)),
                    "偏好模型必须在任务允许列表中"
                );
                ensure!(
                    a != b && !prefers(&t.preferences, b, a),
                    "模型偏好形成循环：{a} > {b}"
                );
            }
        }
        let mut active = BTreeSet::new();
        for b in &self.budgets {
            self.task(b.task)?;
            self.model(&b.model)?;
            tokens(b.input, b.output)?;
            ensure!(
                b.released || active.insert((b.task, &b.model)),
                "任务/模型有重复的有效预算"
            );
        }
        for c in &self.credits {
            for p in &c.pools {
                self.pool(p)?;
            }
            for w in &c.windows {
                ensure!(
                    self.pools
                        .iter()
                        .filter(|p| c.pools.is_empty() || c.pools.contains(&p.name))
                        .any(|p| p.windows.iter().any(|x| &x.name == w)),
                    "重置机会引用了未知窗口"
                );
            }
        }
        for e in &self.events {
            match &e.kind {
                EventKind::Usage {
                    task,
                    model,
                    input,
                    output,
                    cached,
                    cost,
                    units,
                } => {
                    if let Some(t) = task {
                        self.task(*t)?;
                    }
                    self.model(model)?;
                    tokens(*input, *output)?;
                    ensure!(cached <= input, "缓存 token 大于输入 token");
                    nonnegative(*cost)?;
                    for (p, u) in units {
                        self.pool(p)?;
                        nonnegative(*u)?;
                    }
                    ensure!(
                        units.len() == 1 && units.contains_key(model),
                        "用量只能记入其模型自己的额度"
                    );
                }
                EventKind::Snapshot { pool, window, used } => {
                    ensure!(
                        self.pool(pool)?.windows.iter().any(|w| &w.name == window),
                        "快照窗口不存在"
                    );
                    nonnegative(*used)?;
                }
                EventKind::Reset {
                    targets, credit, ..
                } => {
                    if let Some(c) = credit {
                        ensure!(
                            self.credits.iter().any(|x| x.id == *c),
                            "重置记录引用未知机会"
                        );
                    }
                    for (p, ws) in targets {
                        for w in ws {
                            ensure!(
                                self.pool(p)?.windows.iter().any(|x| &x.name == w),
                                "重置窗口不存在"
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn visit(&self, id: u64, done: &mut BTreeSet<u64>, stack: &mut BTreeSet<u64>) -> Result<()> {
        if done.contains(&id) {
            return Ok(());
        }
        ensure!(stack.insert(id), "任务依赖形成循环（#{id}）");
        for dep in &self.task(id)?.depends {
            self.visit(*dep, done, stack)?;
        }
        stack.remove(&id);
        done.insert(id);
        Ok(())
    }
}

/// Transitive reachability preserves incomparability; no total order is invented.
pub fn prefers(edges: &[[String; 2]], a: &str, b: &str) -> bool {
    let mut pending = vec![a];
    let mut seen = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        for [from, to] in edges {
            if from == node {
                if to == b {
                    return true;
                }
                pending.push(to);
            }
        }
    }
    false
}

pub fn level(x: u8) -> Result<()> {
    ensure!((1..=5).contains(&x), "等级须为 1–5");
    Ok(())
}
pub fn nonnegative(x: f64) -> Result<()> {
    ensure!(
        x.is_finite() && (0.0..=1e15).contains(&x),
        "数值须为有限的非负数，且不超过 1e15"
    );
    Ok(())
}
pub fn positive(x: f64) -> Result<()> {
    nonnegative(x)?;
    ensure!(x > 0.0, "数值须大于 0");
    Ok(())
}
pub fn tokens(i: u64, o: u64) -> Result<()> {
    ensure!(
        i <= MAX_TOKENS && o <= MAX_TOKENS,
        "单项 token 数不能超过 {MAX_TOKENS}"
    );
    Ok(())
}
pub fn name(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && !s.chars().any(|c| c.is_whitespace()
                || c.is_control()
                || c == '='
                || c == ','
                || c == '/'),
        "名称不能为空或包含空格、逗号、斜线、等号"
    );
    Ok(())
}
pub fn parse_time(s: &str, end_of_day: bool) -> Result<Time> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    let naive = if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        d.and_hms_opt(
            if end_of_day { 23 } else { 0 },
            if end_of_day { 59 } else { 0 },
            if end_of_day { 59 } else { 0 },
        )
        .unwrap()
    } else {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M"))
            .context("日期格式：2026-09-21、2026-09-21T18:00 或带时区的 RFC3339")?
    };
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|d| d.with_timezone(&Utc))
        .context("当地时间不唯一或不存在，请使用带时区的 RFC3339")
}
pub fn parse_duration(s: &str) -> Result<i64> {
    let split = s
        .find(|c: char| !c.is_ascii_digit())
        .context("周期需要单位，例如 5h、7d、30m")?;
    let n: i64 = s[..split].parse().context("周期数值无效")?;
    let factor = match &s[split..] {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => bail!("周期单位须为 s/m/h/d/w"),
    };
    let seconds = n.checked_mul(factor).context("周期过大")?;
    ensure!(
        (1..=315_360_000).contains(&seconds),
        "周期须在 1 秒到 10 年之间"
    );
    Ok(seconds)
}
