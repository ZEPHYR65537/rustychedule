use crate::domain::*;
use crate::ledger::{Funding, FundingPolicy};
use anyhow::{Result, ensure};
use chrono::Duration;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct Assignment {
    pub model: String,
    pub funding: Funding,
    pub input: u64,
    pub output: u64,
    pub existing: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct PlanItem {
    pub task: u64,
    pub title: String,
    pub score: u32,
    pub minutes: u32,
    pub start: Time,
    pub end: Time,
    pub assignments: Vec<Assignment>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Deferred {
    pub task: u64,
    pub title: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub at: Time,
    pub minutes: u32,
    pub used_minutes: u32,
    pub reserve_percent: f64,
    pub items: Vec<PlanItem>,
    pub deferred: Vec<Deferred>,
    pub notes: Vec<String>,
}

pub fn build(
    state: &State,
    now: Time,
    minutes: u32,
    reserve_percent: f64,
    project: Option<&str>,
) -> Result<Plan> {
    ensure!(minutes > 0, "可用分钟数须大于 0");
    ensure!(
        reserve_percent.is_finite() && (0.0..100.0).contains(&reserve_percent),
        "安全余量百分比须在 [0,100) 内"
    );
    let tasks: Vec<_> = state
        .tasks
        .iter()
        .filter(|t| t.status.active() && project.is_none_or(|p| t.project == p))
        .collect();
    let mut scores: BTreeMap<u64, u32> = tasks.iter().map(|t| (t.id, t.score(now))).collect();
    // A high-priority blocked successor should pull its prerequisites forward.
    for _ in 0..tasks.len() {
        let mut changed = false;
        for t in &tasks {
            for dep in &t.depends {
                let value = scores[&t.id];
                if let Some(old) = scores.get_mut(dep) {
                    if *old < value {
                        *old = value;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut pending = tasks;
    pending.sort_by(|a, b| {
        scores[&b.id]
            .cmp(&scores[&a.id])
            .then(
                a.due
                    .unwrap_or(Time::MAX_UTC)
                    .cmp(&b.due.unwrap_or(Time::MAX_UTC)),
            )
            .then(a.id.cmp(&b.id))
    });
    // Available units include reservations for ALL active tasks, even outside this plan.
    let mut free: BTreeMap<(String, Funding), f64> = state
        .pools
        .iter()
        .map(|p| {
            let available = p
                .windows
                .iter()
                .map(|w| {
                    let q = state.quota(p, w, now, None);
                    (q.limit * (1.0 - reserve_percent / 100.0) - q.used - q.reserved).max(0.0)
                })
                .fold(f64::INFINITY, f64::min);
            (
                (p.name.clone(), Funding::Subscription),
                if state.subscription_enabled(&p.name) {
                    available
                } else {
                    0.0
                },
            )
        })
        .collect();
    for model in &state.models {
        let q = state.api_quota(&model.name, now);
        free.insert(
            (model.name.clone(), Funding::Api),
            (q.limit * (1.0 - reserve_percent / 100.0) - q.used - q.reserved).max(0.0),
        );
    }
    let mut plan = Plan {
        at: now,
        minutes,
        used_minutes: 0,
        reserve_percent,
        items: vec![],
        deferred: vec![],
        notes: vec![
            "只使用当前已存在的额度；不预支自然重置、随机 Tibo 刷新或未兑换 reset 卡。".into(),
            "计划为可解释的优先级贪心排程；分钟数是本次剩余工作量，只管理用量，不管理费用。".into(),
        ],
    };
    let mut scheduled = BTreeSet::new();
    loop {
        let index = pending.iter().position(|t| {
            t.status != Status::Blocked
                && t.depends.iter().all(|d| {
                    scheduled.contains(d) || state.task(*d).is_ok_and(|x| x.status == Status::Done)
                })
        });
        let Some(index) = index else {
            break;
        };
        let t = pending.remove(index);
        let preferences = state.preferences_for(t);
        let defer = |plan: &mut Plan, reason: String| {
            plan.deferred.push(Deferred {
                task: t.id,
                title: t.title.clone(),
                reason,
            })
        };
        let available_minutes = minutes - plan.used_minutes;
        if t.minutes > available_minutes {
            defer(
                &mut plan,
                format!(
                    "需要 {} 分钟，本次仅剩 {} 分钟",
                    t.minutes, available_minutes
                ),
            );
            continue;
        }
        // Progress is explicit: spending tokens does NOT prove useful work was done.
        let remaining = 1.0 - t.progress / 100.0;
        let (need_i, need_o) = (t.input as f64 * remaining, t.output as f64 * remaining);
        let mut assignments = vec![];
        let mut bad = None;
        let (mut reserved_i, mut reserved_o) = (0.0, 0.0);
        for b in state
            .budgets
            .iter()
            .filter(|b| b.task == t.id && !b.released)
        {
            let (i, o) = state.remaining_budget(b, now);
            if i == 0 && o == 0 {
                continue;
            }
            let m = state.model(&b.model)?;
            if !m.enabled
                || m.capability < t.capability
                || (!t.allowed_models.is_empty() && !t.allowed_models.contains(&m.name))
            {
                bad = Some(format!(
                    "已预留模型 {} 已停用、能力不足或不在允许列表，请调整预算",
                    m.name
                ));
                break;
            }
            if b.funding == Funding::Api {
                let q = state.api_quota(&m.name, now);
                if q.used + q.reserved > q.limit + 1e-8 {
                    bad = Some(format!("{}/API 已超配，请 --rebalance 重规划", m.name));
                }
                if state.funding_policy == FundingPolicy::SubscriptionOnly {
                    bad = Some("策略不允许 API，但已有 API 预留；请 --rebalance".into());
                }
            } else {
                if !state.subscription_enabled(&m.name) {
                    bad = Some("已有订阅预留，但订阅已停用；请 --rebalance".into());
                }
                for binding in &m.bindings {
                    let p = state.pool(&binding.pool)?;
                    for w in &p.windows {
                        let q = state.quota(p, w, now, None);
                        if q.used + q.reserved > q.limit + 1e-8 {
                            bad = Some(format!("{}/{} 已超配，请先调整现有预算", p.name, w.name));
                        }
                    }
                }
            }
            let factor = t.factors.get(&m.name).copied().unwrap_or(1.0);
            reserved_i += i as f64 / factor;
            reserved_o += o as f64 / factor;
            assignments.push(Assignment {
                model: m.name.clone(),
                funding: b.funding,
                input: i,
                output: o,
                existing: true,
            });
        }
        if let Some(reason) = bad {
            defer(&mut plan, reason);
            continue;
        }
        let (mut extra_i, mut extra_o) = (
            (need_i - reserved_i).max(0.0),
            (need_o - reserved_o).max(0.0),
        );
        let mut preference_notes = vec![];
        let mut trial = free.clone();
        let mut attempted = BTreeSet::new();
        if !t.splittable
            && assignments
                .iter()
                .map(|a| &a.model)
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        {
            defer(
                &mut plan,
                "禁止分段，但已有多个模型预算；请用 --rebalance 重排".into(),
            );
            continue;
        }
        while extra_i > 1e-7 || extra_o > 1e-7 {
            let mut candidates: Vec<_> = state
                .models
                .iter()
                .filter(|m| {
                    m.enabled
                        && m.capability >= t.capability
                        && (t.allowed_models.is_empty() || t.allowed_models.contains(&m.name))
                        && (t.splittable || assignments.first().is_none_or(|a| a.model == m.name))
                })
                .collect();
            candidates.sort_by(|a, b| {
                let ai = t.factors.get(&a.name).copied().unwrap_or(1.0);
                let bi = t.factors.get(&b.name).copied().unwrap_or(1.0);
                ((extra_i + extra_o) * ai)
                    .total_cmp(&((extra_i + extra_o) * bi))
                    .then(a.capability.cmp(&b.capability))
                    .then(a.name.cmp(&b.name))
            });
            let mut feasible: Vec<_> = candidates
                .iter()
                .flat_map(|m| {
                    let factor = t.factors.get(&m.name).copied().unwrap_or(1.0);
                    if !t.splittable
                        && !can_cover(
                            m,
                            extra_i * factor,
                            extra_o * factor,
                            &trial,
                            &attempted,
                            state.funding_policy,
                        )
                    {
                        return vec![];
                    }
                    [Funding::Subscription, Funding::Api]
                        .into_iter()
                        .filter_map(|source| {
                            if attempted.contains(&(m.name.clone(), source))
                                || (source == Funding::Api
                                    && state.funding_policy == FundingPolicy::SubscriptionOnly)
                            {
                                return None;
                            }
                            let fraction = fit_fraction(
                                m,
                                source,
                                extra_i * factor,
                                extra_o * factor,
                                &trial,
                                true,
                            );
                            (fraction > 1e-12).then_some((*m, source, fraction, factor))
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            if state.funding_policy == FundingPolicy::SubscriptionFirst
                && feasible
                    .iter()
                    .any(|(_, s, _, _)| *s == Funding::Subscription)
            {
                feasible.retain(|(_, s, _, _)| *s == Funding::Subscription);
            }
            feasible.sort_by(|(a, sa, _, fa), (b, sb, _, fb)| {
                let ca = (extra_i + extra_o) * fa;
                let cb = (extra_i + extra_o) * fb;
                if a.name == b.name {
                    sa.cmp(sb)
                } else {
                    ca.total_cmp(&cb)
                        .then(a.capability.cmp(&b.capability))
                        .then(a.name.cmp(&b.name))
                }
            });
            let maximal: Vec<_> = feasible
                .iter()
                .filter(|(m, _, _, _)| {
                    !feasible
                        .iter()
                        .any(|(other, _, _, _)| prefers(preferences, &other.name, &m.name))
                })
                .collect();
            if let Some((m, source, fraction, factor)) = maximal.first().copied() {
                let incomparable = maximal
                    .iter()
                    .map(|(m, _, _, _)| &m.name)
                    .collect::<BTreeSet<_>>()
                    .len();
                if incomparable > 1 {
                    preference_notes.push(format!(
                        "{} 个可用模型互不可比，按预计 token、能力等级、名称选择 {}",
                        incomparable, m.name
                    ));
                }
                if state.models.iter().any(|other| {
                    prefers(preferences, &other.name, &m.name)
                        && !feasible.iter().any(|(f, _, _, _)| f.name == other.name)
                }) {
                    preference_notes.push(format!(
                        "更偏好的模型不可用/额度已用尽，降级选择 {}",
                        m.name
                    ));
                }
                let (i, o) = (
                    (extra_i * factor * fraction).ceil() as u64,
                    (extra_o * factor * fraction).ceil() as u64,
                );
                *trial.get_mut(&(m.name.clone(), *source)).unwrap() -=
                    state.funding_units(&m.name, *source, i, o);
                if *source == Funding::Api {
                    preference_notes.push(format!("{} 使用 API 补充额度：{} token", m.name, i + o));
                }
                assignments.push(Assignment {
                    model: m.name.clone(),
                    funding: *source,
                    input: i,
                    output: o,
                    existing: false,
                });
                extra_i = (extra_i - i as f64 / factor).max(0.0);
                extra_o = (extra_o - o as f64 / factor).max(0.0);
                attempted.insert((m.name.clone(), *source));
            } else {
                break;
            }
        }
        if extra_i > 1e-7 || extra_o > 1e-7 {
            defer(
                &mut plan,
                format!(
                    "独立模型额度/能力/允许列表不足，剩余基准 token 输入 {:.0}、输出 {:.0}{}；未占用任何新增额度",
                    extra_i,
                    extra_o,
                    if t.splittable {
                        ""
                    } else {
                        "（任务禁止分段）"
                    }
                ),
            );
            continue;
        }
        free = trial;
        let start = now + Duration::minutes(i64::from(plan.used_minutes));
        plan.used_minutes += t.minutes;
        let end = now + Duration::minutes(i64::from(plan.used_minutes));
        let mut warnings = preference_notes;
        if t.due.is_some_and(|d| end > d) {
            warnings.push("按此顺序预计超过截止时间".into());
        }
        for a in &assignments {
            let p = state.pool(&a.model)?;
            if p.windows.is_empty() && a.funding == Funding::Subscription {
                warnings.push(format!("{} 未设置限额，按不受限处理", a.model));
            }
        }
        scheduled.insert(t.id);
        plan.items.push(PlanItem {
            task: t.id,
            title: t.title.clone(),
            score: scores[&t.id],
            minutes: t.minutes,
            start,
            end,
            assignments,
            warnings,
        });
    }
    for t in pending {
        let reason = if t.status == Status::Blocked {
            "手动标记为阻塞".into()
        } else {
            let deps = t
                .depends
                .iter()
                .filter(|d| {
                    !scheduled.contains(d)
                        && !state.task(**d).is_ok_and(|x| x.status == Status::Done)
                })
                .map(|d| format!("#{d}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("前置任务尚未完成或未能排入本次计划：{deps}")
        };
        plan.deferred.push(Deferred {
            task: t.id,
            title: t.title.clone(),
            reason,
        });
    }
    Ok(plan)
}
/// Largest proportional segment fitting integer token charges; rounding never exceeds quota.
fn can_cover(
    m: &Model,
    mut i: f64,
    mut o: f64,
    free: &BTreeMap<(String, Funding), f64>,
    attempted: &BTreeSet<(String, Funding)>,
    policy: FundingPolicy,
) -> bool {
    for source in [Funding::Subscription, Funding::Api] {
        if attempted.contains(&(m.name.clone(), source))
            || (source == Funding::Api && policy == FundingPolicy::SubscriptionOnly)
        {
            continue;
        }
        let f = fit_fraction(m, source, i, o, free, true);
        i = (i - (i * f).ceil()).max(0.0);
        o = (o - (o * f).ceil()).max(0.0);
    }
    i <= 1e-7 && o <= 1e-7
}
fn fit_fraction(
    m: &Model,
    source: Funding,
    i: f64,
    o: f64,
    free: &BTreeMap<(String, Funding), f64>,
    split: bool,
) -> f64 {
    let fits = |f: f64| {
        let (i, o) = ((i * f).ceil() as u64, (o * f).ceil() as u64);
        let amount = if source == Funding::Api {
            i as f64 + o as f64
        } else {
            m.bindings[0].units(i, o)
        };
        amount <= free[&(m.name.clone(), source)]
    };
    if fits(1.0) {
        return 1.0;
    }
    if !split {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}
pub fn release_scope(state: &mut State, project: Option<&str>) {
    let tasks: BTreeSet<_> = state
        .tasks
        .iter()
        .filter(|t| t.status.active() && project.is_none_or(|p| t.project == p))
        .map(|t| t.id)
        .collect();
    for b in &mut state.budgets {
        if tasks.contains(&b.task) {
            b.released = true;
        }
    }
}
pub fn commit(state: &mut State, plan: &Plan) -> Result<()> {
    for item in &plan.items {
        for a in item.assignments.iter().filter(|a| !a.existing) {
            let (old_i, old_o) = state
                .budgets
                .iter()
                .filter(|b| b.task == item.task && b.model == a.model && b.funding == a.funding)
                .map(|b| state.remaining_budget(b, plan.at))
                .fold((0u64, 0u64), |(i, o), (bi, bo)| {
                    (i.saturating_add(bi), o.saturating_add(bo))
                });
            state.budget_set_funded(
                item.task,
                &a.model,
                (old_i + a.input, old_o + a.output),
                a.funding,
                plan.at,
                false,
            )?;
        }
    }
    Ok(())
}
