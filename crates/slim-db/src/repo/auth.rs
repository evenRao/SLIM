//! Authentication service — SQLite implementation.

use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::*;
use slim_core::model::auth::*;
use slim_core::service::{ActorRepository, AuditRepository, AuthService};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::actor::{OptionalExt, SqliteActorRepo, format_time, parse_time};
use super::audit::SqliteAuditRepo;

/// Default station session duration: 12 hours.
const DEFAULT_SESSION_DURATION_HOURS: i64 = 12;

/// SQLite-backed authentication and session service.
pub struct SqliteAuthService<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteAuthService<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn create_session_for_actor(&self, actor: &Actor) -> CoreResult<Session> {
        let session_id = SessionId::new();
        let token = generate_session_token();
        let now = OffsetDateTime::now_utc();
        let expires_at = now + Duration::hours(DEFAULT_SESSION_DURATION_HOURS);

        let now_str = format_time(now);
        let expires_str = format_time(expires_at);

        self.conn
            .execute(
                "INSERT INTO sessions (id, actor_id, token, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    session_id.0.to_string(),
                    actor.id.0.to_string(),
                    token,
                    now_str,
                    expires_str,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(Session {
            id: session_id,
            actor_id: actor.id,
            token,
            created_at: now,
            expires_at,
        })
    }
}

