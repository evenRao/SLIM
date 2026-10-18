//! Actor repository — SQLite implementation.

use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::*;
use slim_core::service::ActorRepository;
use time::OffsetDateTime;

/// SQLite-backed actor repository.
pub struct SqliteActorRepo<'a> {
    pub(super) conn: &'a Connection,
}

impl<'a> SqliteActorRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Retrieve an actor by their scanned badge token (e.g., NFC or barcode).
    pub fn get_by_badge_token(&self, badge: &str) -> CoreResult<Option<Actor>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, display_name, os_username, role, active, created_at
                 FROM actors WHERE badge_token = ?1 AND active = 1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row([badge], row_to_actor)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    /// Get the hashed PIN for an actor.
    pub fn get_pin_hash(&self, actor_id: &ActorId) -> CoreResult<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pin_hash FROM actors WHERE id = ?1")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result: Option<Option<String>> = stmt
            .query_row([actor_id.0.to_string()], |row| row.get(0))
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result.flatten())
    }

    /// Set or update an actor's hashed PIN.
    pub fn set_pin_hash(&self, actor_id: &ActorId, pin_hash: &str) -> CoreResult<()> {
        let rows = self
            .conn
            .execute(
                "UPDATE actors SET pin_hash = ?1 WHERE id = ?2",
                rusqlite::params![pin_hash, actor_id.0.to_string()],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "actor",
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    /// Set, update, or clear an actor's badge token.
    pub fn set_badge_token(&self, actor_id: &ActorId, badge: Option<&str>) -> CoreResult<()> {
        let rows = self
            .conn
            .execute(
                "UPDATE actors SET badge_token = ?1 WHERE id = ?2",
                rusqlite::params![badge, actor_id.0.to_string()],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("UNIQUE constraint failed") =>
                {
                    CoreError::InvalidInput(format!(
                        "badge token '{}' is already assigned to another actor",
                        badge.unwrap_or_default()
                    ))
                }
                other => CoreError::Storage(other.to_string()),
            })?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "actor",
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }

    /// Set or update an actor's authorization role.
    pub fn set_role(&self, actor_id: &ActorId, role: Role) -> CoreResult<()> {
        let rows = self
            .conn
            .execute(
                "UPDATE actors SET role = ?1 WHERE id = ?2",
                rusqlite::params![role.to_string(), actor_id.0.to_string()],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "actor",
                id: actor_id.to_string(),
            });
        }
        Ok(())
    }
}

impl ActorRepository for SqliteActorRepo<'_> {
    fn create(&self, actor: &NewActor) -> CoreResult<Actor> {
        actor.validate()?;

        let id = ActorId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        self.conn
            .execute(
                "INSERT INTO actors (id, display_name, os_username, role, active, created_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                rusqlite::params![
                    id.0.to_string(),
                    actor.display_name.trim(),
                    actor.os_username.as_deref(),
                    actor.role.to_string(),
                    now_str,
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("UNIQUE constraint failed") =>
                {
                    CoreError::InvalidInput(format!(
                        "actor with name '{}' already exists",
                        actor.display_name
                    ))
                }
                other => CoreError::Storage(other.to_string()),
            })?;

        Ok(Actor {
            id,
            display_name: actor.display_name.trim().to_string(),
            os_username: actor.os_username.clone(),
            role: actor.role,
            active: true,
            created_at: now,
        })
    }

    fn get_by_id(&self, id: &ActorId) -> CoreResult<Option<Actor>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, display_name, os_username, role, active, created_at
                 FROM actors WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row([id.0.to_string()], row_to_actor)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn get_by_name(&self, name: &str) -> CoreResult<Option<Actor>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, display_name, os_username, role, active, created_at
                 FROM actors WHERE display_name = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row([name], row_to_actor)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn get_by_os_username(&self, username: &str) -> CoreResult<Option<Actor>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, display_name, os_username, role, active, created_at
                 FROM actors WHERE os_username = ?1 AND active = 1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row([username], row_to_actor)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn list(&self) -> CoreResult<Vec<Actor>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, display_name, os_username, role, active, created_at
                 FROM actors WHERE system_actor=0 ORDER BY display_name",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_actor)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut actors = Vec::new();
        for row in rows {
            actors.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(actors)
    }
}

