use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::json;
use slim_core::{model::actor::*, service::ActorRepository};
use slim_db::repo::{
    actor::SqliteActorRepo,
    catalog::{CatalogKind as Kind, SqliteCatalogService},
};
use slim_tui::{
    App,
    app::{Modal, Tab},
    event::handle_key_event,
};
fn database() -> rusqlite::Connection {
    let c = slim_db::connection::open_memory().unwrap();
    slim_db::migrate::run_migrations(&c).unwrap();
    c
}
fn setup(c: &rusqlite::Connection) -> App<'_> {
    let a = SqliteActorRepo::new(c)
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    App::new(c, a).unwrap()
}
fn key(app: &mut App, key: KeyCode) {
    handle_key_event(app, KeyEvent::new(key, KeyModifiers::NONE));
}
fn screen(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| slim_tui::ui::render(app, f)).unwrap();
    let b = terminal.backend().buffer();
    (0..24)
        .map(|y| (0..80).map(|x| b[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn custom_storage_can_be_created_edited_deleted_restored_and_purged_in_tui() {
    let c = database();
    let mut app = setup(&c);
    app.switch_tab(Tab::Locations);
    key(&mut app, KeyCode::Char('n'));
    let Some(Modal::RecordEditor(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    for (key, _, value) in &mut f.fields {
        *value = match *key {
            "name" => "Tool chest",
            "kind" => "custom",
            "type_label" => "Rolling cabinet",
            "notes" => "Garage",
            "width_mm" => "600",
            _ => value.as_str(),
        }
        .into();
    }
    app.submit_active_modal();
    let Some(Modal::Directory(f)) = app.active_modal.as_ref() else {
        panic!()
    };
    let id = f.records[0].id.clone();
    key(&mut app, KeyCode::Char('e'));
    let Some(Modal::RecordEditor(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.fields[0].2 = "Renamed toolbox".into();
    app.submit_active_modal();
    key(&mut app, KeyCode::Char('d'));
    app.submit_active_modal();
    let Some(Modal::RecordAction(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    assert!(f.error.is_some());
    f.confirmation = "DELETE".into();
    app.submit_active_modal();
    assert!(screen(&mut app).contains("Deleted"));
    key(&mut app, KeyCode::Char('R'));
    let Some(Modal::RecordAction(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "RESTORE".into();
    app.submit_active_modal();
    key(&mut app, KeyCode::Char('d'));
    let Some(Modal::RecordAction(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "DELETE".into();
    app.submit_active_modal();
    key(&mut app, KeyCode::Char('d'));
    assert!(screen(&mut app).contains("Cannot be undone"));
    key(&mut app, KeyCode::Esc);
    assert!(
        SqliteCatalogService::new(&c)
            .get(Kind::Storage, &id)
            .is_ok()
    );
    key(&mut app, KeyCode::Char('d'));
    let Some(Modal::RecordAction(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "DELETE PERMANENTLY".into();
    app.submit_active_modal();
    assert!(
        SqliteCatalogService::new(&c)
            .get(Kind::Storage, &id)
            .is_err()
    );
}
#[test]
fn directory_navigation_and_record_fields_ignore_key_release() {
    let c = database();
    let mut app = setup(&c);
    key(&mut app, KeyCode::Char('m'));
    key(&mut app, KeyCode::Right);
    let Some(Modal::Directory(f)) = &app.active_modal else {
        panic!()
    };
    assert_eq!(f.kind, Kind::Base);
    key(&mut app, KeyCode::Char('n'));
    key(&mut app, KeyCode::Char('a'));
    handle_key_event(
        &mut app,
        KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ),
    );
    let Some(Modal::RecordEditor(f)) = &app.active_modal else {
        panic!()
    };
    assert_eq!(f.fields[0].2, "a");
    app.close_modal();
    app.open_record_editor(Kind::Storage, None);
    for _ in 0..7 {
        key(&mut app, KeyCode::Tab);
    }
    assert!(screen(&mut app).contains("Height mm"));
}
#[test]
fn removed_users_can_be_permanently_deleted_through_people() {
    let c = database();
    let mut app = setup(&c);
    let repo = SqliteActorRepo::new(&c);
    let user = repo
        .create_as_admin(&app.actor.id, &NewActor::new("Staff"), Some("test-pin"))
        .unwrap();
    repo.update_as_admin(
        &app.actor.id,
        &user.id,
        &ActorUpdate {
            active: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    app.open_people_modal();
    let Some(Modal::People(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.selected = f.users.iter().position(|a| a.id == user.id).unwrap();
    key(&mut app, KeyCode::Char('d'));
    assert!(screen(&mut app).contains("Permanently delete user"));
    let Some(Modal::RemoveUser(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "DELETE PERMANENTLY".into();
    app.submit_active_modal();
    assert!(repo.get_by_id(&user.id).unwrap().is_none());
}
#[test]
fn revoked_admin_cannot_submit_pending_catalog_delete() {
    let c = database();
    let mut app = setup(&c);
    let service = SqliteCatalogService::new(&c);
    let r = service
        .save(
            &app.actor.id,
            Kind::Storage,
            None,
            &json!({"name":"Box","kind":"custom"}),
        )
        .unwrap();
    app.open_directory(Kind::Storage, false);
    app.directory_action(false);
    let Some(Modal::RecordAction(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "DELETE".into();
    c.execute(
        "UPDATE actors SET role='manager' WHERE id=?1",
        [app.actor.id.to_string()],
    )
    .unwrap();
    app.submit_active_modal();
    assert!(!service.get(Kind::Storage, &r.id).unwrap().deleted);
}
