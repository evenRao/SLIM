use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use rusqlite::Connection;
use slim_core::model::{
    actor::{Actor, NewActor, Role},
    item::{Item, NewItem},
    location::{Location, LocationKind, NewLocation},
    operation::{OperationSource, ReceiveStock},
    stock::StockThreshold,
};
use slim_core::service::*;
use slim_db::{
    connection::open_memory,
    migrate::run_migrations,
    repo::{
        actor::SqliteActorRepo,
        inventory::{SqliteInventoryService, SqliteThresholdRepo},
        item::SqliteItemRepo,
        location::SqliteLocationRepo,
    },
};
use slim_tui::{
    app::{App, Modal, ReceiveForm, Tab},
    event::handle_key_event,
    ui,
};

fn setup() -> (Connection, Actor, Item, Location) {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let a = SqliteActorRepo::new(&c)
        .create(&NewActor::new("Auditor").with_role(Role::Manager))
        .unwrap();
    let i = SqliteItemRepo::new(&c)
        .create(&NewItem {
            sku: Some("AUDIT".into()),
            name: "Audit Item".into(),
            unit: "each".into(),
            description: None,
            metadata: None,
        })
        .unwrap();
    let l = SqliteLocationRepo::new(&c)
        .create(&NewLocation {
            parent_id: None,
            name: "Drawer".into(),
            kind: LocationKind::Drawer,
        })
        .unwrap();
    (c, a, i, l)
}
fn command(a: &Actor, i: &Item, l: &Location, q: u32) -> ReceiveStock {
    ReceiveStock {
        actor_id: a.id,
        item_id: i.id,
        destination_id: l.id,
        quantity: q,
        source: OperationSource::Tui,
        reason: None,
        idempotency_key: None,
    }
}
fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
}
fn screen(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| ui::render(app, f)).unwrap();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| t.backend().buffer()[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn one_press_and_release_produces_one_character() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    app.open_search_modal();
    handle_key_event(&mut app, key(KeyCode::Char('a'), KeyEventKind::Press));
    handle_key_event(&mut app, key(KeyCode::Char('a'), KeyEventKind::Release));
    if let Some(Modal::Search(f)) = &app.active_modal {
        assert_eq!(f.query, "a");
    } else {
        panic!("search unexpectedly closed");
    }
}
#[test]
fn one_tab_press_and_release_moves_one_tab() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    handle_key_event(&mut app, key(KeyCode::Tab, KeyEventKind::Press));
    handle_key_event(&mut app, key(KeyCode::Tab, KeyEventKind::Release));
    assert_eq!(app.current_tab, Tab::Items);
}

