//! Headless integration tests for SLIM Terminal User Interface (TUI).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rusqlite::Connection;
use slim_core::model::actor::NewActor;
use slim_core::model::item::NewItem;
use slim_core::model::location::{LocationKind, NewLocation};
use slim_core::model::stock::StockThreshold;
use slim_core::service::{
    ActorRepository, InventoryService, ItemRepository, LocationRepository, ThresholdRepository,
};
use slim_db::migrate::run_migrations;
use slim_db::repo::actor::SqliteActorRepo;
use slim_db::repo::inventory::{SqliteInventoryService, SqliteThresholdRepo};
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;
use slim_tui::app::{App, NotificationLevel, Tab};
use slim_tui::event::handle_key_event;
use slim_tui::ui;

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )
    .unwrap();
    run_migrations(&conn).unwrap();
    conn
}

fn populate_sample_data(
    conn: &Connection,
) -> (
    slim_core::model::actor::Actor,
    slim_core::model::item::Item,
    slim_core::model::location::Location,
) {
    let actor_repo = SqliteActorRepo::new(conn);
    let item_repo = SqliteItemRepo::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);

    let actor = actor_repo
        .create(
            &NewActor::new("Alice Operator")
                .with_os_username("alice")
                .with_role(slim_core::model::actor::Role::Manager),
        )
        .unwrap();

    let item = item_repo
        .create(&NewItem {
            sku: Some("M3X10".to_string()),
            name: "M3x10 Machine Screw".to_string(),
            unit: "each".to_string(),
            description: Some("Hex socket cap screw".to_string()),
            metadata: None,
        })
        .unwrap();

    let workshop = loc_repo
        .create(&NewLocation {
            parent_id: None,
            name: "Workshop".to_string(),
            kind: LocationKind::Room,
        })
        .unwrap();

    let drawer = loc_repo
        .create(&NewLocation {
            parent_id: Some(workshop.id),
            name: "Drawer 1".to_string(),
            kind: LocationKind::Drawer,
        })
        .unwrap();

    (actor, item, drawer)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_tui_app_initialization() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let app = App::new(&conn, actor).unwrap();
    assert_eq!(app.current_tab, Tab::Dashboard);
    assert_eq!(app.metrics.total_items, 1);
    assert_eq!(app.metrics.total_units, 0);
    assert_eq!(app.metrics.total_locations, 2);
    assert_eq!(app.items.len(), 1);
}

#[test]
fn test_tui_render_dashboard() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    // Verify key titles appear in rendered terminal buffer
    let text = format!("{:?}", buffer);
    assert!(text.contains("Items"));
    assert!(text.contains("Stock on hand"));
    assert!(text.contains("Recent Inventory Activity"));
    assert!(text.contains("Alice Operator"));
}

#[test]
fn test_tui_render_items_tab() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Items);

    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let text = format!("{:?}", buffer);
    assert!(text.contains("Catalog Items"));
    assert!(text.contains("M3X10"));
    assert!(text.contains("M3x10 Machine Screw"));
    assert!(text.contains("Item Details"));
}

#[test]
fn test_tui_render_locations_tab() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Locations);

    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let text = format!("{:?}", buffer);
    assert!(text.contains("Location Hierarchy"));
    assert!(text.contains("Workshop"));
    assert!(text.contains("Drawer 1"));
    assert!(text.contains("Location Info"));
}

