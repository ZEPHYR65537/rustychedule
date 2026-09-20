use chrono::Duration;
use std::collections::BTreeMap;
use tongchou::{domain::*, planner, store::Store};

fn now() -> Time {
    parse_time("2026-09-21T10:00:00+08:00", false).unwrap()
}
fn state() -> State {
    let mut s = State::default();
    for (name, cap, limit, price) in [
        ("strong", 5, 1000.0, 10.0),
        ("fast", 3, 2000.0, 1.0),
        ("local", 1, 5000.0, 0.0),
    ] {
        s.models.push(Model {
            name: name.into(),
            provider: "test".into(),
            capability: cap,
            enabled: true,
            input_price: price,
            output_price: price * 2.0,
            cached_price: 0.0,
            bindings: vec![Binding {
                pool: name.into(),
                input_weight: 1.0,
                output_weight: 1.0,
            }],
        });
        s.pools.push(Pool {
            name: name.into(),
            unit: "token".into(),
            windows: vec![Window {
                name: "short".into(),
                kind: WindowKind::Fixed,
                limit,
                seconds: 3600,
                anchor: now(),
            }],
        });
    }
    s
}
fn task(s: &mut State, input: u64, output: u64) -> u64 {
    let id = s.id();
    s.tasks.push(Task {
        id,
        title: format!("任务 {id}"),
        importance: 5,
        urgency: 5,
        due: None,
        minutes: 30,
        input,
        output,
        capability: 1,
        status: Status::Todo,
        project: "project".into(),
        tags: vec![],
        depends: vec![],
        allowed_models: vec![],
        preferences: vec![
            ["strong".into(), "fast".into()],
            ["fast".into(), "local".into()],
        ],
        progress: 0.0,
        factors: BTreeMap::new(),
        splittable: true,
        note: String::new(),
        created: now(),
    });
    id
}
fn log(s: &mut State, model: &str, task: Option<u64>, i: u64, o: u64, at: Time) -> u64 {
    let id = s.id();
    s.events.push(Event {
        id,
        at,
        note: String::new(),
        voided: false,
        kind: EventKind::Usage {
            task,
            model: model.into(),
            input: i,
            output: o,
            cached: 0,
            cost: 0.0,
            units: BTreeMap::from([(model.into(), (i + o) as f64)]),
        },
    });
    id
}
fn snapshot(s: &mut State, model: &str, window: &str, used: f64, at: Time) {
    let id = s.id();
    s.events.push(Event {
        id,
        at,
        note: String::new(),
        voided: false,
        kind: EventKind::Snapshot {
            pool: model.into(),
            window: window.into(),
            used,
        },
    });
}
fn used(s: &State, model: &str, at: Time) -> f64 {
    s.quotas(at).iter().find(|q| q.pool == model).unwrap().used
}