#[test]
fn terminal_cursor_edits_unicode_and_paste_without_shortcuts() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    app.open_search_modal();
    slim_tui::event::handle_paste(&mut app, "éa");
    handle_key_event(&mut app, key(KeyCode::Left, KeyEventKind::Press));
    handle_key_event(&mut app, key(KeyCode::Char('b'), KeyEventKind::Press));
    handle_key_event(&mut app, key(KeyCode::Home, KeyEventKind::Press));
    handle_key_event(&mut app, key(KeyCode::Delete, KeyEventKind::Press));
    if let Some(Modal::Search(f)) = &app.active_modal {
        assert_eq!(f.query, "ba");
    } else {
        panic!("Search closed");
    }
}
#[test]
fn opening_receive_does_not_type_shortcut_into_item() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    handle_key_event(&mut app, key(KeyCode::Char('r'), KeyEventKind::Press));
    handle_key_event(&mut app, key(KeyCode::Char('r'), KeyEventKind::Release));
    if let Some(Modal::Receive(f)) = &app.active_modal {
        assert_eq!(f.item, "");
    } else {
        panic!("missing receive form");
    }
}
#[test]
fn receive_form_all_values_visible_at_80_by_24() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    app.active_modal = Some(Modal::Receive(ReceiveForm {
        item: "ITEM_VALUE".into(),
        location: "LOCATION_VALUE".into(),
        quantity: "QUANTITY_VALUE".into(),
        reason: "REASON_VALUE".into(),
        focused_field: 0,
    }));
    let s = screen(&mut app, 80, 24);
    println!("{s}");
    for value in [
        "ITEM_VALUE",
        "LOCATION_VALUE",
        "QUANTITY_VALUE",
        "REASON_VALUE",
    ] {
        assert!(s.contains(value), "hidden field value: {value}");
    }
}
#[test]
fn all_tabs_visible_at_120_by_35() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    let s = screen(&mut app, 120, 35);
    assert!(s.contains("8: Devices"), "Devices tab clipped from header");
}
#[test]
fn global_threshold_update_replaces_old_value() {
    let (c, _, i, _) = setup();
    let r = SqliteThresholdRepo::new(&c);
    for min_qty in [10, 3] {
        r.set(&StockThreshold {
            item_id: i.id,
            stock_location_id: None,
            min_qty,
        })
        .unwrap();
    }
    let thresholds = r.get_for_item(&i.id).unwrap();
    assert_eq!(
        thresholds.len(),
        1,
        "duplicate global thresholds: {thresholds:?}"
    );
    assert_eq!(thresholds[0].min_qty, 3);
}
#[test]
fn zero_stock_new_item_with_global_minimum_is_low() {
    let (c, _, i, _) = setup();
    SqliteThresholdRepo::new(&c)
        .set(&StockThreshold {
            item_id: i.id,
            stock_location_id: None,
            min_qty: 5,
        })
        .unwrap();
    assert!(
        !SqliteInventoryService::new(&c)
            .get_low_stock()
            .unwrap()
            .is_empty(),
        "zero-stock item omitted"
    );
}
#[test]
fn threshold_event_failure_rolls_back_stock() {
    let (c, a, i, l) = setup();
    SqliteThresholdRepo::new(&c)
        .set(&StockThreshold {
            item_id: i.id,
            stock_location_id: None,
            min_qty: 10,
        })
        .unwrap();
    c.execute_batch("CREATE TRIGGER fail_low BEFORE INSERT ON event_outbox WHEN NEW.event_type='StockBelowThreshold' BEGIN SELECT RAISE(ABORT,'injected disk failure'); END;").unwrap();
    let svc = SqliteInventoryService::new(&c);
    let result = svc.receive(command(&a, &i, &l, 5));
    assert!(result.is_err());
    let balance = svc
        .get_balance(&i.id, &l.id)
        .unwrap()
        .map(|b| b.qty)
        .unwrap_or(0);
    assert_eq!(balance, 0, "operation reported error but inventory changed");
}
#[test]
fn exceeding_quantity_range_is_rejected() {
    let (c, a, i, l) = setup();
    let svc = SqliteInventoryService::new(&c);
    svc.receive(command(&a, &i, &l, u32::MAX)).unwrap();
    let result = svc.receive(command(&a, &i, &l, 1));
    assert!(result.is_err(), "overflow accepted: {result:?}");
}
#[test]
fn mismatched_idempotency_payload_is_rejected() {
    let (c, a, i, l) = setup();
    let svc = SqliteInventoryService::new(&c);
    let mut cmd = command(&a, &i, &l, 5);
    cmd.idempotency_key = Some("request-1".into());
    svc.receive(cmd.clone()).unwrap();
    cmd.quantity = 999;
    assert!(
        svc.receive(cmd).is_err(),
        "reused key silently accepted for a different quantity"
    );
}
#[test]
fn successful_stock_change_records_audit_entry() {
    let (c, a, i, l) = setup();
    SqliteInventoryService::new(&c)
        .receive(command(&a, &i, &l, 5))
        .unwrap();
    let count: i64 = c
        .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
        .unwrap();
    assert!(count > 0, "stock mutation missing audit record");
}
#[test]
fn repeated_low_stock_does_not_duplicate_crossing_event() {
    let (c, a, i, l) = setup();
    SqliteThresholdRepo::new(&c)
        .set(&StockThreshold {
            item_id: i.id,
            stock_location_id: None,
            min_qty: 10,
        })
        .unwrap();
    let svc = SqliteInventoryService::new(&c);
    svc.receive(command(&a, &i, &l, 1)).unwrap();
    svc.receive(command(&a, &i, &l, 1)).unwrap();
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM event_outbox WHERE event_type='StockBelowThreshold'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "low-stock event repeated without a new crossing");
}
#[test]
fn tui_notices_external_inventory_changes() {
    let (c, a, i, l) = setup();
    let mut app = App::new(&c, a.clone()).unwrap();
    SqliteInventoryService::new(&c)
        .receive(command(&a, &i, &l, 5))
        .unwrap();
    screen(&mut app, 120, 35);
    handle_key_event(&mut app, key(KeyCode::Char('2'), KeyEventKind::Press));
    screen(&mut app, 120, 35);
    assert_eq!(
        app.metrics.total_units, 5,
        "open UI retains stale inventory"
    );
}
#[test]
fn normal_receive_and_reconcile_control() {
    let (c, a, i, l) = setup();
    SqliteInventoryService::new(&c)
        .receive(command(&a, &i, &l, 5))
        .unwrap();
    assert!(
        slim_db::repo::inventory::SqliteReconciliationService::new(&c)
            .check()
            .unwrap()
            .mismatches
            .is_empty()
    );
}
#[test]
fn location_specific_threshold_update_control() {
    let (c, _, i, l) = setup();
    let r = SqliteThresholdRepo::new(&c);
    for min_qty in [10, 3] {
        r.set(&StockThreshold {
            item_id: i.id,
            stock_location_id: Some(l.id),
            min_qty,
        })
        .unwrap();
    }
    let t = r.get_for_item(&i.id).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].min_qty, 3);
}
#[test]
fn render_size_matrix_without_panics_control() {
    let (c, a, _, _) = setup();
    let mut app = App::new(&c, a).unwrap();
    for (w, h) in [(1, 1), (20, 8), (80, 24), (100, 30), (120, 35), (160, 50)] {
        for tab in Tab::ALL {
            app.current_tab = tab;
            screen(&mut app, w, h);
        }
        app.open_receive_modal();
        screen(&mut app, w, h);
        app.close_modal();
    }
}