#[test]
fn test_tui_render_low_stock_tab() {
    let conn = setup_test_db();
    let (actor, item, loc) = populate_sample_data(&conn);

    let thresh_repo = SqliteThresholdRepo::new(&conn);
    thresh_repo
        .set(&StockThreshold {
            item_id: item.id,
            stock_location_id: None,
            min_qty: 50,
        })
        .unwrap();

    let inv_service = SqliteInventoryService::new(&conn);
    inv_service
        .receive(slim_core::model::operation::ReceiveStock {
            actor_id: actor.id,
            item_id: item.id,
            destination_id: loc.id,
            quantity: 20,
            source: slim_core::model::operation::OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::LowStock);

    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let text = format!("{:?}", buffer);
    assert!(text.contains("Low Stock Alerts"));
    assert!(text.contains("M3X10"));
    assert!(text.contains("-30")); // deficit of 30 (50 - 20)
}

#[test]
fn test_tui_tab_navigation() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    assert_eq!(app.current_tab, Tab::Dashboard);

    handle_key_event(&mut app, key(KeyCode::Tab));
    assert_eq!(app.current_tab, Tab::Items);

    handle_key_event(&mut app, key(KeyCode::Tab));
    assert_eq!(app.current_tab, Tab::Locations);

    handle_key_event(&mut app, key(KeyCode::BackTab));
    assert_eq!(app.current_tab, Tab::Items);

    handle_key_event(&mut app, key(KeyCode::Char('4')));
    assert_eq!(app.current_tab, Tab::Gridfinity);

    handle_key_event(&mut app, key(KeyCode::Char('5')));
    assert_eq!(app.current_tab, Tab::LowStock);

    handle_key_event(&mut app, key(KeyCode::Char('6')));
    assert_eq!(app.current_tab, Tab::History);

    handle_key_event(&mut app, key(KeyCode::Char('1')));
    assert_eq!(app.current_tab, Tab::Dashboard);
}

#[test]
fn test_tui_table_navigation() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);
    let item_repo = SqliteItemRepo::new(&conn);

    // Add a second item
    item_repo
        .create(&NewItem {
            sku: Some("ESP32".to_string()),
            name: "ESP32 DevKit".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Items);
    assert_eq!(app.items_table_state.selected(), Some(0));

    handle_key_event(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.items_table_state.selected(), Some(1));

    handle_key_event(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.items_table_state.selected(), Some(0));
}

#[test]
fn test_tui_receive_modal_flow() {
    let conn = setup_test_db();
    let (actor, item, loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Items);

    // Press 'r' to open Receive modal
    handle_key_event(&mut app, key(KeyCode::Char('r')));
    assert!(app.active_modal.is_some());

    // Enter destination location
    handle_key_event(&mut app, key(KeyCode::Tab)); // focus field 1: location
    for c in loc.name.chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Enter quantity
    handle_key_event(&mut app, key(KeyCode::Tab)); // focus field 2: qty
    for c in "100".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Enter reason
    handle_key_event(&mut app, key(KeyCode::Tab)); // focus field 3: reason
    for c in "Restock".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Submit form with Enter
    handle_key_event(&mut app, key(KeyCode::Enter));

    // Verify modal closed and success notification was set
    assert!(app.active_modal.is_none());
    assert!(app.notification.is_some());
    let notif = app.notification.unwrap();
    assert!(notif.message.contains("Received 100"));

    // Verify database was actually mutated
    let inv_service = SqliteInventoryService::new(&conn);
    let bal = inv_service.get_balance(&item.id, &loc.id).unwrap().unwrap();
    assert_eq!(bal.qty, 100);
}

