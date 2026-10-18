//! The explicit Admin purge is the only exception to ledger immutability.
use super::{actor::SqliteActorRepo, audit::SqliteAuditRepo, item::SqliteItemRepo};
use slim_core::{
    error::{CoreError, CoreResult},
    model::{
        actor::{ActorId, Role},
        auth::NewAuditEvent,
        item::ItemId,
    },
    service::{ActorRepository, AuditRepository, ItemRepository},
};

impl SqliteItemRepo<'_> {
    /// Erase a previously deleted, zero-stock item and its dependent history atomically.
    pub fn permanently_delete_as_admin(
        &self,
        id: &ItemId,
        actor: &ActorId,
        confirmation: &str,
    ) -> CoreResult<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(storage)?;
        if !SqliteActorRepo::new(self.conn)
            .get_by_id(actor)?
            .is_some_and(|a| a.active && a.role == Role::Admin)
        {
            return Err(CoreError::Unauthorized(
                "Only an active Admin can permanently delete inventory items".into(),
            ));
        }
        if confirmation.trim() != "DELETE PERMANENTLY" {
            return Err(CoreError::InvalidInput(
                "Type DELETE PERMANENTLY to erase the item and its history".into(),
            ));
        }
        let item = self.get_by_id(id)?.ok_or_else(|| CoreError::NotFound {
            entity: "item",
            id: id.to_string(),
        })?;
        if item.archived_at.is_none() {
            return Err(CoreError::InvalidInput(
                "Move this item to Deleted items before permanently deleting it".into(),
            ));
        }
        let key = id.to_string();
        let nonzero:bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM stock_balances WHERE item_id=?1 AND qty<>0) OR EXISTS(SELECT 1 FROM stock_movements WHERE item_id=?1 GROUP BY location_id HAVING sum(delta_qty)<>0)",
            [&key], |r|r.get(0)).map_err(storage)?;
        if nonzero {
            return Err(CoreError::InvalidInput(
                "Stock and ledger balances must both be zero before permanent deletion".into(),
            ));
        }
        // An imported multi-item operation must never lose another item's history.
        let shared:bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM stock_movements WHERE item_id<>?1 AND operation_id IN (SELECT operation_id FROM stock_movements WHERE item_id=?1))",[&key],|r|r.get(0)).map_err(storage)?;
        if shared {
            return Err(CoreError::InvalidInput("This item shares an imported operation with another item; permanent deletion is blocked".into()));
        }
        let operations = self
            .conn
            .prepare("SELECT DISTINCT operation_id FROM stock_movements WHERE item_id=?1")
            .map_err(storage)?
            .query_map([&key], |r| r.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        self.conn.execute("DELETE FROM observations WHERE processed_operation_id IN (SELECT operation_id FROM stock_movements WHERE item_id=?1)",[&key]).map_err(storage)?;
        self.conn.execute("DELETE FROM automation_runs WHERE outbox_id IN (SELECT id FROM event_outbox WHERE aggregate_id IN (SELECT operation_id FROM stock_movements WHERE item_id=?1) OR aggregate_id=?1 OR CASE WHEN json_valid(payload) THEN json_extract(payload,'$.item_id')=?1 ELSE 0 END) OR rule_id IN (SELECT id FROM automation_rules WHERE item_sku=?2)",rusqlite::params![key,item.sku]).map_err(storage)?;
        self.conn.execute("DELETE FROM event_outbox WHERE aggregate_id IN (SELECT operation_id FROM stock_movements WHERE item_id=?1) OR aggregate_id=?1 OR CASE WHEN json_valid(payload) THEN json_extract(payload,'$.item_id')=?1 ELSE 0 END",[&key]).map_err(storage)?;
        self.conn
            .execute(
                "DELETE FROM automation_rules WHERE item_sku=?1",
                [item.sku.as_deref()],
            )
            .map_err(storage)?;
        self.conn
            .execute(
                "DELETE FROM audit_events WHERE CASE WHEN json_valid(details) THEN json_extract(details,'$.item_id')=?1 OR json_extract(details,'$.result.id')=?1 ELSE 0 END OR (action IN ('item.deleted','item.restored') AND substr(details,-38)='(' || ?1 || ')')",
                [&key],
            )
            .map_err(storage)?;
        // SQLite schema changes are transactional. Other writers stay locked out,
        // and rollback restores this guard as well as all deleted records.
        let guard: String = self
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='movement_no_delete'",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        self.conn
            .execute_batch("DROP TRIGGER movement_no_delete;")
            .map_err(storage)?;
        self.conn
            .execute("DELETE FROM stock_movements WHERE item_id=?1", [&key])
            .map_err(storage)?;
        self.conn.execute_batch(&guard).map_err(storage)?;
        for operation in operations {
            self.conn
                .execute("DELETE FROM operations WHERE id=?1", [operation])
                .map_err(storage)?;
        }
        for table in ["stock_balances", "stock_thresholds", "identifiers"] {
            self.conn
                .execute(&format!("DELETE FROM {table} WHERE item_id=?1"), [&key])
                .map_err(storage)?;
        }
        self.conn
            .execute("DELETE FROM items WHERE id=?1", [&key])
            .map_err(storage)?;
        // Retain a minimal administrative record, not the erased catalog details/history.
        SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new("item.permanently_deleted")
                .with_actor(*actor)
                .with_details(key),
        )?;
        tx.commit().map_err(storage)?;
        Ok(())
    }
}
fn storage(error: rusqlite::Error) -> CoreError {
    CoreError::Storage(error.to_string())
}
