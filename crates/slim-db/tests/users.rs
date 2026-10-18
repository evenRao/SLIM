use slim_core::{
    model::actor::*,
    service::{ActorRepository, AuthService},
};
use slim_db::{
    connection::open_memory,
    migrate::run_migrations,
    repo::{actor::SqliteActorRepo, auth::SqliteAuthService},
};

fn database() -> rusqlite::Connection {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    c
}

#[test]
fn non_admins_cannot_edit_remove_or_reset_even_their_own_pin() {
    let c = database();
    let r = SqliteActorRepo::new(&c);
    for role in [Role::Viewer, Role::Operator, Role::Manager] {
        let a = r
            .create(&NewActor::new(role.to_string()).with_role(role))
            .unwrap();
        for change in [
            ActorUpdate {
                display_name: Some("Renamed".into()),
                ..Default::default()
            },
            ActorUpdate {
                active: Some(false),
                ..Default::default()
            },
            ActorUpdate {
                pin: Some("4321".into()),
                ..Default::default()
            },
        ] {
            assert!(r.update_as_admin(&a.id, &a.id, &change).is_err());
        }
        assert_eq!(
            r.get_by_id(&a.id).unwrap().unwrap().display_name,
            role.to_string()
        );
        assert!(
            r.create_as_admin(&a.id, &NewActor::new("Unauthorized"), Some("1234"))
                .is_err()
        );
    }
}

#[test]
fn rename_pin_reset_removal_and_restore_preserve_identity_and_revoke_sessions() {
    let c = database();
    let r = SqliteActorRepo::new(&c);
    let auth = SqliteAuthService::new(&c);
    let owner = r
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    let user = r
        .create_as_admin(&owner.id, &NewActor::new("Staff"), Some("old-test-pin"))
        .unwrap();
    let session = auth.login_with_pin("Staff", "old-test-pin").unwrap();
    let updated = r
        .update_as_admin(
            &owner.id,
            &user.id,
            &ActorUpdate {
                display_name: Some("New name".into()),
                pin: Some("new-test-pin".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.id, user.id);
    assert!(auth.validate_session(&session.token).is_err());
    assert!(auth.login_with_pin("New name", "old-test-pin").is_err());
    let session = auth.login_with_pin("New name", "new-test-pin").unwrap();
    r.update_as_admin(
        &owner.id,
        &user.id,
        &ActorUpdate {
            active: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(auth.validate_session(&session.token).is_err());
    assert!(auth.login_with_pin("New name", "new-test-pin").is_err());
    r.update_as_admin(
        &owner.id,
        &user.id,
        &ActorUpdate {
            active: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(auth.login_with_pin("New name", "new-test-pin").is_ok());
    let leaked: i64 = c
        .query_row(
            "SELECT count(*) FROM audit_events WHERE details LIKE '%test-pin%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
}

#[test]
fn last_active_admin_cannot_be_removed_or_demoted() {
    let c = database();
    let r = SqliteActorRepo::new(&c);
    let owner = r
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    for change in [
        ActorUpdate {
            active: Some(false),
            ..Default::default()
        },
        ActorUpdate {
            role: Some(Role::Manager),
            ..Default::default()
        },
    ] {
        assert!(
            r.update_as_admin(&owner.id, &owner.id, &change)
                .unwrap_err()
                .to_string()
                .contains("last active")
        );
    }
    let second = r
        .create_as_admin(
            &owner.id,
            &NewActor::new("Second").with_role(Role::Admin),
            Some("1234"),
        )
        .unwrap();
    r.update_as_admin(
        &owner.id,
        &owner.id,
        &ActorUpdate {
            active: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        r.update_as_admin(
            &owner.id,
            &second.id,
            &ActorUpdate {
                active: Some(false),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        r.update_as_admin(
            &second.id,
            &second.id,
            &ActorUpdate {
                role: Some(Role::Viewer),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn failed_audit_rolls_back_account_pin_and_session_changes() {
    let c = database();
    let r = SqliteActorRepo::new(&c);
    let auth = SqliteAuthService::new(&c);
    let owner = r
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    let user = r
        .create_as_admin(&owner.id, &NewActor::new("Staff"), Some("original-pin"))
        .unwrap();
    let session = auth.login_with_pin("Staff", "original-pin").unwrap();
    c.execute_batch("CREATE TRIGGER fail_user_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'test audit failure'); END;").unwrap();
    assert!(
        r.update_as_admin(
            &owner.id,
            &user.id,
            &ActorUpdate {
                display_name: Some("Changed".into()),
                active: Some(false),
                pin: Some("changed-pin".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(auth.validate_session(&session.token).is_ok());
    assert_eq!(
        r.get_by_id(&user.id).unwrap().unwrap().display_name,
        "Staff"
    );
    assert!(auth.login_with_pin("Staff", "original-pin").is_ok());
    assert!(
        r.create_as_admin(&owner.id, &NewActor::new("New"), Some("1234"))
            .is_err()
    );
    assert!(r.get_by_name("New").unwrap().is_none());
}

#[test]
fn invalid_compound_update_is_atomic_and_role_change_revokes_session() {
    let c = database();
    let r = SqliteActorRepo::new(&c);
    let auth = SqliteAuthService::new(&c);
    let owner = r
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    let user = r
        .create_as_admin(&owner.id, &NewActor::new("Staff"), Some("1234"))
        .unwrap();
    let session = auth.login_with_pin("Staff", "1234").unwrap();
    for change in [
        ActorUpdate {
            display_name: Some("Changed".into()),
            pin: Some("x".into()),
            ..Default::default()
        },
        ActorUpdate {
            display_name: Some(" ".into()),
            ..Default::default()
        },
        ActorUpdate {
            display_name: Some("Owner".into()),
            ..Default::default()
        },
    ] {
        assert!(r.update_as_admin(&owner.id, &user.id, &change).is_err());
        assert_eq!(
            r.get_by_id(&user.id).unwrap().unwrap().display_name,
            "Staff"
        );
        assert!(auth.validate_session(&session.token).is_ok());
    }
    r.update_as_admin(
        &owner.id,
        &user.id,
        &ActorUpdate {
            role: Some(Role::Manager),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(auth.validate_session(&session.token).is_err());
}
