//! Audit repository — SQLite implementation.

use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::ActorId;
use slim_core::model::auth::*;
use slim_core::service::AuditRepository;
use time::OffsetDateTime;
use uuid::Uuid;

use super::actor::{format_time, parse_time};

/// SQLite-backed audit log repository.
pub struct SqliteAuditRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteAuditRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl AuditRepository for SqliteAuditRepo<'_> {
    fn record(&self, event: &NewAuditEvent) -> CoreResult<AuditEvent> {
        if event.action.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "audit action must not be empty".to_string(),
            ));
        }

        let id = AuditEventId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);
        let actor_id_str = event.actor_id.map(|a| a.0.to_string());

        self.conn
            .execute(
                "INSERT INTO audit_events (id, actor_id, action, details, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    id.0.to_string(),
                    actor_id_str,
                    event.action.trim(),
                    event.details.as_deref(),
                    now_str,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(AuditEvent {
            id,
            actor_id: event.actor_id,
            action: event.action.trim().to_string(),
            details: event.details.clone(),
            created_at: now,
        })
    }

    fn list(&self, filter: &AuditFilter) -> CoreResult<Vec<AuditEvent>> {
        let mut sql =
            "SELECT id, actor_id, action, details, created_at FROM audit_events WHERE 1=1"
                .to_string();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(actor_id) = filter.actor_id {
            sql.push_str(" AND actor_id = ?");
            params.push(Box::new(actor_id.0.to_string()));
        }

        if let Some(ref action) = filter.action {
            sql.push_str(" AND action LIKE ?");
            params.push(Box::new(format!("%{}%", action)));
        }

        sql.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filter.limit {
            sql.push_str(" LIMIT ?");
            params.push(Box::new(limit as i64));
        }

        let rusqlite_params: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite_params.as_slice(), row_to_audit_event)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }

        Ok(events)
    }
}

fn row_to_audit_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEvent> {
    let id_str: String = row.get(0)?;
    let actor_str: Option<String> = row.get(1)?;
    let action: String = row.get(2)?;
    let details: Option<String> = row.get(3)?;
    let created_str: String = row.get(4)?;

    let id = AuditEventId(Uuid::parse_str(&id_str).unwrap_or_default());
    let actor_id = actor_str.and_then(|s| Uuid::parse_str(&s).ok().map(ActorId));
    let created_at = parse_time(&created_str);

    Ok(AuditEvent {
        id,
        actor_id,
        action,
        details,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;
    use slim_core::service::ActorRepository;

    fn setup() -> Connection {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn record_and_list_audit_events() {
        let conn = setup();
        let actor_repo = crate::repo::actor::SqliteActorRepo::new(&conn);
        let repo = SqliteAuditRepo::new(&conn);

        let actor = actor_repo
            .create(&slim_core::model::actor::NewActor::new("Alice"))
            .unwrap();

        let event = repo
            .record(
                &NewAuditEvent::new("actor.created")
                    .with_actor(actor.id)
                    .with_details("Created actor Alice"),
            )
            .unwrap();

        assert_eq!(event.action, "actor.created");
        assert_eq!(event.actor_id, Some(actor.id));

        let list = repo
            .list(&AuditFilter {
                actor_id: Some(actor.id),
                action: None,
                limit: None,
            })
            .unwrap();

        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, event.id);
        assert_eq!(list[0].details.as_deref(), Some("Created actor Alice"));
    }

    #[test]
    fn filter_by_action_and_limit() {
        let conn = setup();
        let repo = SqliteAuditRepo::new(&conn);

        repo.record(&NewAuditEvent::new("user.login")).unwrap();
        repo.record(&NewAuditEvent::new("user.logout")).unwrap();
        repo.record(&NewAuditEvent::new("stock.adjust")).unwrap();

        let user_events = repo
            .list(&AuditFilter {
                actor_id: None,
                action: Some("user.".to_string()),
                limit: Some(10),
            })
            .unwrap();

        assert_eq!(user_events.len(), 2);

        let limited = repo
            .list(&AuditFilter {
                actor_id: None,
                action: None,
                limit: Some(1),
            })
            .unwrap();

        assert_eq!(limited.len(), 1);
    }
}