#[test]
fn test_tui_issue_modal_flow() {
    let conn = setup_test_db();
    let (actor, item, loc) = populate_sample_data(&conn);

    // Initial receive of 50
    let inv_service = SqliteInventoryService::new(&conn);
    inv_service
        .receive(slim_core::model::operation::ReceiveStock {
            actor_id: actor.id,
            item_id: item.id,
            destination_id: loc.id,
            quantity: 50,
            source: slim_core::model::operation::OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Items);

    // Press 'i' to open Issue modal
    handle_key_event(&mut app, key(KeyCode::Char('i')));
    assert!(app.active_modal.is_some());

    // Enter source location
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in loc.name.chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Enter quantity: 15
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in "15".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Submit
    handle_key_event(&mut app, key(KeyCode::Enter));

    assert!(app.active_modal.is_none());
    let bal = inv_service.get_balance(&item.id, &loc.id).unwrap().unwrap();
    assert_eq!(bal.qty, 35);
}

#[test]
fn test_tui_transfer_modal_flow() {
    let conn = setup_test_db();
    let (actor, item, loc1) = populate_sample_data(&conn);

    let loc_repo = SqliteLocationRepo::new(&conn);
    let loc2 = loc_repo
        .create(&NewLocation {
            parent_id: None,
            name: "Assembly Bench".to_string(),
            kind: LocationKind::Shelf,
        })
        .unwrap();

    let inv_service = SqliteInventoryService::new(&conn);
    inv_service
        .receive(slim_core::model::operation::ReceiveStock {
            actor_id: actor.id,
            item_id: item.id,
            destination_id: loc1.id,
            quantity: 100,
            source: slim_core::model::operation::OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Items);

    // Press 't' to open Transfer modal
    handle_key_event(&mut app, key(KeyCode::Char('t')));
    assert!(app.active_modal.is_some());

    // Source location: Drawer 1
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in loc1.name.chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Destination location: Assembly Bench
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in loc2.name.chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Quantity: 30
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in "30".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Submit
    handle_key_event(&mut app, key(KeyCode::Enter));

    assert!(app.active_modal.is_none());
    assert_eq!(
        inv_service
            .get_balance(&item.id, &loc1.id)
            .unwrap()
            .unwrap()
            .qty,
        70
    );
    assert_eq!(
        inv_service
            .get_balance(&item.id, &loc2.id)
            .unwrap()
            .unwrap()
            .qty,
        30
    );
}

#[test]
fn test_tui_adjust_modal_flow() {
    let conn = setup_test_db();
    let (actor, item, loc) = populate_sample_data(&conn);

    let inv_service = SqliteInventoryService::new(&conn);
    inv_service
        .receive(slim_core::model::operation::ReceiveStock {
            actor_id: actor.id,
            item_id: item.id,
            destination_id: loc.id,
            quantity: 100,
            source: slim_core::model::operation::OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    app.switch_tab(Tab::Items);

    handle_key_event(&mut app, key(KeyCode::Char('a')));
    assert!(app.active_modal.is_some());

    // Location
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in loc.name.chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // New quantity: 88
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in "88".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Reason (mandatory)
    handle_key_event(&mut app, key(KeyCode::Tab));
    for c in "Inventory audit".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Submit
    handle_key_event(&mut app, key(KeyCode::Enter));

    assert!(app.active_modal.is_none());
    assert_eq!(
        inv_service
            .get_balance(&item.id, &loc.id)
            .unwrap()
            .unwrap()
            .qty,
        88
    );
}

#[test]
fn test_tui_search_modal_flow() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    assert_eq!(app.current_tab, Tab::Dashboard);

    // Open search modal with '/'
    handle_key_event(&mut app, key(KeyCode::Char('/')));
    assert!(app.active_modal.is_some());

    for c in "M3X".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Enter to submit search
    handle_key_event(&mut app, key(KeyCode::Enter));

    assert!(app.active_modal.is_none());
    assert_eq!(app.current_tab, Tab::Items);
    assert_eq!(app.items_table_state.selected(), Some(0));
}

#[test]
fn test_tui_help_modal() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    handle_key_event(&mut app, key(KeyCode::Char('?')));
    assert!(app.active_modal.is_some());

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let text = format!("{:?}", buffer);
    assert!(text.contains("Keyboard Shortcuts"));

    // Close with Esc
    handle_key_event(&mut app, key(KeyCode::Esc));
    assert!(app.active_modal.is_none());
}

#[test]
fn test_tui_render_gridfinity_tab() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    app.switch_tab(Tab::Gridfinity);
    terminal.draw(|f| ui::render(&mut app, f)).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let text = format!("{:?}", buffer);
    assert!(text.contains("Gridfinity Baseplates") || text.contains("Gridfinity"));
}

#[test]
fn test_tui_scan_modal_flow() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let mut app = App::new(&conn, actor).unwrap();
    assert_eq!(app.current_tab, Tab::Dashboard);

    // Open scan modal with 's'
    handle_key_event(&mut app, key(KeyCode::Char('s')));
    assert!(app.active_modal.is_some());

    // Enter SKU "M3X10"
    for c in "M3X10".chars() {
        handle_key_event(&mut app, key(KeyCode::Char(c)));
    }

    // Submit with Enter
    handle_key_event(&mut app, key(KeyCode::Enter));

    assert!(app.active_modal.is_none());
    assert_eq!(app.current_tab, Tab::Items);
    assert_eq!(app.items_table_state.selected(), Some(0));
    assert!(app.notification.is_some());
    let notif = app.notification.unwrap();
    assert_eq!(notif.level, NotificationLevel::Success);
    assert!(notif.message.contains("Scanned Item"));
}