fn row_to_actor(row: &rusqlite::Row<'_>) -> rusqlite::Result<Actor> {
    let id_str: String = row.get(0)?;
    let display_name: String = row.get(1)?;
    let os_username: Option<String> = row.get(2)?;
    let role_str: String = row.get(3)?;
    let active: bool = row.get(4)?;
    let created_str: String = row.get(5)?;

    let id = ActorId(uuid::Uuid::parse_str(&id_str).unwrap_or_default());
    let role: Role = role_str.parse().unwrap_or(Role::Operator);
    let created_at = parse_time(&created_str);

    Ok(Actor {
        id,
        display_name,
        os_username,
        role,
        active,
        created_at,
    })
}

/// Format an OffsetDateTime as RFC 3339 string.
pub(crate) fn format_time(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| t.to_string())
}

/// Parse an RFC 3339 string into an OffsetDateTime.
pub(crate) fn parse_time(s: &str) -> OffsetDateTime {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Extension trait for optional query results.
pub(crate) trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
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
    fn create_and_get_actor() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        let actor = repo
            .create(&NewActor::new("Alice").with_os_username("alice"))
            .unwrap();

        assert_eq!(actor.display_name, "Alice");
        assert_eq!(actor.role, Role::Operator);
        assert!(actor.active);

        let found = repo.get_by_id(&actor.id).unwrap().unwrap();
        assert_eq!(found.display_name, "Alice");
        assert_eq!(found.role, Role::Operator);
        assert_eq!(found.os_username, Some("alice".to_string()));
    }

    #[test]
    fn get_by_name() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        repo.create(&NewActor::new("Bob")).unwrap();

        let found = repo.get_by_name("Bob").unwrap();
        assert!(found.is_some());

        let missing = repo.get_by_name("Nobody").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn get_by_os_username() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        repo.create(&NewActor::new("Charlie").with_os_username("charlie_os"))
            .unwrap();

        let found = repo.get_by_os_username("charlie_os").unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn duplicate_name_rejected() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        repo.create(&NewActor::new("Duplicate")).unwrap();

        let result = repo.create(&NewActor::new("Duplicate"));
        assert!(result.is_err());
    }

    #[test]
    fn list_actors() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        repo.create(&NewActor::new("Zara")).unwrap();
        repo.create(&NewActor::new("Adam")).unwrap();

        let list = repo.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].display_name, "Adam"); // sorted
        assert_eq!(list[1].display_name, "Zara");
    }

    #[test]
    fn role_update_and_persistence() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        let actor = repo
            .create(&NewActor::new("Dana").with_role(Role::Viewer))
            .unwrap();
        assert_eq!(actor.role, Role::Viewer);

        repo.set_role(&actor.id, Role::Manager).unwrap();
        let updated = repo.get_by_id(&actor.id).unwrap().unwrap();
        assert_eq!(updated.role, Role::Manager);
    }

    #[test]
    fn pin_and_badge_management() {
        let conn = setup();
        let repo = SqliteActorRepo::new(&conn);

        let actor = repo.create(&NewActor::new("Evan")).unwrap();
        assert!(repo.get_pin_hash(&actor.id).unwrap().is_none());

        repo.set_pin_hash(&actor.id, "salt$hash123").unwrap();
        assert_eq!(
            repo.get_pin_hash(&actor.id).unwrap(),
            Some("salt$hash123".to_string())
        );

        repo.set_badge_token(&actor.id, Some("NFC-TAG-8888"))
            .unwrap();
        let found_by_badge = repo.get_by_badge_token("NFC-TAG-8888").unwrap().unwrap();
        assert_eq!(found_by_badge.id, actor.id);

        // Clearing badge
        repo.set_badge_token(&actor.id, None).unwrap();
        assert!(repo.get_by_badge_token("NFC-TAG-8888").unwrap().is_none());
    }
}
