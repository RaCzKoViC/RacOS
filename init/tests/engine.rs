// init — host-side coverage for the parser, topological sort, and the
// restart burst tracker. None of these touch syscalls so they run on the
// host under `cargo test -p init`.

use init::engine::{Engine, RestartTracker};
use init::{parse_unit, RestartPolicy, ServiceType, Unit, UnitType};

// ─────────────────────────────────────────────────
// Unit file parser
// ─────────────────────────────────────────────────

#[test]
fn parse_unit_extracts_all_known_sections() {
    let src = r#"
[Unit]
Description=Demo service

[Dependencies]
After=base.target other.service
Requires=base.target
Wants=optional.service

[Service]
Type=oneshot
ExecStart=/sbin/demo
Restart=on-failure
RestartDelaySec=3
TimeoutStartSec=15
TimeoutStopSec=7

[Install]
WantedBy=base.target
"#;
    let unit = parse_unit("demo.service", src).expect("parse_unit should succeed");
    assert_eq!(unit.unit_type, UnitType::Service);
    assert_eq!(unit.description, "Demo service");
    assert_eq!(
        unit.after,
        vec![String::from("base.target"), String::from("other.service")]
    );
    assert_eq!(unit.requires, vec![String::from("base.target")]);
    assert_eq!(unit.wants, vec![String::from("optional.service")]);
    assert_eq!(unit.service_type, ServiceType::Oneshot);
    assert_eq!(unit.exec_start, "/sbin/demo");
    assert_eq!(unit.restart, RestartPolicy::OnFailure);
    assert_eq!(unit.restart_delay_sec, 3);
    assert_eq!(unit.timeout_start_sec, 15);
    assert_eq!(unit.timeout_stop_sec, 7);
    assert_eq!(unit.wanted_by, vec![String::from("base.target")]);
}

#[test]
fn parse_unit_detects_type_from_suffix() {
    assert_eq!(
        parse_unit("base.target", "[Unit]\n").unwrap().unit_type,
        UnitType::Target
    );
    assert_eq!(
        parse_unit("net.timer", "[Unit]\n").unwrap().unit_type,
        UnitType::Timer
    );
    assert!(parse_unit("garbage", "[Unit]\n").is_err());
}

#[test]
fn parse_unit_rejects_unknown_restart_policy() {
    let src = "[Service]\nRestart=teleport\n";
    assert!(parse_unit("x.service", src).is_err());
}

// ─────────────────────────────────────────────────
// Engine topo sort
// ─────────────────────────────────────────────────

fn unit_with_deps(name: &str, after: &[&str]) -> Unit {
    let mut u = Unit::new(name, UnitType::Service);
    u.after = after.iter().map(|s| String::from(*s)).collect();
    u
}

#[test]
fn resolve_start_order_linear_chain() {
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &[]));
    e.add_unit(unit_with_deps("b", &["a"]));
    e.add_unit(unit_with_deps("c", &["b"]));

    let res = e.resolve_start_order();
    assert!(res.cycle.is_empty(), "no cycle expected");
    let names: Vec<&str> = res
        .order
        .iter()
        .map(|&i| e.units()[i].name.as_str())
        .collect();
    // a must come before b, b before c — the topo order respects that.
    let pos_a = names.iter().position(|n| *n == "a").unwrap();
    let pos_b = names.iter().position(|n| *n == "b").unwrap();
    let pos_c = names.iter().position(|n| *n == "c").unwrap();
    assert!(pos_a < pos_b && pos_b < pos_c, "got {:?}", names);
}

#[test]
fn resolve_start_order_diamond_dependency() {
    // a → b, a → c, b → d, c → d
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &[]));
    e.add_unit(unit_with_deps("b", &["a"]));
    e.add_unit(unit_with_deps("c", &["a"]));
    e.add_unit(unit_with_deps("d", &["b", "c"]));

    let res = e.resolve_start_order();
    assert!(res.cycle.is_empty());
    assert_eq!(res.order.len(), 4);
    let names: Vec<&str> = res
        .order
        .iter()
        .map(|&i| e.units()[i].name.as_str())
        .collect();
    let pos = |n: &str| names.iter().position(|m| *m == n).unwrap();
    assert!(pos("a") < pos("b"));
    assert!(pos("a") < pos("c"));
    assert!(pos("b") < pos("d"));
    assert!(pos("c") < pos("d"));
}

#[test]
fn resolve_start_order_requires_implies_after() {
    // `b` Requires `a` (no explicit After) — engine treats Requires as an
    // ordering dep so `a` still comes first.
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &[]));
    let mut b = Unit::new("b", UnitType::Service);
    b.requires.push(String::from("a"));
    e.add_unit(b);

    let res = e.resolve_start_order();
    assert!(res.cycle.is_empty());
    let names: Vec<&str> = res
        .order
        .iter()
        .map(|&i| e.units()[i].name.as_str())
        .collect();
    assert_eq!(names, vec!["a", "b"]);
}

#[test]
fn resolve_start_order_detects_cycle() {
    // a → b → c → a (cycle)
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &["c"]));
    e.add_unit(unit_with_deps("b", &["a"]));
    e.add_unit(unit_with_deps("c", &["b"]));

    let res = e.resolve_start_order();
    assert_eq!(
        res.cycle.len(),
        3,
        "all three units should land in the cycle bucket"
    );
    assert!(res.order.is_empty(), "no unit can be scheduled");
}

#[test]
fn resolve_start_order_isolates_cycle_from_clean_units() {
    // a → b (clean), c ↔ d (cycle)
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &[]));
    e.add_unit(unit_with_deps("b", &["a"]));
    e.add_unit(unit_with_deps("c", &["d"]));
    e.add_unit(unit_with_deps("d", &["c"]));

    let res = e.resolve_start_order();
    assert_eq!(res.order.len(), 2, "a and b should still schedule");
    assert_eq!(res.cycle.len(), 2, "c and d are the cycle");
}

#[test]
fn resolve_start_order_ignores_unknown_dependency_names() {
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &["nonexistent.service"]));
    let res = e.resolve_start_order();
    assert!(res.cycle.is_empty());
    assert_eq!(res.order, vec![0]);
}

#[test]
fn resolve_start_order_ignores_self_dependency() {
    let mut e = Engine::new();
    e.add_unit(unit_with_deps("a", &["a"]));
    let res = e.resolve_start_order();
    assert!(
        res.cycle.is_empty(),
        "self-edge must be dropped, not cycled"
    );
    assert_eq!(res.order, vec![0]);
}

// ─────────────────────────────────────────────────
// Restart burst tracker
// ─────────────────────────────────────────────────

#[test]
fn restart_tracker_below_limit_does_not_trip() {
    let mut t = RestartTracker::default();
    for i in 0..5 {
        assert!(!t.record_and_check(i as u64), "iteration {} should pass", i);
    }
}

#[test]
fn restart_tracker_above_limit_within_window_trips() {
    let mut t = RestartTracker::default();
    // 6 restarts at t=0..5s — all inside the 30s window → 6th trips.
    for i in 0..5 {
        assert!(!t.record_and_check(i as u64));
    }
    assert!(t.record_and_check(5));
}

#[test]
fn restart_tracker_window_decays() {
    let mut t = RestartTracker::default();
    // Fill the budget at t=0..4.
    for i in 0..5 {
        t.record_and_check(i as u64);
    }
    // Now jump past the window — old timestamps should be evicted and a
    // fresh restart is fine.
    assert!(!t.record_and_check(120), "after window we should reset");
}