#[test]
fn test_tui_role_header_and_user_modal_flow() {
    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    // Create a second actor
    let actor_repo = SqliteActorRepo::new(&conn);
    let bob = actor_repo
        .create(
            &NewActor::new("Bob Admin")
                .with_os_username("bob")
                .with_role(slim_core::model::actor::Role::Admin),
        )
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // Verify header renders role badge
    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let text = format!("{:?}", buffer);
    assert!(text.contains("Alice Operator"));
    assert!(text.contains("[manager]"));

    // Open user modal with 'u'
    handle_key_event(&mut app, key(KeyCode::Char('u')));
    assert!(matches!(
        app.active_modal,
        Some(slim_tui::app::Modal::User(_))
    ));

    // Navigate to Bob
    handle_key_event(&mut app, key(KeyCode::Down));

    // Submit user switch
    handle_key_event(&mut app, key(KeyCode::Enter));

    assert!(app.active_modal.is_none());
    assert_eq!(app.actor.id, bob.id);
    assert_eq!(app.actor.display_name, "Bob Admin");
    assert_eq!(app.actor.role, slim_core::model::actor::Role::Admin);
}

#[test]
fn test_tui_adjust_blocked_for_operator() {
    let conn = setup_test_db();
    let actor_repo = SqliteActorRepo::new(&conn);
    let operator = actor_repo
        .create(&NewActor::new("Ollie Operator").with_role(slim_core::model::actor::Role::Operator))
        .unwrap();

    let mut app = App::new(&conn, operator).unwrap();

    // Attempt to open adjust modal with 'a'
    handle_key_event(&mut app, key(KeyCode::Char('a')));

    // Should NOT open modal, and should set warning notification
    assert!(app.active_modal.is_none());
    assert!(app.notification.is_some());
    let notif = app.notification.unwrap();
    assert_eq!(notif.level, NotificationLevel::Warning);
    assert!(notif.message.contains("cannot adjust inventory"));
}