impl AuthService for SqliteAuthService<'_> {
    fn login_with_pin(&self, name: &str, pin: &str) -> CoreResult<Session> {
        let actor_repo = SqliteActorRepo::new(self.conn);
        let actor = actor_repo
            .get_by_name(name)?
            .or(actor_repo.get_by_os_username(name)?)
            .ok_or_else(|| {
                CoreError::Unauthorized(format!("invalid credentials for '{}'", name))
            })?;

        if !actor.active {
            return Err(CoreError::Unauthorized(format!(
                "account for '{}' is inactive",
                name
            )));
        }

        let pin_hash = actor_repo.get_pin_hash(&actor.id)?.ok_or_else(|| {
            CoreError::Unauthorized(format!(
                "actor '{}' has no PIN configured; please set a PIN first",
                actor.display_name
            ))
        })?;

        if !verify_pin(pin, &pin_hash) {
            return Err(CoreError::Unauthorized(format!(
                "invalid credentials for '{}'",
                name
            )));
        }

        let session = self.create_session_for_actor(&actor)?;

        let audit_repo = SqliteAuditRepo::new(self.conn);
        let _ = audit_repo.record(
            &NewAuditEvent::new("auth.login.pin")
                .with_actor(actor.id)
                .with_details(format!("Session started for {}", actor.display_name)),
        );

        Ok(session)
    }

    fn login_with_badge(&self, badge_token: &str) -> CoreResult<Session> {
        let actor_repo = SqliteActorRepo::new(self.conn);
        let actor = actor_repo
            .get_by_badge_token(badge_token)?
            .ok_or_else(|| CoreError::Unauthorized("unrecognized or inactive badge".to_string()))?;

        if !actor.active {
            return Err(CoreError::Unauthorized(format!(
                "account for '{}' is inactive",
                actor.display_name
            )));
        }

        let session = self.create_session_for_actor(&actor)?;

        let audit_repo = SqliteAuditRepo::new(self.conn);
        let _ = audit_repo.record(
            &NewAuditEvent::new("auth.login.badge")
                .with_actor(actor.id)
                .with_details(format!("Badge login for {}", actor.display_name)),
        );

        Ok(session)
    }

    fn validate_session(&self, token: &str) -> CoreResult<AuthenticatedSession> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, actor_id, token, created_at, expires_at
                 FROM sessions WHERE token = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let session_res = stmt
            .query_row([token], |row| {
                let id_str: String = row.get(0)?;
                let actor_str: String = row.get(1)?;
                let token_val: String = row.get(2)?;
                let created_str: String = row.get(3)?;
                let expires_str: String = row.get(4)?;

                Ok(Session {
                    id: SessionId(Uuid::parse_str(&id_str).unwrap_or_default()),
                    actor_id: ActorId(Uuid::parse_str(&actor_str).unwrap_or_default()),
                    token: token_val,
                    created_at: parse_time(&created_str),
                    expires_at: parse_time(&expires_str),
                })
            })
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let session =
            session_res.ok_or_else(|| CoreError::Unauthorized("session not found".to_string()))?;

        if session.is_expired() {
            let _ = self.conn.execute(
                "DELETE FROM sessions WHERE token = ?1",
                rusqlite::params![token],
            );
            return Err(CoreError::Unauthorized("session has expired".to_string()));
        }

        let actor_repo = SqliteActorRepo::new(self.conn);
        let actor = actor_repo
            .get_by_id(&session.actor_id)?
            .ok_or_else(|| CoreError::Unauthorized("actor not found".to_string()))?;

        if !actor.active {
            return Err(CoreError::Unauthorized(format!(
                "account for '{}' is inactive",
                actor.display_name
            )));
        }

        Ok(AuthenticatedSession { session, actor })
    }

    fn terminate_session(&self, token: &str) -> CoreResult<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT actor_id FROM sessions WHERE token = ?1")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let actor_str: Option<String> = stmt
            .query_row([token], |row| row.get(0))
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        self.conn
            .execute(
                "DELETE FROM sessions WHERE token = ?1",
                rusqlite::params![token],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if let Some(s) = actor_str
            && let Ok(actor_id) = Uuid::parse_str(&s).map(ActorId)
        {
            let audit_repo = SqliteAuditRepo::new(self.conn);
            let _ = audit_repo.record(
                &NewAuditEvent::new("auth.logout")
                    .with_actor(actor_id)
                    .with_details("Session terminated"),
            );
        }

        Ok(())
    }

    fn set_pin(&self, actor_id: &ActorId, pin: &str) -> CoreResult<()> {
        let trimmed = pin.trim();
        if trimmed.len() < 4 {
            return Err(CoreError::InvalidInput(
                "PIN must be at least 4 characters".to_string(),
            ));
        }

        let hashed = hash_pin(trimmed);
        let actor_repo = SqliteActorRepo::new(self.conn);
        actor_repo.set_pin_hash(actor_id, &hashed)?;

        let audit_repo = SqliteAuditRepo::new(self.conn);
        let _ = audit_repo.record(
            &NewAuditEvent::new("actor.set_pin")
                .with_actor(*actor_id)
                .with_details("Updated PIN"),
        );

        Ok(())
    }

    fn set_badge_token(&self, actor_id: &ActorId, badge: Option<&str>) -> CoreResult<()> {
        let actor_repo = SqliteActorRepo::new(self.conn);
        actor_repo.set_badge_token(actor_id, badge)?;

        let audit_repo = SqliteAuditRepo::new(self.conn);
        let _ = audit_repo.record(
            &NewAuditEvent::new("actor.set_badge")
                .with_actor(*actor_id)
                .with_details(match badge {
                    Some(b) => format!("Assigned badge token {}", b),
                    None => "Removed badge token".to_string(),
                }),
        );

        Ok(())
    }

    fn set_role(&self, actor_id: &ActorId, role: Role) -> CoreResult<()> {
        let actor_repo = SqliteActorRepo::new(self.conn);
        actor_repo.set_role(actor_id, role)?;

        let audit_repo = SqliteAuditRepo::new(self.conn);
        let _ = audit_repo.record(
            &NewAuditEvent::new("actor.set_role")
                .with_actor(*actor_id)
                .with_details(format!("Role updated to {}", role)),
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;

    fn setup() -> Connection {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn login_with_pin_flow() {
        let conn = setup();
        let actor_repo = SqliteActorRepo::new(&conn);
        let auth_service = SqliteAuthService::new(&conn);

        let actor = actor_repo.create(&NewActor::new("Alice")).unwrap();

        // Login before PIN set should fail
        assert!(auth_service.login_with_pin("Alice", "1234").is_err());

        // Set PIN
        auth_service.set_pin(&actor.id, "1234").unwrap();

        // Login with wrong PIN should fail
        assert!(auth_service.login_with_pin("Alice", "9999").is_err());

        // Login with correct PIN succeeds
        let session = auth_service.login_with_pin("Alice", "1234").unwrap();
        assert_eq!(session.actor_id, actor.id);

        // Validate session
        let auth = auth_service.validate_session(&session.token).unwrap();
        assert_eq!(auth.actor.id, actor.id);
        assert_eq!(auth.actor.display_name, "Alice");

        // Terminate session
        auth_service.terminate_session(&session.token).unwrap();
        assert!(auth_service.validate_session(&session.token).is_err());
    }

    #[test]
    fn login_with_badge_flow() {
        let conn = setup();
        let actor_repo = SqliteActorRepo::new(&conn);
        let auth_service = SqliteAuthService::new(&conn);

        let actor = actor_repo.create(&NewActor::new("Bob")).unwrap();
        auth_service
            .set_badge_token(&actor.id, Some("BADGE-777"))
            .unwrap();

        let session = auth_service.login_with_badge("BADGE-777").unwrap();
        assert_eq!(session.actor_id, actor.id);

        let auth = auth_service.validate_session(&session.token).unwrap();
        assert_eq!(auth.actor.display_name, "Bob");
    }

    #[test]
    fn role_management_and_audit() {
        let conn = setup();
        let actor_repo = SqliteActorRepo::new(&conn);
        let auth_service = SqliteAuthService::new(&conn);
        let audit_repo = SqliteAuditRepo::new(&conn);

        let actor = actor_repo.create(&NewActor::new("Charlie")).unwrap();
        assert_eq!(actor.role, Role::Operator);

        auth_service.set_role(&actor.id, Role::Admin).unwrap();

        let updated = actor_repo.get_by_id(&actor.id).unwrap().unwrap();
        assert_eq!(updated.role, Role::Admin);

        let audits = audit_repo
            .list(&AuditFilter {
                actor_id: Some(actor.id),
                action: None,
                limit: None,
            })
            .unwrap();

        assert!(!audits.is_empty());
        assert_eq!(audits[0].action, "actor.set_role");
    }
}
