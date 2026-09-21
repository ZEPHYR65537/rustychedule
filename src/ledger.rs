use crate::domain::*;
use anyhow::{Result, ensure};
use chrono::{Datelike, Months, NaiveDate};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum Funding {
    #[default]
    Subscription,
    Api,
}
impl Funding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Subscription => "订阅",
            Self::Api => "API",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum FundingPolicy {
    #[default]
    SubscriptionFirst,
    ModelFirst,
    SubscriptionOnly,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelAccount {
    pub subscription: Option<Subscription>,
    /// Lifetime cumulative purchased token ceiling; never reset by subscription resets.
    pub api_token_limit: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub plan: String,
    pub renewal_day: Option<u32>,
    pub active: bool,
    pub estimate_note: String,
}
impl Subscription {
    pub fn next_renewal(&self, today: NaiveDate) -> Option<NaiveDate> {
        let day = self.renewal_day?;
        let in_month = |date: NaiveDate| {
            let first = date.with_day(1).unwrap();
            let last = first
                .checked_add_months(Months::new(1))
                .unwrap()
                .pred_opt()
                .unwrap()
                .day();
            first.with_day(day.min(last)).unwrap()
        };
        let current = in_month(today);
        Some(if current >= today {
            current
        } else {
            in_month(
                today
                    .with_day(1)
                    .unwrap()
                    .checked_add_months(Months::new(1))
                    .unwrap(),
            )
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkSession {
    pub id: u64,
    pub task: u64,
    pub at: Time,
    pub minutes: u32,
    pub usage_event: Option<u64>,
    pub completed: String,
    pub learned: String,
    pub next_steps: String,
    pub voided: bool,
}

impl State {
    pub fn subscription_enabled(&self, model: &str) -> bool {
        // Legacy models keep their v1 limits; API-only models have no subscription capacity.
        self.accounts
            .get(model)
            .is_none_or(|b| match &b.subscription {
                Some(s) => s.active,
                None => self.pool(model).is_ok_and(|p| !p.windows.is_empty()),
            })
    }
    pub fn funding_units(&self, model: &str, source: Funding, i: u64, o: u64) -> f64 {
        match source {
            Funding::Api => i as f64 + o as f64,
            Funding::Subscription => self
                .model(model)
                .map(|m| m.bindings[0].units(i, o))
                .unwrap_or(0.0),
        }
    }
    pub fn api_quota(&self, model: &str, now: Time) -> QuotaView {
        let limit = self.accounts.get(model).map_or(0, |b| b.api_token_limit) as f64;
        let used = self
            .events
            .iter()
            .filter(|e| !e.voided && e.at <= now && e.funding == Funding::Api)
            .filter_map(|e| {
                if let EventKind::Usage {
                    model: m,
                    input,
                    output,
                    ..
                } = &e.kind
                {
                    (m == model).then_some(*input as f64 + *output as f64)
                } else {
                    None
                }
            })
            .sum::<f64>();
        let reserved = self
            .budgets
            .iter()
            .filter(|b| b.model == model && b.funding == Funding::Api)
            .map(|b| {
                let (i, o) = self.remaining_budget(b, now);
                i as f64 + o as f64
            })
            .sum::<f64>();
        QuotaView {
            pool: model.into(),
            window: "api".into(),
            unit: "token".into(),
            kind: WindowKind::Manual,
            limit,
            used,
            reserved,
            available: (limit - used - reserved).max(0.0),
            next_reset: None,
            approximate: false,
            funding: Funding::Api,
            observed_percent: None,
        }
    }
    pub fn preferences_for<'a>(&'a self, t: &'a Task) -> &'a [[String; 2]] {
        if t.preferences.is_empty() {
            &self.model_preferences
        } else {
            &t.preferences
        }
    }
    pub fn validate_ledger(&self) -> Result<()> {
        for (m, b) in &self.accounts {
            self.model(m)?;
            ensure!(b.api_token_limit <= MAX_TOKENS, "API token 上界过大");
            if let Some(sub) = &b.subscription {
                ensure!(
                    sub.renewal_day.is_none_or(|d| (1..=31).contains(&d)),
                    "月续期日须为 1–31"
                );
                let w = self.pool(m)?.windows.iter().find(|w| w.name == "week");
                ensure!(
                    w.is_some_and(|w| w.kind == WindowKind::Fixed && w.seconds == 604800),
                    "订阅须有 7 天固定 week 窗口"
                );
                ensure!(
                    self.pool(m)?.unit == "token"
                        && self.model(m)?.bindings[0].input_weight == 1.0
                        && self.model(m)?.bindings[0].output_weight == 1.0,
                    "订阅估计按原始 token 计量，权重须为 1"
                );
            }
        }
        for [a, b] in &self.model_preferences {
            self.model(a)?;
            self.model(b)?;
            ensure!(
                a != b && !prefers(&self.model_preferences, b, a),
                "全局模型偏序存在循环"
            );
        }
        for session in &self.sessions {
            self.task(session.task)?;
            ensure!(
                session.minutes > 0
                    || session.usage_event.is_some()
                    || !session.completed.trim().is_empty()
                    || !session.learned.trim().is_empty()
                    || !session.next_steps.trim().is_empty(),
                "复盘记录不能为空"
            );
            if let Some(id) = session.usage_event {
                ensure!(
                    self.events.iter().any(|e| e.id == id
                        && matches!(&e.kind,EventKind::Usage{task:Some(t),..} if *t==session.task)),
                    "复盘引用的用量记录无效"
                );
            }
        }
        Ok(())
    }
}
