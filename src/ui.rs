use crate::ledger::Funding;
use crate::{domain::*, planner::Plan};
use chrono::{Duration, Local};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn fit(s: &str, width: usize) -> String {
    let s = clean(s);
    let mut out = String::new();
    let mut n = 0;
    let truncated = s.width() > width;
    let limit = width.saturating_sub(usize::from(truncated));
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if n + w > limit {
            break;
        }
        out.push(c);
        n += w;
    }
    if truncated {
        out.push('…');
        n += 1;
    }
    out.push_str(&" ".repeat(width.saturating_sub(n)));
    out
}
pub fn table(headers: &[&str], rows: Vec<Vec<String>>, caps: &[usize]) -> String {
    let widths: Vec<_> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| clean(&r[i]).width())
                .max()
                .unwrap_or(0)
                .max(h.width())
                .min(caps[i])
        })
        .collect();
    let mut out = headers
        .iter()
        .enumerate()
        .map(|(i, s)| fit(s, widths[i]))
        .collect::<Vec<_>>()
        .join("  ");
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("  "),
    );
    out.push('\n');
    for row in rows {
        out.push_str(
            &row.iter()
                .enumerate()
                .map(|(i, s)| fit(s, widths[i]))
                .collect::<Vec<_>>()
                .join("  "),
        );
        out.push('\n');
    }
    out
}
pub fn fmt_time(t: Time) -> String {
    t.with_timezone(&Local).format("%m-%d %H:%M").to_string()
}
pub fn number(x: f64) -> String {
    if x == 0.0 {
        "0".into()
    } else if x.abs() >= 1_000_000.0 {
        format!("{:.2}M", x / 1_000_000.0)
    } else if x.abs() >= 10_000.0 {
        format!("{:.1}k", x / 1000.0)
    } else {
        format!("{x:.0}")
    }
}
fn bar(used: f64, reserved: f64, limit: f64) -> String {
    if limit <= 0.0 {
        return "[ 上限为0 ]".into();
    }
    let used_n = ((used / limit).clamp(0.0, 1.0) * 18.0).round() as usize;
    let reserve_n = (((reserved / limit).clamp(0.0, 1.0) * 18.0).round() as usize).min(18 - used_n);
    format!(
        "[{}{}{}]",
        "█".repeat(used_n),
        "░".repeat(reserve_n),
        "·".repeat(18 - used_n - reserve_n)
    )
}
pub fn task_table(tasks: &[&Task], now: Time) -> String {
    if tasks.is_empty() {
        return "暂无任务。用 task add 添加任务。\n".into();
    }
    table(
        &[
            "ID",
            "任务",
            "状态",
            "重要/紧迫",
            "分数",
            "进度",
            "分钟",
            "截止",
        ],
        tasks
            .iter()
            .map(|t| {
                vec![
                    format!("#{}", t.id),
                    t.title.clone(),
                    t.status.label().into(),
                    format!("{}/{}", t.importance, t.urgency_at(now)),
                    t.score(now).to_string(),
                    format!("{:.0}%", t.progress),
                    t.minutes.to_string(),
                    t.due.map(fmt_time).unwrap_or_else(|| "—".into()),
                ]
            })
            .collect(),
        &[8, 32, 6, 10, 5, 6, 6, 12],
    )
}
pub fn quota_table(quotas: &[QuotaView], now: Time) -> String {
    if quotas.is_empty() {
        return "暂无额度窗口。用 quota add 为每个模型分别配置。\n".into();
    }
    let mut out = table(
        &[
            "模型/窗口",
            "用量 / 预留",
            "已用% / 面板观测",
            "已用+预留 / 限额",
            "可分配",
            "自然重置/释放",
        ],
        quotas
            .iter()
            .map(|q| {
                let next = q
                    .next_reset
                    .map(|t| format!("{} ({}m)", fmt_time(t), (t - now).num_minutes().max(0)))
                    .unwrap_or_else(|| {
                        if q.funding == crate::ledger::Funding::Api {
                            "累计上限，不自动重置".into()
                        } else if q.kind == WindowKind::Rolling {
                            "滚动窗口（暂无用量）".into()
                        } else {
                            "仅手动重置".into()
                        }
                    });
                vec![
                    format!("{}/{}/{}", q.pool, q.funding.label(), q.window),
                    bar(q.used, q.reserved, q.limit),
                    format!(
                        "{}{:.0}%{}{}",
                        if q.approximate { "~" } else { "" },
                        if q.limit > 0.0 && q.used > 0.0 {
                            q.used / q.limit * 100.0
                        } else {
                            0.0
                        },
                        if q.used + q.reserved > q.limit {
                            "!"
                        } else {
                            ""
                        },
                        q.observed_percent
                            .map(|p| format!(" / {p:.0}%"))
                            .unwrap_or_default()
                    ),
                    format!(
                        "{}+{} / {} {}",
                        number(q.used),
                        number(q.reserved),
                        number(q.limit),
                        q.unit
                    ),
                    number(q.available),
                    if q.approximate && q.kind == WindowKind::Rolling {
                        format!("~{next}")
                    } else {
                        next
                    },
                ]
            })
            .collect(),
        &[28, 20, 20, 34, 10, 24],
    );
    out.push_str("█ 已用  ░ 任务预留  · 空闲  ! 超限  ~ 估计值\n每模型的订阅与 API 分开记账。订阅窗口须同时满足；API 上限为 0 则不使用。\n");
    out
}
pub fn matrix(s: &State, now: Time) -> String {
    let headings = [
        "Ⅰ 立即做：重要且紧迫",
        "Ⅱ 安排：重要不紧迫",
        "Ⅲ 简化/委派：紧迫不重要",
        "Ⅳ 延后/取消：不重要不紧迫",
    ];
    let mut out = String::new();
    let groups: Vec<Vec<_>> = (0..4)
        .map(|q| {
            let mut ts: Vec<_> = s
                .tasks
                .iter()
                .filter(|t| t.status.active() && t.quadrant(now) == q)
                .collect();
            ts.sort_by_key(|t| std::cmp::Reverse(t.score(now)));
            ts
        })
        .collect();
    for pair in [[0, 1], [2, 3]] {
        out.push_str(&format!("┌{}┬{}┐\n", "─".repeat(46), "─".repeat(46)));
        out.push_str(&format!(
            "│{}│{}│\n",
            fit(headings[pair[0]], 46),
            fit(headings[pair[1]], 46)
        ));
        for i in 0..groups[pair[0]].len().max(groups[pair[1]].len()).max(1) {
            let cell = |q: usize| {
                groups[q]
                    .get(i)
                    .map(|t| format!("#{} {} [{}]", t.id, t.title, t.status.label()))
                    .unwrap_or_else(|| {
                        if i == 0 {
                            "（空）".into()
                        } else {
                            String::new()
                        }
                    })
            };
            out.push_str(&format!(
                "│{}│{}│\n",
                fit(&cell(pair[0]), 46),
                fit(&cell(pair[1]), 46)
            ));
        }
        out.push_str(&format!("└{}┴{}┘\n", "─".repeat(46), "─".repeat(46)));
    }
    out.push_str("重要/紧迫 ≥ 4 为高；紧迫性会随截止日期临近自动上升。\n");
    out
}
pub fn models(s: &State) -> String {
    table(
        &["模型", "提供方", "能力", "启用", "额度折算 I/O"],
        s.models
            .iter()
            .map(|m| {
                vec![
                    m.name.clone(),
                    m.provider.clone(),
                    m.capability.to_string(),
                    m.enabled.to_string(),
                    format!(
                        "{}/{}",
                        m.bindings[0].input_weight, m.bindings[0].output_weight
                    ),
                ]
            })
            .collect(),
        &[22, 24, 6, 6, 20],
    )
}
pub fn credits(s: &State, now: Time) -> String {
    if s.credits.is_empty() {
        return "暂无 reset 卡。随机 Tibo 刷新使用 reset --source tibo 登记。\n".into();
    }
    table(
        &[
            "ID",
            "来源",
            "剩余次数",
            "适用模型",
            "窗口",
            "有效期",
            "状态",
        ],
        s.credits
            .iter()
            .map(|c| {
                vec![
                    format!("#{}", c.id),
                    format!("{:?}", c.kind),
                    c.count.to_string(),
                    if c.pools.is_empty() {
                        "任意模型".into()
                    } else {
                        c.pools.join(",")
                    },
                    if c.windows.is_empty() {
                        "所有窗口".into()
                    } else {
                        c.windows.join(",")
                    },
                    c.expires.map(fmt_time).unwrap_or_else(|| "不限".into()),
                    if c.expires.is_some_and(|t| t <= now) {
                        "过期"
                    } else if c.count == 0 {
                        "用完"
                    } else {
                        "可用"
                    }
                    .into(),
                ]
            })
            .collect(),
        &[8, 8, 8, 24, 16, 14, 6],
    )
}
pub fn dashboard(s: &State, now: Time) -> String {
    let mut tasks: Vec<_> = s.tasks.iter().filter(|t| t.status.active()).collect();
    tasks.sort_by_key(|t| (std::cmp::Reverse(t.score(now)), t.id));
    let overdue = tasks
        .iter()
        .filter(|t| t.due.is_some_and(|d| d < now))
        .count();
    let logged_minutes: u64 = s
        .sessions
        .iter()
        .filter(|r| {
            !r.voided
                && r.at.with_timezone(&Local).date_naive() == now.with_timezone(&Local).date_naive()
        })
        .map(|r| u64::from(r.minutes))
        .sum();
    format!(
        "Schedule / tc  ·  {}\n未结束 {}  ·  逾期 {}  ·  模型 {} · 今日已记账 {} 分钟 · 项目 {}\n\n当前优先任务\n{}\n订阅信息\n{}\n各模型独立额度\n{}\nReset 卡库存\n{}\n常用：matrix | plan --minutes 240 | work log | project status | hub status\n",
        fmt_time(now),
        tasks.len(),
        overdue,
        s.models.len(),
        logged_minutes,
        s.projects.len(),
        task_table(&tasks.into_iter().take(8).collect::<Vec<_>>(), now),
        crate::commands::accounts_table(s, now),
        quota_table(&s.quotas(now), now),
        credits(s, now)
    )
}
pub fn task_detail(s: &State, t: &Task, now: Time) -> String {
    let (i, o) = s.task_usage(t.id, now);
    format!(
        "{}\n项目：{}；标签：{}\n依赖：{:?}\n允许模型：{}\n有效偏序：{}\n模型倍率：{}；允许分段：{}\n基准 token 估计：输入 {} / 输出 {}；进度 {:.1}%（手动登记）\n累计实际：输入 {} / 输出 {}\n备注：{}\n",
        task_table(&[t], now),
        clean(&t.project),
        clean(&t.tags.join(",")),
        t.depends,
        if t.allowed_models.is_empty() {
            "所有".into()
        } else {
            t.allowed_models.join(",")
        },
        s.preferences_for(t)
            .iter()
            .map(|[a, b]| format!("{a} > {b}"))
            .collect::<Vec<_>>()
            .join("；"),
        serde_json::to_string(&t.factors).unwrap_or_default(),
        t.splittable,
        t.input,
        t.output,
        t.progress,
        i,
        o,
        clean(&t.note)
    )
}
pub fn budget_rows(rows: &[Value]) -> String {
    table(
        &["预算", "任务", "模型", "来源", "剩余输入", "剩余输出"],
        rows.iter()
            .map(|r| {
                vec![
                    format!("#{}", r["id"]),
                    format!("#{}", r["task"]),
                    r["model"].as_str().unwrap_or("").into(),
                    if r["source"] == "api" {
                        "API"
                    } else {
                        "订阅"
                    }
                    .into(),
                    r["input"].to_string(),
                    r["output"].to_string(),
                ]
            })
            .collect(),
        &[10, 10, 24, 14, 16, 16],
    )
}
pub fn events(events: &[&Event]) -> String {
    table(
        &["ID", "时间", "类型", "明细", "备注"],
        events
            .iter()
            .map(|e| {
                let (kind, detail) = match &e.kind {
                    EventKind::Usage {
                        model,
                        task,
                        input,
                        output,
                        ..
                    } => (
                        "用量",
                        format!(
                            "{model}/{} task={task:?} I={input} O={output}",
                            e.funding.label()
                        ),
                    ),
                    EventKind::Snapshot { pool, window, used } => {
                        ("校准", format!("{pool}/{window} 已用 {used}"))
                    }
                    EventKind::Reset {
                        targets,
                        source,
                        credit,
                    } => (
                        "刷新",
                        format!(
                            "{source} card={credit:?} {}",
                            serde_json::to_string(targets).unwrap_or_default()
                        ),
                    ),
                };
                vec![
                    format!("#{}{}", e.id, if e.voided { "×" } else { "" }),
                    fmt_time(e.at),
                    kind.into(),
                    detail,
                    e.note.clone(),
                ]
            })
            .collect(),
        &[10, 12, 6, 70, 24],
    )
}
pub fn plan(p: &Plan, committed: bool) -> String {
    let mut out = format!(
        "本次计划 · {}/{} 分钟 · 新分配保留 {:.0}% 安全余量 · {}\n",
        p.used_minutes,
        p.minutes,
        p.reserve_percent,
        if committed {
            "已保存预算"
        } else {
            "预览，尚未写入预算"
        }
    );
    for item in &p.items {
        out.push_str(&format!(
            "\n{}–{}  #{} {}  [{}m / 优先分 {}]\n",
            item.start.with_timezone(&Local).format("%H:%M"),
            item.end.with_timezone(&Local).format("%H:%M"),
            item.task,
            clean(&item.title),
            item.minutes,
            item.score
        ));
        if item.assignments.is_empty() {
            out.push_str("  人工工作 / 无新增模型需求\n");
        }
        for a in &item.assignments {
            out.push_str(&format!(
                "  → {} / {}  输入 {} / 输出 {} token  {}\n",
                clean(&a.model),
                a.funding.label(),
                a.input,
                a.output,
                if a.existing {
                    "[已有预留]"
                } else {
                    "[新分配]"
                }
            ));
        }
        for w in &item.warnings {
            out.push_str(&format!("  提示：{}\n", clean(w)));
        }
    }
    if !p.deferred.is_empty() {
        out.push_str("\n暂缓\n");
        for d in &p.deferred {
            out.push_str(&format!(
                "  #{} {}：{}\n",
                d.task,
                clean(&d.title),
                clean(&d.reason)
            ));
        }
    }
    for note in &p.notes {
        out.push_str(&format!("\n{note}"));
    }
    out.push('\n');
    out
}
pub fn report_data(s: &State, now: Time, days: u32) -> Value {
    let first = now.with_timezone(&Local).date_naive() - Duration::days(i64::from(days) - 1);
    let mut rows = vec![];
    for (m, source) in s
        .models
        .iter()
        .flat_map(|m| [Funding::Subscription, Funding::Api].map(|f| (m, f)))
    {
        let (mut input, mut output, mut count) = (0u64, 0u64, 0u64);
        let mut daily: BTreeMap<String, (u64, u64)> = (0..days)
            .map(|d| ((first + Duration::days(i64::from(d))).to_string(), (0, 0)))
            .collect();
        for e in s.events.iter().filter(|e| {
            !e.voided && e.at <= now && e.at.with_timezone(&Local).date_naive() >= first
        }) {
            if let EventKind::Usage {
                model,
                input: i,
                output: o,
                ..
            } = &e.kind
            {
                if model == &m.name && e.funding == source {
                    input = input.saturating_add(*i);
                    output = output.saturating_add(*o);
                    count += 1;
                    if let Some(v) =
                        daily.get_mut(&e.at.with_timezone(&Local).date_naive().to_string())
                    {
                        v.0 = v.0.saturating_add(*i);
                        v.1 = v.1.saturating_add(*o);
                    }
                }
            }
        }
        rows.push(json!({"model":m.name,"source":source,"input":input,"output":output,"records":count,"daily":daily}));
    }
    json!({"from":first.to_string(),"to":now.with_timezone(&Local).date_naive().to_string(),"days":days,"models":rows})
}
pub fn report(value: &Value) -> String {
    let rows = value["models"].as_array().unwrap();
    let mut out = format!(
        "{} 至 {} · 按本地日历日统计\n",
        value["from"].as_str().unwrap(),
        value["to"].as_str().unwrap()
    );
    out.push_str(&table(
        &["模型", "来源", "输入 token", "输出 token", "记录数"],
        rows.iter()
            .map(|r| {
                vec![
                    r["model"].as_str().unwrap().into(),
                    r["source"].as_str().unwrap().into(),
                    r["input"].to_string(),
                    r["output"].to_string(),
                    r["records"].to_string(),
                ]
            })
            .collect(),
        &[24, 14, 16, 16, 8],
    ));
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    for r in rows {
        let daily = r["daily"].as_object().unwrap();
        let counts: Vec<f64> = daily
            .values()
            .map(|v| v[0].as_f64().unwrap() + v[1].as_f64().unwrap())
            .collect();
        let max = counts.iter().copied().fold(1.0, f64::max);
        let spark: String = counts
            .iter()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|v| {
                if *v == 0.0 {
                    '·'
                } else {
                    blocks[((v / max) * 7.0).round() as usize]
                }
            })
            .collect();
        out.push_str(&format!(
            "{}  {}  日 token 趋势（各模型独立尺度，最多显示最近 60 天）\n",
            fit(
                &format!(
                    "{}/{}",
                    r["model"].as_str().unwrap(),
                    r["source"].as_str().unwrap()
                ),
                30
            ),
            spark
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_width_and_controls() {
        assert_eq!(fit("中文abc", 5).width(), 5);
        assert!(!fit("\u{1b}[31m", 12).contains('\u{1b}'));
    }
}