#[test]
fn preference_transitivity_and_incomparability() {
    let edges = vec![
        ["a".into(), "b".into()],
        ["b".into(), "d".into()],
        ["a".into(), "c".into()],
    ];
    assert!(prefers(&edges, "a", "d"));
    assert!(!prefers(&edges, "b", "c"));
    assert!(!prefers(&edges, "c", "b"));
}
#[test]
fn preference_cycles_and_dependency_cycles_rejected() {
    let mut s = state();
    let a = task(&mut s, 100, 0);
    let b = task(&mut s, 100, 0);
    s.tasks[0].depends.push(b);
    s.tasks[1].depends.push(a);
    assert!(s.validate().is_err());
    s.tasks[1].depends.clear();
    assert!(s.validate().is_ok());
    s.tasks[0]
        .preferences
        .push(["local".into(), "strong".into()]);
    assert!(s.validate().is_err());
}
#[test]
fn shared_model_quota_is_rejected() {
    let mut s = state();
    s.models[1].bindings[0].pool = "strong".into();
    assert!(s.validate().is_err());
}
#[test]
fn models_never_share_consumption_or_resets() {
    let mut s = state();
    log(&mut s, "strong", None, 800, 0, now());
    log(&mut s, "fast", None, 400, 0, now());
    assert_eq!(used(&s, "strong", now()), 800.0);
    assert_eq!(used(&s, "fast", now()), 400.0);
    s.reset(
        BTreeMap::from([("strong".into(), vec!["short".into()])]),
        None,
        false,
        "tibo",
        "".into(),
        now(),
    )
    .unwrap();
    assert_eq!(used(&s, "strong", now()), 0.0);
    assert_eq!(used(&s, "fast", now()), 400.0);
}
#[test]
fn natural_fixed_reset_at_exact_boundary_and_long_absence() {
    let mut s = state();
    log(
        &mut s,
        "strong",
        None,
        400,
        0,
        now() + Duration::minutes(10),
    );
    assert_eq!(used(&s, "strong", now() + Duration::minutes(59)), 400.0);
    assert_eq!(used(&s, "strong", now() + Duration::hours(1)), 0.0);
    assert_eq!(used(&s, "strong", now() + Duration::days(365)), 0.0);
    log(&mut s, "strong", None, 7, 0, now() + Duration::hours(1));
    assert_eq!(used(&s, "strong", now() + Duration::hours(1)), 7.0);
}
#[test]
fn quota_multi_window_is_conjunction() {
    let mut s = state();
    s.pools[0].windows.push(Window {
        name: "week".into(),
        kind: WindowKind::Fixed,
        limit: 100.0,
        seconds: 604800,
        anchor: now(),
    });
    let t = task(&mut s, 101, 0);
    assert!(s.budget_set(t, "strong", 101, 0, now(), false).is_err());
    assert!(s.budget_set(t, "fast", 101, 0, now(), false).is_ok());
}
#[test]
fn snapshot_orders_same_timestamp_records_and_is_window_scoped() {
    let mut s = state();
    s.pools[0].windows.push(Window {
        name: "week".into(),
        kind: WindowKind::Fixed,
        limit: 5000.0,
        seconds: 604800,
        anchor: now(),
    });
    log(&mut s, "strong", None, 100, 0, now());
    snapshot(&mut s, "strong", "short", 600.0, now());
    log(&mut s, "strong", None, 50, 0, now());
    let qs = s.quotas(now());
    assert_eq!(qs[0].used, 650.0);
    assert_eq!(qs[1].used, 150.0);
}
#[test]
fn rolling_expiry_preserves_newer_events_when_snapshot_expires() {
    let mut s = state();
    s.pools[0].windows[0].kind = WindowKind::Rolling;
    snapshot(&mut s, "strong", "short", 500.0, now());
    log(
        &mut s,
        "strong",
        None,
        100,
        0,
        now() + Duration::minutes(10),
    );
    assert_eq!(used(&s, "strong", now() + Duration::minutes(59)), 600.0);
    assert_eq!(used(&s, "strong", now() + Duration::hours(1)), 100.0);
    assert_eq!(used(&s, "strong", now() + Duration::minutes(70)), 0.0);
}
#[test]
fn manual_window_never_naturally_resets() {
    let mut s = state();
    s.pools[0].windows[0].kind = WindowKind::Manual;
    log(&mut s, "strong", None, 100, 0, now());
    assert_eq!(used(&s, "strong", now() + Duration::days(365)), 100.0);
}
#[test]
fn reset_card_scope_expiry_and_single_redemption() {
    let mut s = state();
    let id = s.id();
    s.credits.push(Credit {
        id,
        kind: CreditKind::Card,
        count: 1,
        pools: vec!["strong".into()],
        windows: vec!["short".into()],
        expires: Some(now() + Duration::days(1)),
        note: "".into(),
    });
    let targets = |m: &str| BTreeMap::from([(m.into(), vec!["short".into()])]);
    assert!(
        s.reset(targets("fast"), Some(id), false, "manual", "".into(), now())
            .is_err()
    );
    assert_eq!(s.credits[0].count, 1);
    assert!(
        s.reset(
            targets("strong"),
            Some(id),
            false,
            "manual",
            "".into(),
            now() + Duration::days(1)
        )
        .is_err()
    );
    s.reset(
        targets("strong"),
        Some(id),
        false,
        "manual",
        "".into(),
        now(),
    )
    .unwrap();
    assert_eq!(s.credits[0].count, 0);
    assert!(
        s.reset(
            targets("strong"),
            Some(id),
            false,
            "manual",
            "".into(),
            now()
        )
        .is_err()
    );
}
#[test]
fn tibo_does_not_consume_card_or_change_clock_by_default() {
    let mut s = state();
    let id = s.id();
    s.credits.push(Credit {
        id,
        kind: CreditKind::Card,
        count: 2,
        pools: vec![],
        windows: vec![],
        expires: None,
        note: "".into(),
    });
    let targets = BTreeMap::from([("strong".into(), vec!["short".into()])]);
    s.reset(
        targets.clone(),
        None,
        false,
        "tibo",
        "".into(),
        now() + Duration::minutes(15),
    )
    .unwrap();
    assert_eq!(s.credits[0].count, 2);
    assert_eq!(s.pools[0].windows[0].anchor, now());
    s.reset(
        targets,
        None,
        true,
        "tibo",
        "".into(),
        now() + Duration::minutes(30),
    )
    .unwrap();
    assert_eq!(
        s.pools[0].windows[0]
            .bounds(now() + Duration::minutes(30))
            .1,
        Some(now() + Duration::minutes(90))
    );
}
#[test]
fn strong_first_then_fallback_with_factor() {
    let mut s = state();
    task(&mut s, 800, 200);
    log(&mut s, "strong", None, 500, 0, now());
    s.tasks[0].factors.insert("fast".into(), 2.0);
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    let a = &p.items[0].assignments;
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].model, "strong");
    assert_eq!((a[0].input, a[0].output), (400, 100));
    assert_eq!(a[1].model, "fast");
    assert_eq!((a[1].input, a[1].output), (800, 200));
    planner::commit(&mut s, &p).unwrap();
    assert!(
        s.quotas(now())
            .iter()
            .all(|q| q.used + q.reserved <= q.limit)
    );
}
#[test]
fn high_priority_gets_strong_model_before_low_priority() {
    let mut s = state();
    let low = task(&mut s, 800, 200);
    s.tasks[0].importance = 1;
    s.tasks[0].urgency = 1;
    let high = task(&mut s, 800, 200);
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].task, high);
    assert_eq!(p.items[0].assignments[0].model, "strong");
    assert_eq!(p.items[1].task, low);
    assert_eq!(p.items[1].assignments[0].model, "fast");
}
#[test]
fn hard_floor_and_allowlist_never_silently_downgrade() {
    let mut s = state();
    task(&mut s, 2000, 0);
    s.tasks[0].capability = 5;
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert!(p.items.is_empty());
    s.tasks[0].capability = 1;
    s.tasks[0].allowed_models = vec!["strong".into()];
    s.tasks[0].preferences.clear();
    assert!(
        planner::build(&s, now(), 60, 0.0, None)
            .unwrap()
            .items
            .is_empty()
    );
}
#[test]
fn unsplittable_skips_partial_strong_and_uses_whole_fast() {
    let mut s = state();
    task(&mut s, 1200, 0);
    s.tasks[0].splittable = false;
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].assignments.len(), 1);
    assert_eq!(p.items[0].assignments[0].model, "fast");
}
#[test]
fn incomparable_models_tie_break_on_cost_not_invented_preference() {
    let mut s = state();
    task(&mut s, 100, 0);
    s.tasks[0].preferences = vec![["strong".into(), "fast".into()]];
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].assignments[0].model, "local");
    assert!(p.items[0].warnings.iter().any(|w| w.contains("互不可比")));
}
#[test]
fn spending_tokens_does_not_imply_progress() {
    let mut s = state();
    let t = task(&mut s, 800, 200);
    log(&mut s, "strong", Some(t), 800, 200, now());
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].assignments[0].input, 800);
    assert_eq!(p.items[0].assignments[0].model, "fast");
    s.tasks[0].progress = 75.0;
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].assignments[0].input, 200);
}
#[test]
fn reservations_consumed_by_usage_and_persist_across_reset() {
    let mut s = state();
    let t = task(&mut s, 500, 100);
    s.budget_set(t, "strong", 500, 100, now(), false).unwrap();
    log(&mut s, "strong", Some(t), 100, 20, now());
    assert_eq!(s.remaining_budget(&s.budgets[0], now()), (400, 80));
    s.reset(
        BTreeMap::from([("strong".into(), vec!["short".into()])]),
        None,
        false,
        "tibo",
        "".into(),
        now(),
    )
    .unwrap();
    assert_eq!(used(&s, "strong", now()), 0.0);
    assert_eq!(s.quotas(now())[0].reserved, 480.0);
}
#[test]
fn repeated_commit_is_idempotent_until_progress_or_usage_changes() {
    let mut s = state();
    task(&mut s, 800, 200);
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    planner::commit(&mut s, &p).unwrap();
    let budgets = s.budgets.len();
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert!(p.items[0].assignments.iter().all(|a| a.existing));
    planner::commit(&mut s, &p).unwrap();
    assert_eq!(budgets, s.budgets.len());
}
#[test]
fn rebalance_exhausted_model_preserves_history_and_uses_remaining_progress() {
    let mut s = state();
    let t = task(&mut s, 800, 200);
    s.budget_set(t, "strong", 800, 200, now(), false).unwrap();
    snapshot(&mut s, "strong", "short", 1000.0, now());
    s.tasks[0].progress = 50.0;
    assert!(
        planner::build(&s, now(), 60, 0.0, None)
            .unwrap()
            .items
            .is_empty()
    );
    planner::release_scope(&mut s, None);
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].assignments[0].model, "fast");
    assert_eq!(p.items[0].assignments[0].input, 400);
    planner::commit(&mut s, &p).unwrap();
    assert!(s.budgets[0].released);
    assert_eq!(used(&s, "strong", now()), 1000.0);
}
#[test]
fn project_rebalance_keeps_other_projects_reservations() {
    let mut s = state();
    let a = task(&mut s, 500, 0);
    let b = task(&mut s, 500, 0);
    s.tasks[1].project = "other".into();
    s.budget_set(a, "strong", 500, 0, now(), false).unwrap();
    s.budget_set(b, "strong", 500, 0, now(), false).unwrap();
    planner::release_scope(&mut s, Some("project"));
    assert!(s.budgets[0].released);
    assert!(!s.budgets[1].released);
}
#[test]
fn failed_partial_allocation_does_not_starve_next_task() {
    let mut s = state();
    task(&mut s, 9000, 0);
    let b = task(&mut s, 500, 0);
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.deferred.len(), 1);
    assert_eq!(p.items[0].task, b);
    assert_eq!(p.items[0].assignments[0].model, "strong");
}
#[test]
fn priority_inheritance_schedules_prerequisite_first() {
    let mut s = state();
    let a = task(&mut s, 0, 0);
    s.tasks[0].importance = 1;
    s.tasks[0].urgency = 1;
    task(&mut s, 0, 0);
    s.tasks[1].importance = 3;
    s.tasks[1].urgency = 3;
    let c = task(&mut s, 0, 0);
    s.tasks[2].depends.push(a);
    let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
    assert_eq!(p.items[0].task, a);
    assert_eq!(p.items[1].task, c);
    assert_eq!(p.items[0].score, p.items[1].score);
}
#[test]
fn reservations_and_safety_buffer_do_not_overallocate() {
    let mut s = state();
    let a = task(&mut s, 500, 0);
    s.budget_set(a, "strong", 500, 0, now(), false).unwrap();
    task(&mut s, 500, 0);
    let p = planner::build(&s, now(), 60, 10.0, None).unwrap();
    planner::commit(&mut s, &p).unwrap();
    assert_eq!(s.quotas(now())[0].reserved, 900.0);
}
#[test]
fn integer_rounding_never_exceeds_fractional_model_quota() {
    for limit in [0.1, 0.9, 1.0, 1.1, 9.9, 100.01, 999.99] {
        let mut s = state();
        s.pools[0].windows[0].limit = limit;
        s.models[0].bindings[0].input_weight = 1.7;
        s.models[0].bindings[0].output_weight = 2.3;
        task(&mut s, 831, 239);
        s.tasks[0].factors.insert("strong".into(), 1.33);
        let p = planner::build(&s, now(), 60, 0.0, None).unwrap();
        planner::commit(&mut s, &p).unwrap();
        assert!(
            s.quotas(now())
                .iter()
                .all(|q| q.used + q.reserved <= q.limit + 1e-8)
        );
    }
}
#[test]
fn void_usage_restores_budget_but_does_not_undo_progress() {
    let mut s = state();
    let t = task(&mut s, 500, 0);
    s.budget_set(t, "strong", 500, 0, now(), false).unwrap();
    let id = log(&mut s, "strong", Some(t), 100, 0, now());
    s.tasks[0].progress = 20.0;
    s.events.iter_mut().find(|e| e.id == id).unwrap().voided = true;
    assert_eq!(s.remaining_budget(&s.budgets[0], now()), (500, 0));
    assert_eq!(used(&s, "strong", now()), 0.0);
    assert_eq!(s.tasks[0].progress, 20.0);
}
#[test]
fn invalid_values_and_orphan_data_rejected() {
    let mut s = state();
    s.models[0].input_price = f64::NAN;
    assert!(s.validate().is_err());
    s.models[0].input_price = 0.0;
    task(&mut s, 100, 0);
    s.tasks[0].progress = 101.0;
    assert!(s.validate().is_err());
    s.tasks[0].progress = 0.0;
    s.tasks[0].factors.insert("strong".into(), 0.0);
    assert!(s.validate().is_err());
}
#[test]
fn atomic_persistence_backup_lock_and_corruption_detection() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let mut s = state();
    store.save(&s).unwrap();
    assert!(Store::open(dir.path().into()).is_err());
    task(&mut s, 100, 0);
    store.save(&s).unwrap();
    assert_eq!(store.load().unwrap().tasks.len(), 1);
    assert_eq!(
        store
            .import(&dir.path().join("state.json.bak"))
            .unwrap()
            .tasks
            .len(),
        0
    );
    std::fs::write(dir.path().join("state.json"), "broken").unwrap();
    assert!(store.load().is_err());
    drop(store);
    assert!(Store::open(dir.path().into()).is_ok());
}
#[test]
fn model_cost_cached_input_is_subset() {
    let s = state();
    let m = &s.models[0];
    assert_eq!(m.cost(1000, 100, 500), 0.007);
}
#[test]
fn duration_and_timezone_parser() {
    assert_eq!(parse_duration("5h").unwrap(), 18000);
    assert_eq!(parse_duration("7d").unwrap(), 604800);
    assert!(parse_duration("0h").is_err());
    assert!(parse_duration("99999999999999w").is_err());
    assert_eq!(now().to_rfc3339(), "2026-09-21T02:00:00+00:00");
}

#[test]
fn deadline_urgency_uses_exact_boundary() {
    let mut s = state();
    task(&mut s, 0, 0);
    s.tasks[0].urgency = 1;
    s.tasks[0].due = Some(now() + Duration::hours(24));
    assert_eq!(s.tasks[0].urgency_at(now()), 5);
    s.tasks[0].due = Some(now() + Duration::hours(24) + Duration::seconds(1));
    assert_eq!(s.tasks[0].urgency_at(now()), 4);
    s.tasks[0].due = Some(now() + Duration::hours(72) + Duration::seconds(1));
    assert_eq!(s.tasks[0].urgency_at(now()), 3);
}
