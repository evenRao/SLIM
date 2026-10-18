use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use slim_core::{model::actor::*, service::ActorRepository};
use slim_db::repo::actor::SqliteActorRepo;
use slim_tui::{App, app::Modal, catalog::CatalogKind, event::handle_key_event};

fn database() -> rusqlite::Connection {
    let c = slim_db::connection::open_memory().unwrap();
    slim_db::migrate::run_migrations(&c).unwrap();
    c
}
fn fill(app: &mut App, values: &[&str]) {
    let Some(Modal::Catalog(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    for (field, value) in f.fields.iter_mut().zip(values) {
        field.value = (*value).into();
    }
}
fn setup(c: &rusqlite::Connection) -> App<'_> {
    let mut app = App::new_locked(c).unwrap();
    fill(&mut app, &["Owner", "1234", "1234"]);
    app.submit_active_modal();
    app
}
fn key(app: &mut App, c: char) {
    handle_key_event(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}
fn select(app: &mut App, id: ActorId) {
    let Some(Modal::People(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.selected = f.users.iter().position(|a| a.id == id).unwrap();
}

#[test]
fn admin_can_edit_reset_remove_and_reactivate_in_the_tui() {
    let c = database();
    let mut app = setup(&c);
    let r = SqliteActorRepo::new(&c);
    let user = r
        .create_as_admin(&app.actor.id, &NewActor::new("Staff"), Some("4321"))
        .unwrap();
    key(&mut app, 'U');
    select(&mut app, user.id);
    key(&mut app, 'e');
    fill(&mut app, &["New name", "manager"]);
    app.submit_active_modal();
    assert_eq!(
        r.get_by_id(&user.id).unwrap().unwrap().display_name,
        "New name"
    );
    select(&mut app, user.id);
    key(&mut app, 'p');
    fill(&mut app, &["5678", "5678"]);
    app.submit_active_modal();
    select(&mut app, user.id);
    key(&mut app, 'd');
    let Some(Modal::RemoveUser(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "REMOVE".into();
    app.submit_active_modal();
    assert!(!r.get_by_id(&user.id).unwrap().unwrap().active);
    select(&mut app, user.id);
    key(&mut app, 'r');
    assert!(r.get_by_id(&user.id).unwrap().unwrap().active);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| slim_tui::ui::render(&mut app, f))
        .unwrap();
}

#[test]
fn own_pin_reset_locks_terminal_and_last_admin_removal_is_blocked() {
    let c = database();
    let mut app = setup(&c);
    key(&mut app, 'U');
    key(&mut app, 'd');
    let Some(Modal::RemoveUser(f)) = app.active_modal.as_mut() else {
        panic!()
    };
    f.confirmation = "REMOVE".into();
    app.submit_active_modal();
    let Some(Modal::RemoveUser(f)) = app.active_modal.as_ref() else {
        panic!()
    };
    assert!(f.error.as_ref().unwrap().contains("last active"));
    app.open_catalog(CatalogKind::ResetPin(app.actor.id));
    fill(&mut app, &["5678", "5678"]);
    app.submit_active_modal();
    assert!(!app.authenticated);
    assert!(matches!(app.active_modal, Some(Modal::User(_))));
}

#[test]
fn remote_removal_and_pin_reset_lock_open_terminals_and_hide_removed_accounts() {
    let c = database();
    let owner = setup(&c);
    let r = SqliteActorRepo::new(&c);
    let user = r
        .create_as_admin(&owner.actor.id, &NewActor::new("Staff"), Some("4321"))
        .unwrap();
    let mut app = App::new(&c, user.clone()).unwrap();
    r.update_as_admin(
        &owner.actor.id,
        &user.id,
        &ActorUpdate {
            pin: Some("5678".into()),
            ..Default::default()
        },
    )
    .unwrap();
    key(&mut app, 'r');
    assert!(!app.authenticated);
    let mut app = App::new(&c, user.clone()).unwrap();
    r.update_as_admin(
        &owner.actor.id,
        &user.id,
        &ActorUpdate {
            active: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    app.revalidate_actor().unwrap();
    assert!(!app.authenticated);
    let Some(Modal::User(f)) = app.active_modal.as_ref() else {
        panic!()
    };
    assert!(!f.actors.iter().any(|a| a.id == user.id));
}

#[test]
fn nonadmin_and_revoked_admin_cannot_open_people_management() {
    let c = database();
    let owner = setup(&c);
    let r = SqliteActorRepo::new(&c);
    let user = r
        .create_as_admin(
            &owner.actor.id,
            &NewActor::new("Second").with_role(Role::Admin),
            Some("4321"),
        )
        .unwrap();
    let mut app = App::new(&c, user.clone()).unwrap();
    r.update_as_admin(
        &owner.actor.id,
        &user.id,
        &ActorUpdate {
            role: Some(Role::Manager),
            ..Default::default()
        },
    )
    .unwrap();
    key(&mut app, 'U');
    assert!(app.active_modal.is_none());
    assert_eq!(app.actor.role, Role::Manager);
    app.open_catalog(CatalogKind::ResetPin(user.id));
    assert!(app.active_modal.is_none());
}