fn cli(dir: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_slim"))
        .current_dir(dir.path())
        .env("SLIM_SESSION_FILE", dir.path().join("session.token"))
        .args(["--db", dir.path().join("audit.sqlite3").to_str().unwrap()])
        .args(args)
        .output()
        .unwrap()
}
fn init_cli() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let out = cli(&dir, &["init"]);
    assert!(out.status.success());
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "owner".into());
    assert!(
        cli(
            &dir,
            &[
                "actor",
                "add",
                "Test owner",
                "--role",
                "admin",
                "--os-user",
                &user
            ]
        )
        .status
        .success()
    );
    dir
}

#[test]
fn pin_protected_actor_cannot_be_impersonated_by_flag() {
    let dir = init_cli();
    for args in [
        vec!["actor", "add", "Protected", "--role", "admin"],
        vec!["actor", "set-pin", "Protected", "--pin", "2468"],
        vec!["item", "add", "--sku", "P", "--name", "Part"],
        vec!["location", "add", "--name", "Drawer", "--kind", "drawer"],
    ] {
        let o = cli(&dir, &args);
        assert!(o.status.success());
    }
    let out = cli(
        &dir,
        &[
            "--actor",
            "Protected",
            "stock",
            "receive",
            "P",
            "--qty",
            "1",
            "--to",
            "Drawer",
        ],
    );
    assert!(
        !out.status.success(),
        "PIN-protected admin accepted without login: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
#[test]
fn viewer_cannot_promote_themselves_to_admin() {
    let dir = init_cli();
    assert!(
        cli(&dir, &["actor", "add", "ReadOnly", "--role", "viewer"])
            .status
            .success()
    );
    let out = cli(
        &dir,
        &[
            "--actor", "ReadOnly", "actor", "set-role", "ReadOnly", "admin",
        ],
    );
    assert!(
        !out.status.success(),
        "viewer self-promotion succeeded: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
#[test]
fn invalid_bin_creation_does_not_leave_orphan_location() {
    let dir = init_cli();
    let bad = cli(
        &dir,
        &[
            "bin", "create", "--code", "BAD", "--width", "0", "--depth", "1", "--height", "3",
        ],
    );
    assert!(!bad.status.success());
    let out = cli(&dir, &["--json", "location", "list"]);
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        rows.as_array().unwrap().is_empty(),
        "failed bin left behind a location: {rows}"
    );
}
#[test]
fn malformed_explicit_config_is_reported() {
    let dir = init_cli();
    let p = dir.path().join("broken.toml");
    std::fs::write(&p, "db_path = [not valid toml").unwrap();
    let out = cli(&dir, &["--config", p.to_str().unwrap(), "item", "list"]);
    assert!(!out.status.success(), "malformed config silently ignored");
}
#[test]
fn duplicate_location_can_be_resolved_using_reported_id() {
    let dir = init_cli();
    for args in [
        vec!["location", "add", "--name", "A", "--kind", "room"],
        vec!["location", "add", "--name", "B", "--kind", "room"],
        vec![
            "location", "add", "--name", "Drawer", "--kind", "drawer", "--parent", "A",
        ],
        vec![
            "location", "add", "--name", "Drawer", "--kind", "drawer", "--parent", "B",
        ],
    ] {
        assert!(cli(&dir, &args).status.success());
    }
    let o = cli(&dir, &["--json", "location", "list"]);
    let rows: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let id = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "Drawer")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let out = cli(
        &dir,
        &[
            "grid", "create", "--code", "BASE", "--parent", id, "--cols", "2", "--rows", "2",
        ],
    );
    assert!(
        out.status.success(),
        "cannot resolve location UUID: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn archived_stock_does_not_disappear_from_tui() {
    let (c, a, i, l) = setup();
    SqliteInventoryService::new(&c)
        .receive(command(&a, &i, &l, 5))
        .unwrap();
    assert!(SqliteItemRepo::new(&c).archive(&i.id).is_err());
    let app = App::new(&c, a).unwrap();
    assert_eq!(
        app.metrics.total_units, 5,
        "five physical units hidden after archiving"
    );
}
#[test]
fn webhook_is_not_successful_without_delivery() {
    use slim_core::model::automation::*;
    use slim_db::repo::{
        automation::{AutomationEngine, AutomationOptions},
        outbox::SqliteOutboxRepo,
    };
    let (c, a, i, l) = setup();
    SqliteInventoryService::new(&c)
        .receive(command(&a, &i, &l, 5))
        .unwrap();
    let event = SqliteOutboxRepo::new(&c)
        .fetch_pending(1)
        .unwrap()
        .remove(0);
    let rule = AutomationRule {
        id: "webhook".into(),
        name: None,
        enabled: true,
        event: "*".into(),
        item_sku: None,
        location_name: None,
        condition: None,
        action: RuleAction::Webhook {
            url: "http://127.0.0.1:1/not-running".into(),
            payload: None,
        },
    };
    let run =
        AutomationEngine::new(&c).execute_action(&rule, &event, &AutomationOptions::default());
    assert_ne!(
        run.status,
        AutomationRunStatus::Success,
        "webhook marked successful without an HTTP request"
    );
}

fn placed_bin(c: &Connection, l: &Location, code: &str) -> slim_core::model::gridfinity::GridBase {
    use slim_core::model::gridfinity::*;
    let repo = slim_db::repo::grid::SqliteGridRepo::new(c);
    let base = repo
        .create_base(&NewGridBase {
            location_id: l.id,
            code: "TEST-BASE".into(),
            cols: 3,
            rows: 2,
            pitch_mm: None,
        })
        .unwrap();
    let bin = repo
        .create_bin(
            &NewGridfinityBin {
                bin_code: code.into(),
                width_units: 1,
                depth_units: 1,
                height_units: 3,
                compartment_count: 1,
                parent_location_id: None,
            },
            &l.id,
        )
        .unwrap();
    repo.place_bin(&PlaceBinCmd {
        bin_id: bin.id,
        base_id: base.id,
        x: 0,
        y: 0,
        rotation: Rotation::Rot0,
    })
    .unwrap();
    base
}
#[test]
fn unicode_bin_code_does_not_crash_ascii_map() {
    let (c, _, _, l) = setup();
    let base = placed_bin(&c, &l, "éé");
    slim_db::repo::grid::SqliteGridRepo::new(&c)
        .render_grid_map(&base.id)
        .unwrap()
        .render_ascii();
}
#[test]
fn unicode_bin_code_does_not_crash_tui() {
    let (c, a, _, l) = setup();
    placed_bin(&c, &l, "éé");
    let mut app = App::new(&c, a).unwrap();
    app.current_tab = Tab::Gridfinity;
    screen(&mut app, 120, 35);
}
#[test]
fn item_label_without_sku_resolves_back_to_same_item() {
    let (c, _, mut i, _) = setup();
    c.execute("UPDATE items SET sku=NULL WHERE id=?1", [i.id.to_string()])
        .unwrap();
    i.sku = None;
    let label = slim_core::label::generate_item_label(&i).unwrap();
    let uri = slim_core::model::scan::item_uri(&i.id.to_string());
    assert!(label.contains(&uri));
    let target = slim_db::repo::scan::SqliteScanResolver::new(&c)
        .resolve_scan(&uri)
        .unwrap();
    assert!(
        matches!(target, slim_core::model::scan::ScanTarget::Item(_)),
        "generated item label is unresolvable: {target:?}"
    );
}
#[test]
fn generated_location_label_resolves_back_to_same_location() {
    let (c, _, _, l) = setup();
    let label = slim_core::label::generate_location_label(&l).unwrap();
    let uri = slim_core::model::scan::location_uri(&l.name);
    assert!(label.contains(&uri));
    let target = slim_db::repo::scan::SqliteScanResolver::new(&c)
        .resolve_scan(&uri)
        .unwrap();
    assert!(
        matches!(target, slim_core::model::scan::ScanTarget::Location(_)),
        "generated location label is unresolvable: {target:?}"
    );
}
#[test]
fn led_respects_location_override_of_global_minimum() {
    let (c, a, i, l) = setup();
    let base = placed_bin(&c, &l, "BIN");
    let t = SqliteThresholdRepo::new(&c);
    t.set(&StockThreshold {
        item_id: i.id,
        stock_location_id: None,
        min_qty: 10,
    })
    .unwrap();
    t.set(&StockThreshold {
        item_id: i.id,
        stock_location_id: Some(l.id),
        min_qty: 2,
    })
    .unwrap();
    SqliteInventoryService::new(&c)
        .receive(command(&a, &i, &l, 5))
        .unwrap();
    assert!(
        SqliteInventoryService::new(&c)
            .get_low_stock()
            .unwrap()
            .is_empty()
    );
    let states = slim_db::repo::led::SqliteLedRepo::new(&c)
        .evaluate_base_leds(&base.id, time::OffsetDateTime::now_utc())
        .unwrap();
    assert!(
        states
            .iter()
            .all(|s| s.mode == slim_core::model::led::LedMode::Off),
        "LED says low stock despite location override: {states:?}"
    );
}