#[test]
fn test_tui_render_automation_tab() {
    use slim_core::service::AutomationRepository;

    let conn = setup_test_db();
    let (actor, _item, _loc) = populate_sample_data(&conn);

    let auto_repo = slim_db::repo::automation::SqliteAutomationRepo::new(&conn);
    auto_repo
        .save_rule(&slim_core::model::automation::AutomationRule {
            id: "rule-auto-1".to_string(),
            name: Some("Test Low Stock Auto".to_string()),
            enabled: true,
            event: "stock.below_threshold".to_string(),
            item_sku: Some("M3X10".to_string()),
            location_name: None,
            condition: None,
            action: slim_core::model::automation::RuleAction::Notify {
                message: "Alert low stock".to_string(),
            },
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    // Switch to Tab 7 (Automation) via key '7'
    handle_key_event(&mut app, key(KeyCode::Char('7')));
    assert_eq!(app.current_tab, Tab::Automation);

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let text = format!("{:?}", buffer);
    assert!(text.contains("Automation Rules"));
    assert!(text.contains("rule-auto-1"));
    assert!(text.contains("stock.below_threshold"));
    assert!(text.contains("Execution History"));
}

#[test]
fn test_tui_render_devices_tab() {
    use slim_core::model::device::{DeviceKind, DeviceObservation, NewDevice};
    use slim_core::service::{DeviceRepository, ObservationRepository};

    let conn = setup_test_db();
    let (actor, _item, loc) = populate_sample_data(&conn);

    let dev_repo = slim_db::repo::device::SqliteDeviceRepo::new(&conn);
    dev_repo
        .register_device(&NewDevice {
            id: "scanner-tui-1".to_string(),
            name: "Main Bench Scanner".to_string(),
            kind: DeviceKind::SerialScanner,
            port_or_address: Some("COM4".to_string()),
            location_id: Some(loc.id),
            settings_json: None,
        })
        .unwrap();

    dev_repo
        .record_observation(
            "scanner-tui-1",
            &DeviceObservation::IdentifierScanned {
                value: "M3X10".to_string(),
            },
        )
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    // Switch to Tab 8 (Devices) via key '8'
    handle_key_event(&mut app, key(KeyCode::Char('8')));
    assert_eq!(app.current_tab, Tab::Devices);

    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    let text = format!("{:?}", buffer);
    assert!(text.contains("Hardware Devices"));
    assert!(text.contains("scanner-tui-1"));
    assert!(text.contains("Main Bench Scanner"));
    assert!(text.contains("Observations Queue"));
    assert!(text.contains("Label scanned"));
}

#[test]
fn test_tui_device_confirm_reject() {
    use slim_core::model::device::{DeviceKind, DeviceObservation, NewDevice, ObservationStatus};
    use slim_core::service::{DeviceRepository, ObservationRepository};

    let conn = setup_test_db();
    let (actor, _item, loc) = populate_sample_data(&conn);

    let dev_repo = slim_db::repo::device::SqliteDeviceRepo::new(&conn);
    dev_repo
        .register_device(&NewDevice {
            id: "ir-tui-1".to_string(),
            name: "IR Sensor 1".to_string(),
            kind: DeviceKind::IrBreakbeam,
            port_or_address: None,
            location_id: Some(loc.id),
            settings_json: None,
        })
        .unwrap();

    let obs1 = dev_repo
        .record_observation(
            "ir-tui-1",
            &DeviceObservation::BeamInterrupted {
                zone: "ZONE-A".to_string(),
            },
        )
        .unwrap();

    let obs2 = dev_repo
        .record_observation(
            "ir-tui-1",
            &DeviceObservation::BeamInterrupted {
                zone: "ZONE-B".to_string(),
            },
        )
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();

    // Switch to Tab 8
    handle_key_event(&mut app, key(KeyCode::Char('8')));
    assert_eq!(app.current_tab, Tab::Devices);

    // Initial selected should be first observation (obs2 since ordered created_at DESC)
    assert_eq!(app.observations_table_state.selected(), Some(0));

    // Confirm selected observation with 'c'
    handle_key_event(&mut app, key(KeyCode::Char('c')));
    assert!(app.notification.is_some());
    let notif = app.notification.clone().unwrap();
    assert_eq!(notif.level, NotificationLevel::Success);
    assert!(notif.message.contains("confirmed"));

    // Verify in db
    let stored1 = dev_repo.get_observation(&obs2.id).unwrap().unwrap();
    assert_eq!(stored1.status, ObservationStatus::Confirmed);

    // Move selection down to next observation and reject with 'x'
    handle_key_event(&mut app, key(KeyCode::Down));
    assert_eq!(app.observations_table_state.selected(), Some(1));

    handle_key_event(&mut app, key(KeyCode::Char('x')));
    let stored2 = dev_repo.get_observation(&obs1.id).unwrap().unwrap();
    assert_eq!(stored2.status, ObservationStatus::Rejected);
}

#[test]
fn test_tui_gridfinity_led_rendering_and_locate_shortcut() {
    use slim_core::model::gridfinity::{NewGridBase, NewGridfinityBin, PlaceBinCmd, Rotation};
    use slim_core::model::led::{DockLedState, LedMode};
    use slim_core::service::{GridRepository, LedRepository};
    use slim_db::repo::grid::SqliteGridRepo;
    use slim_db::repo::led::SqliteLedRepo;
    use time::OffsetDateTime;

    let conn = setup_test_db();
    let (actor, _item, loc) = populate_sample_data(&conn);

    let grid_repo = SqliteGridRepo::new(&conn);
    let base = grid_repo
        .create_base(&NewGridBase {
            code: "BASE-ALPHA".to_string(),
            location_id: loc.id,
            cols: 3,
            rows: 3,
            pitch_mm: None,
        })
        .unwrap();

    let loc_repo = slim_db::repo::location::SqliteLocationRepo::new(&conn);
    let bin_loc = loc_repo
        .create(&slim_core::model::location::NewLocation {
            parent_id: Some(loc.id),
            name: "Bin Loc 01".to_string(),
            kind: slim_core::model::location::LocationKind::Bin,
        })
        .unwrap();

    let bin = grid_repo
        .create_bin(
            &NewGridfinityBin {
                bin_code: "BIN-01".to_string(),
                width_units: 2,
                depth_units: 1,
                height_units: 3,
                compartment_count: 1,
                parent_location_id: None,
            },
            &bin_loc.id,
        )
        .unwrap();

    grid_repo
        .place_bin(&PlaceBinCmd {
            bin_id: bin.id,
            base_id: base.id,
            x: 0,
            y: 0,
            rotation: Rotation::Rot0,
        })
        .unwrap();

    let mut app = App::new(&conn, actor).unwrap();
    // Switch to Gridfinity tab (tab index 4 / key '4')
    handle_key_event(&mut app, key(KeyCode::Char('4')));
    assert_eq!(app.current_tab, Tab::Gridfinity);
    assert_eq!(app.grid_bases.len(), 1);
    assert!(app.active_base_grid_map.is_some());

    // Initially, no LEDs set (idle / off)
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let text = format!("{:?}", buffer);
    assert!(text.contains("Storage Map"));
    assert!(text.contains("BASE-ALPHA"));
    assert!(text.contains("BIN-01"));
    assert!(text.contains("OFF"));

    // Trigger locate via 'l' shortcut on the selected bin
    handle_key_event(&mut app, key(KeyCode::Char('l')));
    assert!(app.notification.is_some());
    let notif = app.notification.clone().unwrap();
    assert_eq!(notif.level, NotificationLevel::Success);
    assert!(notif.message.contains("Illuminated") && notif.message.contains("BIN-01"));

    // Redraw: verify Locate LED indicator appears in the 2D layout and placed bins table
    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer2 = terminal.backend().buffer().clone();
    let text2 = format!("{:?}", buffer2);
    assert!(text2.contains("[LOCATE]"));
    assert!(text2.contains("[*"));

    // Now simulate a missing bin LED state and low-stock LED state directly via SqliteLedRepo
    let led_repo = SqliteLedRepo::new(&conn);
    let now = OffsetDateTime::now_utc();
    led_repo
        .set_dock_led(&DockLedState {
            dock_id: "BASE-ALPHA:2:2".to_string(),
            base_id: base.id,
            x: 2,
            y: 2,
            mode: LedMode::Missing,
            r: 255,
            g: 40,
            b: 0,
            expires_at: None,
            reason: Some("Bin missing".to_string()),
            updated_at: now,
        })
        .unwrap();

    led_repo
        .set_dock_led(&DockLedState {
            dock_id: "BASE-ALPHA:2:0".to_string(),
            base_id: base.id,
            x: 2,
            y: 0,
            mode: LedMode::LowStock,
            r: 255,
            g: 180,
            b: 0,
            expires_at: None,
            reason: Some("Stock low".to_string()),
            updated_at: now,
        })
        .unwrap();

    // Refresh app data and render again
    app.refresh_data().unwrap();
    terminal.draw(|f| ui::render(&mut app, f)).unwrap();
    let buffer3 = terminal.backend().buffer().clone();
    let text3 = format!("{:?}", buffer3);
    assert!(text3.contains("[!MIS!]"));
    assert!(text3.contains("[?LOW?]"));
}
