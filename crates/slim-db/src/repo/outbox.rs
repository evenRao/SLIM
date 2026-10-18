//! SQLite implementation of OutboxRepository.
//!
//! Manages committed domain events in the `event_outbox` table.

use rusqlite::{Connection, params};
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::automation::{OutboxEvent, OutboxEventId};
use slim_core::service::OutboxRepository;
use time::OffsetDateTime;
use uuid::Uuid;

use super::actor::{OptionalExt, format_time, parse_time};

/// SQLite-backed outbox repository.
pub struct SqliteOutboxRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteOutboxRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

fn row_to_outbox_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxEvent> {
    let id_str: String = row.get(0)?;
    let event_type: String = row.get(1)?;
    let aggregate_id: String = row.get(2)?;
    let payload: String = row.get(3)?;
    let created_str: String = row.get(4)?;
    let delivered_str: Option<String> = row.get(5)?;
    let attempt_count: u32 = row.get(6)?;
    let last_error: Option<String> = row.get(7)?;

    let id = OutboxEventId(Uuid::parse_str(&id_str).unwrap_or_default());
    let created_at = parse_time(&created_str);
    let delivered_at = delivered_str.as_deref().map(parse_time);

    Ok(OutboxEvent {
        id,
        event_type,
        aggregate_id,
        payload,
        created_at,
        delivered_at,
        attempt_count,
        last_error,
    })
}

impl OutboxRepository for SqliteOutboxRepo<'_> {
    fn fetch_pending(&self, limit: usize) -> CoreResult<Vec<OutboxEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, event_type, aggregate_id, payload, created_at, delivered_at, attempt_count, last_error
                 FROM event_outbox
                 WHERE delivered_at IS NULL
                 AND (claimed_until IS NULL OR claimed_until <= strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                 ORDER BY created_at ASC
                 LIMIT ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], row_to_outbox_event)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(events)
    }

    fn mark_delivered(&self, id: &OutboxEventId) -> CoreResult<()> {
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        let rows = self
            .conn
            .execute(
                "UPDATE event_outbox SET delivered_at = ?1 WHERE id = ?2",
                params![now_str, id.0.to_string()],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "outbox event",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    fn record_attempt_error(&self, id: &OutboxEventId, error_msg: &str) -> CoreResult<()> {
        let rows = self
            .conn
            .execute(
                "UPDATE event_outbox
                 SET attempt_count = attempt_count + 1, last_error = ?1
                 WHERE id = ?2",
                params![error_msg, id.0.to_string()],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "outbox event",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    fn pending_count(&self) -> CoreResult<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM event_outbox WHERE delivered_at IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(count as usize)
    }

    fn get_by_id(&self, id: &OutboxEventId) -> CoreResult<Option<OutboxEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, event_type, aggregate_id, payload, created_at, delivered_at, attempt_count, last_error
                 FROM event_outbox
                 WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![id.0.to_string()], row_to_outbox_event)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;

    #[test]
    fn test_fetch_pending_and_delivery_lifecycle() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        let repo = SqliteOutboxRepo::new(&conn);

        assert_eq!(repo.pending_count().unwrap(), 0);

        // Insert two test outbox events directly
        let id1 = OutboxEventId::new();
        let id2 = OutboxEventId::new();
        let now_str = format_time(OffsetDateTime::now_utc());

        conn.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockReceived', 'item-1', '{\"qty\": 10}', ?2)",
            params![id1.0.to_string(), now_str],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockIssued', 'item-1', '{\"qty\": 5}', ?2)",
            params![id2.0.to_string(), now_str],
        )
        .unwrap();

        assert_eq!(repo.pending_count().unwrap(), 2);

        let pending = repo.fetch_pending(10).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].id, id1);
        assert_eq!(pending[0].attempt_count, 0);

        // Record error on first event
        repo.record_attempt_error(&id1, "Webhook connection refused")
            .unwrap();
        let updated1 = repo.get_by_id(&id1).unwrap().unwrap();
        assert_eq!(updated1.attempt_count, 1);
        assert_eq!(
            updated1.last_error.as_deref(),
            Some("Webhook connection refused")
        );

        // Mark first event as delivered
        repo.mark_delivered(&id1).unwrap();
        assert_eq!(repo.pending_count().unwrap(), 1);

        let delivered1 = repo.get_by_id(&id1).unwrap().unwrap();
        assert!(delivered1.delivered_at.is_some());

        let remaining = repo.fetch_pending(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, id2);
    }
}
