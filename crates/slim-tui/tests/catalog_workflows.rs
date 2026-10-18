use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use slim_core::{
    model::actor::{NewActor, Role},
    service::{
        ActorRepository, AuthService, GridRepository, ItemRepository, LocationRepository,
        ThresholdRepository,
    },
};
use slim_db::repo::{
    actor::SqliteActorRepo, auth::SqliteAuthService, grid::SqliteGridRepo,
    inventory::SqliteThresholdRepo, item::SqliteItemRepo, location::SqliteLocationRepo,
};
use slim_tui::{
    App,
    app::{Modal, Tab},
    catalog::CatalogKind,
    event::handle_key_event,
};

fn database() -> rusqlite::Connection {
    let conn = slim_db::connection::open_memory().unwrap();
    slim_db::migrate::run_migrations(&conn).unwrap();
    conn
}
fn owner(conn: &rusqlite::Connection) -> App<'_> {
    let mut app = App::new_locked(conn).unwrap();
    fill(&mut app, &["Owner", "1234", "1234"]);
    app.submit_active_modal();
    assert!(app.authenticated);
    app
}
fn fill(app: &mut App, values: &[&str]) {
    let Some(Modal::Catalog(form)) = app.active_modal.as_mut() else {
        panic!("Expected catalog form");
    };
    assert_eq!(form.fields.len(), values.len());
    for (field, value) in form.fields.iter_mut().zip(values) {
        field.value = (*value).into();
    }
}
fn error<'a>(app: &'a App<'_>) -> &'a str {
    let Some(Modal::Catalog(form)) = app.active_modal.as_ref() else {
        panic!("Expected validation error");
    };
    form.error.as_deref().expect("Expected error")
}
fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| slim_tui::ui::render(app, f)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn setup_is_atomic_and_owner_cannot_be_recreated() {
    let conn = database();
    let mut app = App::new_locked(&conn).unwrap();
    fill(&mut app, &["Owner", "1234", "9999"]);
    app.submit_active_modal();
    assert!(!app.authenticated);
    assert!(SqliteActorRepo::new(&conn).list().unwrap().is_empty());
    assert!(error(&app).contains("match"));
    fill(&mut app, &["Owner", "1234", "1234"]);
    app.submit_active_modal();
    assert!(app.authenticated);
    assert_eq!(app.actor.role, Role::Admin);
    assert!(
        SqliteActorRepo::new(&conn)
            .get_pin_hash(&app.actor.id)
            .unwrap()
            .is_some()
    );
    assert_eq!(app.locations[0].location.name, "Main storage");
    app.open_catalog(CatalogKind::Setup);
    fill(&mut app, &["Another owner", "1234", "1234"]);
    app.submit_active_modal();
    assert!(error(&app).contains("already"));
    assert_eq!(SqliteActorRepo::new(&conn).list().unwrap().len(), 1);
}

#[test]
fn locked_screen_requires_pin_and_hides_inventory() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(
        &mut app,
        &["Private catalog item", "PRIVATE", "each", "", "0"],
    );
    app.submit_active_modal();
    let mut locked = App::new_locked(&conn).unwrap();
    assert!(!screen(&mut locked, 100, 30).contains("Private catalog item"));
    locked.submit_active_modal();
    assert!(!locked.authenticated);
    let Some(Modal::User(form)) = locked.active_modal.as_mut() else {
        panic!()
    };
    form.pin = "1234".into();
    locked.submit_active_modal();
    assert!(locked.authenticated);
    assert_eq!(locked.actor.id, app.actor.id);
}

#[test]
fn escape_from_first_run_exits_without_creating_data() {
    let conn = database();
    let mut app = App::new_locked(&conn).unwrap();
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.should_quit);
    assert!(SqliteActorRepo::new(&conn).list().unwrap().is_empty());
}

#[test]
fn catalog_keyboard_does_not_duplicate_release_events() {
    let conn = database();
    let mut app = owner(&conn);
    handle_key_event(
        &mut app,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ),
    );
    let Some(Modal::Catalog(form)) = app.active_modal.as_ref() else {
        panic!()
    };
    assert_eq!(form.fields[0].value, "a");
}

#[test]
fn deliberate_fast_double_letters_are_preserved_in_every_text_form() {
    let conn = database();
    let mut app = owner(&conn);
    for open in ['n', '/', 's', 'r', 'i', 't', 'a'] {
        app.close_modal();
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char(open), KeyModifiers::NONE),
        );
        for _ in 0..2 {
            handle_key_event(
                &mut app,
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            );
            handle_key_event(
                &mut app,
                KeyEvent::new_with_kind(
                    KeyCode::Char('a'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                ),
            );
        }
        let text = match app.active_modal.as_ref().unwrap() {
            Modal::Catalog(f) => &f.fields[0].value,
            Modal::Search(f) => &f.query,
            Modal::Scan(f) => &f.input,
            Modal::Receive(f) => &f.item,
            Modal::Issue(f) => &f.item,
            Modal::Transfer(f) => &f.item,
            Modal::Adjust(f) => &f.item,
            _ => panic!(),
        };
        assert_eq!(text, "aa", "form shortcut {open}");
    }
}

#[test]
fn sidebar_exposes_every_page_and_version_at_80_by_24() {
    let conn = database();
    let mut app = owner(&conn);
    let rendered = screen(&mut app, 80, 24);
    for label in [
        "1: Overview",
        "2: Inventory",
        "3: Locations",
        "4: Gridfinity",
        "5: Low stock",
        "6: Activity",
        "7: Automation",
        "8: Devices",
        env!("CARGO_PKG_VERSION"),
    ] {
        assert!(
            rendered.contains(label),
            "Hidden navigation/version: {label}"
        );
    }
}

#[test]
fn permanent_delete_requires_deleted_view_confirmation_and_admin_at_save() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Erase me", "PURGE", "each", "", "0"]);
    app.submit_active_modal();
    let id = app.items[0].item.id;
    app.switch_tab(Tab::Items);
    app.open_permanent_delete_modal();
    assert!(app.active_modal.is_none());
    SqliteItemRepo::new(&conn)
        .delete_as_admin(&id, &app.actor.id)
        .unwrap();
    app.toggle_deleted();
    handle_key_event(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let rendered = screen(&mut app, 80, 24);
    assert!(rendered.contains("Permanently delete"));
    assert!(rendered.contains("Cannot be undone"));
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(SqliteItemRepo::new(&conn).get_by_id(&id).unwrap().is_some());
    app.open_permanent_delete_modal();
    app.submit_active_modal();
    let Some(Modal::Delete(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    assert!(f.error.is_some());
    f.confirmation = "DELETE PERMANENTLY".into();
    SqliteAuthService::new(&conn)
        .set_role(&app.actor.id, Role::Manager)
        .unwrap();
    app.submit_active_modal();
    let Some(Modal::Delete(f)) = app.active_modal.as_ref() else {
        panic!()
    };
    assert!(f.error.as_ref().unwrap().contains("Admin"));
    app.close_modal();
    app.open_permanent_delete_modal();
    assert!(app.active_modal.is_none());
    SqliteAuthService::new(&conn)
        .set_role(&app.actor.id, Role::Admin)
        .unwrap();
    app.open_permanent_delete_modal();
    let Some(Modal::Delete(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "DELETE PERMANENTLY".into();
    app.submit_active_modal();
    assert!(app.items.is_empty());
    assert!(SqliteItemRepo::new(&conn).get_by_id(&id).unwrap().is_none());
    assert!(app.items_table_state.selected().is_none());
}

#[test]
fn admin_deletes_with_confirmation_and_restores_inside_tui() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Disposable test item", "TEST", "each", "", "0"]);
    app.submit_active_modal();
    app.switch_tab(Tab::Items);
    handle_key_event(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    assert!(screen(&mut app, 80, 24).contains("Delete item"));
    app.submit_active_modal();
    assert_eq!(app.items.len(), 1);
    let Some(Modal::Delete(form)) = app.active_modal.as_mut() else {
        panic!()
    };
    form.confirmation = "DELETE".into();
    app.submit_active_modal();
    assert!(app.items.is_empty());
    assert!(app.items_table_state.selected().is_none());
    app.toggle_deleted();
    assert_eq!(app.items.len(), 1);
    app.open_delete_modal(true);
    let Some(Modal::Delete(form)) = app.active_modal.as_mut() else {
        panic!()
    };
    form.confirmation = "RESTORE".into();
    app.submit_active_modal();
    assert!(app.items.is_empty());
    app.toggle_deleted();
    assert_eq!(app.items.len(), 1);
}

#[test]
fn non_admin_cannot_open_delete_and_permission_is_rechecked_on_submit() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Test item", "TEST", "each", "", "0"]);
    app.submit_active_modal();
    app.switch_tab(Tab::Items);
    app.open_delete_modal(false);
    let Some(Modal::Delete(form)) = app.active_modal.as_mut() else {
        panic!()
    };
    form.confirmation = "DELETE".into();
    SqliteAuthService::new(&conn)
        .set_role(&app.actor.id, Role::Manager)
        .unwrap();
    app.submit_active_modal();
    let Some(Modal::Delete(form)) = app.active_modal.as_ref() else {
        panic!()
    };
    assert!(form.error.as_ref().unwrap().contains("Admin"));
    assert_eq!(app.items.len(), 1);
    app.close_modal();
    app.open_delete_modal(false);
    assert!(app.active_modal.is_none());
}

#[test]
fn escape_cancels_delete_without_changing_inventory() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Keep me", "KEEP", "each", "", "0"]);
    app.submit_active_modal();
    app.switch_tab(Tab::Items);
    app.open_delete_modal(false);
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.active_modal.is_none());
    assert_eq!(app.items.len(), 1);
}

#[test]
fn stock_form_completes_locations_as_unambiguous_paths() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_receive_modal();
    handle_key_event(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    handle_key_event(&mut app, KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
    let Some(Modal::Receive(form)) = app.active_modal.as_ref() else {
        panic!()
    };
    assert_eq!(form.location, "Main storage");
}

#[test]
fn item_create_and_edit_include_minimum_and_sku() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Screws", "M3", "each", "Steel", "20"]);
    app.submit_active_modal();
    assert!(app.active_modal.is_none());
    let id = app.items[0].item.id;
    app.switch_tab(Tab::Items);
    app.open_edit_catalog();
    fill(
        &mut app,
        &["Socket screws", "M3-10", "each", "Black steel", "25"],
    );
    app.submit_active_modal();
    let item = SqliteItemRepo::new(&conn).get_by_id(&id).unwrap().unwrap();
    assert_eq!(item.name, "Socket screws");
    assert_eq!(item.sku.as_deref(), Some("M3-10"));
    assert_eq!(
        SqliteThresholdRepo::new(&conn).get_for_item(&id).unwrap()[0].min_qty,
        25
    );
    assert_eq!(app.low_stock_alerts[0].min_qty, 25);
}

#[test]
fn invalid_item_edit_and_duplicate_bin_leave_no_partial_changes() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Screws", "M3", "each", "", "0"]);
    app.submit_active_modal();
    app.open_catalog(CatalogKind::Item(Some(app.items[0].item.id)));
    fill(&mut app, &["Wrong name", "M3", "each", "", "-1"]);
    app.submit_active_modal();
    assert!(error(&app).contains("whole number"));
    assert_eq!(
        SqliteItemRepo::new(&conn)
            .get_by_sku("M3")
            .unwrap()
            .unwrap()
            .name,
        "Screws"
    );
    for _ in 0..2 {
        app.open_catalog(CatalogKind::Bin);
        fill(&mut app, &["BIN-A", "", "1", "1", "3", "1"]);
        app.submit_active_modal();
    }
    assert!(app.active_modal.is_some());
    assert_eq!(
        SqliteLocationRepo::new(&conn)
            .list_children(None)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(SqliteGridRepo::new(&conn).list_bins().unwrap().len(), 1);
}

#[test]
fn create_and_rename_locations_and_place_bin() {
    let conn = database();
    let mut app = owner(&conn);
    app.switch_tab(Tab::Locations);
    app.open_catalog(CatalogKind::Location(None));
    fill(&mut app, &["Drawer", "drawer", "Main storage"]);
    app.submit_active_modal();
    let id = app
        .locations
        .iter()
        .find(|l| l.location.name == "Drawer")
        .unwrap()
        .location
        .id;
    app.open_catalog(CatalogKind::Location(Some(id)));
    fill(&mut app, &["Drawer A"]);
    app.submit_active_modal();
    assert!(
        app.locations
            .iter()
            .any(|l| l.path == "Main storage > Drawer A")
    );
    app.open_catalog(CatalogKind::Base);
    fill(&mut app, &["BASE-A", "Main storage > Drawer A", "4", "4"]);
    app.submit_active_modal();
    app.open_catalog(CatalogKind::Bin);
    fill(&mut app, &["BIN-A", "Main storage", "2", "1", "3", "1"]);
    app.submit_active_modal();
    app.open_catalog(CatalogKind::Placement);
    fill(&mut app, &["BIN-A", "BASE-A", "1", "2", "90"]);
    app.submit_active_modal();
    assert!(app.active_modal.is_none());
    assert_eq!(
        SqliteGridRepo::new(&conn)
            .get_active_placements(&app.grid_bases[0].id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn permission_changes_are_checked_at_save_and_login() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Item(None));
    fill(&mut app, &["Forbidden", "", "each", "", "0"]);
    SqliteAuthService::new(&conn)
        .set_role(&app.actor.id, Role::Viewer)
        .unwrap();
    app.submit_active_modal();
    assert!(error(&app).contains("Manager"));
    assert!(app.items.is_empty());
    let mut locked = App::new_locked(&conn).unwrap();
    SqliteAuthService::new(&conn)
        .set_role(&app.actor.id, Role::Operator)
        .unwrap();
    let Some(Modal::User(form)) = locked.active_modal.as_mut() else {
        panic!()
    };
    form.pin = "1234".into();
    locked.submit_active_modal();
    assert_eq!(locked.actor.role, Role::Operator);
    locked.open_catalog(CatalogKind::Item(None));
    assert!(locked.active_modal.is_none());
}

#[test]
fn admin_can_add_pin_protected_person_but_manager_cannot() {
    let conn = database();
    let mut app = owner(&conn);
    app.open_catalog(CatalogKind::Person);
    fill(&mut app, &["Operator", "operator", "4321", "4321"]);
    app.submit_active_modal();
    let person = SqliteActorRepo::new(&conn)
        .get_by_name("Operator")
        .unwrap()
        .unwrap();
    assert_eq!(person.role, Role::Operator);
    assert!(
        SqliteAuthService::new(&conn)
            .login_with_pin("Operator", "4321")
            .is_ok()
    );
    let manager = SqliteActorRepo::new(&conn)
        .create(&NewActor::new("Manager").with_role(Role::Manager))
        .unwrap();
    let mut manager_app = App::new(&conn, manager).unwrap();
    manager_app.open_catalog(CatalogKind::Person);
    assert!(manager_app.active_modal.is_none());
}

#[test]
fn forms_scroll_to_focus_and_mask_pins_at_standard_terminal_size() {
    let conn = database();
    let mut app = App::new_locked(&conn).unwrap();
    fill(&mut app, &["Owner", "secret-1234", "secret-1234"]);
    let rendered = screen(&mut app, 80, 24);
    assert!(rendered.contains("Confirm PIN"));
    assert!(!rendered.contains("secret-1234"));
    app.submit_active_modal();
    app.open_catalog(CatalogKind::Bin);
    for _ in 0..5 {
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    let rendered = screen(&mut app, 80, 24);
    assert!(rendered.contains("Compartments"));
    assert!(rendered.contains("Field 6 of 6"));
    for (w, h) in [(40, 18), (80, 24), (120, 40)] {
        screen(&mut app, w, h);
    }
}

#[test]
fn backup_is_consistent_and_preserves_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inventory.sqlite3");
    let conn = slim_db::connection::open_connection(&path).unwrap();
    slim_db::migrate::run_migrations(&conn).unwrap();
    let mut app = owner(&conn);
    app.create_backup();
    assert!(
        app.notification
            .as_ref()
            .unwrap()
            .message
            .starts_with("Backup: ")
    );
    let backup_path = std::fs::read_dir(dir.path().join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup = rusqlite::Connection::open(backup_path).unwrap();
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM actors", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
