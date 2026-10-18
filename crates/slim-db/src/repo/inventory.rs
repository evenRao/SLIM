//! Inventory service, history queries, reconciliation, and threshold repositories.

use rusqlite::{Connection, params};
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::{ActorId, Role};
use slim_core::model::item::ItemId;
use slim_core::model::location::LocationId;
use slim_core::model::operation::*;
use slim_core::model::stock::*;
use slim_core::service::{
    HistoryQuery, InventoryService, ReconciliationService, ThresholdRepository,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::actor::{format_time, parse_time};

/// SQLite implementation of the inventory service.
pub struct SqliteInventoryService<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteInventoryService<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn check_idempotency(
        &self,
        key: &str,
        fingerprint: &str,
    ) -> CoreResult<Option<OperationReceipt>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, request_fingerprint, receipt_json FROM operations WHERE idempotency_key = ?1")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row([key], |row| {
            let id_str: String = row.get(0)?;
            let kind_str: String = row.get(1)?;
            Ok((
                id_str,
                kind_str,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        });

        match res {
            Ok((id_str, kind_str, saved, receipt)) => {
                if saved.as_deref() != Some(fingerprint) {
                    return Err(CoreError::IdempotencyConflict(key.to_string()));
                }
                if let Some(receipt) = receipt {
                    return serde_json::from_str(&receipt)
                        .map(Some)
                        .map_err(|e| CoreError::Storage(e.to_string()));
                }
                let op_id = OperationId(Uuid::parse_str(&id_str).unwrap_or_default());
                let kind: OperationKind = kind_str.parse().unwrap_or(OperationKind::Receive);

                // Load movements for this operation
                let mut mv_stmt = self
                    .conn
                    .prepare(
                        "SELECT id, operation_id, item_id, location_id, delta_qty, created_at
                         FROM stock_movements WHERE operation_id = ?1 ORDER BY created_at ASC",
                    )
                    .map_err(|e| CoreError::Storage(e.to_string()))?;

                let movements = mv_stmt
                    .query_map([&id_str], row_to_movement)
                    .map_err(|e| CoreError::Storage(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| CoreError::Storage(e.to_string()))?;

                // Load current balances for affected items and locations
                let mut balances = Vec::new();
                for mv in &movements {
                    if let Some(bal) = self.get_balance(&mv.item_id, &mv.location_id)?
                        && !balances.iter().any(|b: &StockBalance| {
                            b.item_id == bal.item_id && b.location_id == bal.location_id
                        })
                    {
                        balances.push(bal);
                    }
                }

                Ok(Some(OperationReceipt {
                    operation_id: op_id,
                    kind,
                    movements,
                    resulting_balances: balances,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn validate_actor(&self, actor_id: &ActorId) -> CoreResult<Role> {
        let mut stmt = self
            .conn
            .prepare("SELECT active, role FROM actors WHERE id = ?1")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let (active, role_str): (bool, String) = stmt
            .query_row([actor_id.0.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound {
                    entity: "actor",
                    id: actor_id.to_string(),
                },
                other => CoreError::Storage(other.to_string()),
            })?;

        if !active {
            return Err(CoreError::InvalidInput("actor is inactive".to_string()));
        }

        let role: Role = role_str.parse().unwrap_or(Role::Operator);
        Ok(role)
    }

    fn validate_item(&self, item_id: &ItemId) -> CoreResult<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT archived_at FROM items WHERE id = ?1")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let archived: Option<String> = stmt
            .query_row([item_id.0.to_string()], |row| row.get(0))
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound {
                    entity: "item",
                    id: item_id.to_string(),
                },
                other => CoreError::Storage(other.to_string()),
            })?;

        if archived.is_some() {
            return Err(CoreError::InvalidInput("item is archived".to_string()));
        }
        Ok(())
    }

    fn validate_location(&self, location_id: &LocationId) -> CoreResult<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT can_hold_stock FROM locations WHERE id = ?1 AND archived_at IS NULL")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let can_hold: bool = stmt
            .query_row([location_id.0.to_string()], |row| row.get(0))
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound {
                    entity: "location",
                    id: location_id.to_string(),
                },
                other => CoreError::Storage(other.to_string()),
            })?;

        if !can_hold {
            return Err(CoreError::InvalidInput(format!(
                "location '{}' is not configured to hold stock",
                location_id
            )));
        }
        Ok(())
    }

    fn read_balance(&self, item_id: &ItemId, location_id: &LocationId) -> CoreResult<u32> {
        let mut stmt = self
            .conn
            .prepare("SELECT qty FROM stock_balances WHERE item_id = ?1 AND location_id = ?2")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row([item_id.0.to_string(), location_id.0.to_string()], |row| {
            row.get::<_, i64>(0)
        });

        match res {
            Ok(qty) => u32::try_from(qty)
                .map_err(|_| CoreError::Storage("quantity outside supported range".into())),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn check_threshold_and_enqueue_outbox(
        &self,
        item_id: &ItemId,
        location_id: &LocationId,
        previous: Option<u32>,
        new_balance: u32,
        now_str: &str,
    ) -> CoreResult<()> {
        use rusqlite::OptionalExtension;
        let min: Option<u32> = self
            .conn
            .query_row(
                "SELECT min_qty FROM stock_thresholds WHERE item_id=?1
             AND (stock_location_id=?2 OR stock_location_id IS NULL)
             ORDER BY stock_location_id IS NULL LIMIT 1",
                params![item_id.to_string(), location_id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        if let Some(min) = min {
            let low = new_balance <= min;
            if previous.map(|q| q <= min) != Some(low) {
                let event = if low {
                    "StockBelowThreshold"
                } else {
                    "StockRecovered"
                };
                let payload = serde_json::json!({"schema_version":1,"item_id":item_id.to_string(),
                    "location_id":location_id.to_string(),"on_hand":new_balance,"min_qty":min});
                self.conn
                    .execute(
                        "INSERT INTO event_outbox (id,event_type,aggregate_id,payload,created_at)
                    VALUES (?1,?2,?3,?4,?5)",
                        params![
                            Uuid::new_v4().to_string(),
                            event,
                            item_id.to_string(),
                            payload.to_string(),
                            now_str
                        ],
                    )
                    .map_err(|e| CoreError::Storage(e.to_string()))?;
            }
        }
        Ok(())
    }
}

impl InventoryService for SqliteInventoryService<'_> {
    fn receive(&self, cmd: ReceiveStock) -> CoreResult<OperationReceipt> {
        cmd.validate()?;
        let fingerprint = format!("{cmd:?}");
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        if let Some(ref key) = cmd.idempotency_key
            && let Some(receipt) = self.check_idempotency(key, &fingerprint)?
        {
            return Ok(receipt);
        }

        let role = self.validate_actor(&cmd.actor_id)?;
        if !role.can_transact() {
            return Err(CoreError::Unauthorized(format!(
                "actor role '{}' does not have permission to receive stock",
                role
            )));
        }
        self.validate_item(&cmd.item_id)?;
        self.validate_location(&cmd.destination_id)?;

        let previous_balances = self
            .conn
            .prepare(
                "SELECT item_id,location_id,qty,updated_at FROM stock_balances WHERE item_id=?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map([cmd.item_id.to_string()], row_to_balance)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let op_id = OperationId::new();
        let mv_id = MovementId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        // 1. Insert operation
        tx.execute(
            "INSERT INTO operations (id, kind, actor_id, source, reason, idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op_id.0.to_string(),
                OperationKind::Receive.to_string(),
                cmd.actor_id.0.to_string(),
                cmd.source.to_string(),
                cmd.reason.as_deref(),
                cmd.idempotency_key.as_deref(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. Insert movement
        tx.execute(
            "INSERT INTO stock_movements (id, operation_id, item_id, location_id, delta_qty, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mv_id.0.to_string(),
                op_id.0.to_string(),
                cmd.item_id.0.to_string(),
                cmd.destination_id.0.to_string(),
                cmd.quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 3. Upsert balance
        tx.execute(
            "INSERT INTO stock_balances (item_id, location_id, qty, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(item_id, location_id) DO UPDATE SET
                 qty = qty + ?3,
                 updated_at = ?4",
            params![
                cmd.item_id.0.to_string(),
                cmd.destination_id.0.to_string(),
                cmd.quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // Query new balance
        let new_qty: i64 = tx
            .query_row(
                "SELECT qty FROM stock_balances WHERE item_id = ?1 AND location_id = ?2",
                params![cmd.item_id.0.to_string(), cmd.destination_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 4. Outbox event
        let event_payload = serde_json::json!({
            "schema_version": 1,
            "operation_id": op_id.to_string(),
            "item_id": cmd.item_id.to_string(),
            "location_id": cmd.destination_id.to_string(),
            "quantity": cmd.quantity,
            "resulting_qty": new_qty,
        });

        tx.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockReceived', ?2, ?3, ?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.item_id.0.to_string(),
                event_payload.to_string(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        self.check_threshold_and_enqueue_outbox(
            &cmd.item_id,
            &cmd.destination_id,
            previous_balances
                .iter()
                .find(|b| b.location_id == cmd.destination_id)
                .map(|b| b.qty),
            new_qty as u32,
            &now_str,
        )?;

        let movement = StockMovement {
            id: mv_id,
            operation_id: op_id,
            item_id: cmd.item_id,
            location_id: cmd.destination_id,
            delta_qty: cmd.quantity as i64,
            created_at: now,
        };

        let balance = StockBalance {
            item_id: cmd.item_id,
            location_id: cmd.destination_id,
            qty: new_qty as u32,
            updated_at: now,
        };

        let saved_receipt = OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Receive,
            movements: vec![movement.clone()],
            resulting_balances: vec![balance.clone()],
        };
        let receipt_json =
            serde_json::to_string(&saved_receipt).map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "UPDATE operations SET request_fingerprint=?1,receipt_json=?3 WHERE id=?2",
            params![fingerprint, op_id.to_string(), receipt_json],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "INSERT INTO audit_events (id,actor_id,action,details,created_at)
            VALUES (?1,?2,'stock.changed',?3,?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.actor_id.to_string(),
                event_payload.to_string(),
                now_str
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Receive,
            movements: vec![movement],
            resulting_balances: vec![balance],
        })
    }

    fn issue(&self, cmd: IssueStock) -> CoreResult<OperationReceipt> {
        cmd.validate()?;
        let fingerprint = format!("{cmd:?}");
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        if let Some(ref key) = cmd.idempotency_key
            && let Some(receipt) = self.check_idempotency(key, &fingerprint)?
        {
            return Ok(receipt);
        }

        let role = self.validate_actor(&cmd.actor_id)?;
        if !role.can_transact() {
            return Err(CoreError::Unauthorized(format!(
                "actor role '{}' does not have permission to issue stock",
                role
            )));
        }
        self.validate_item(&cmd.item_id)?;
        self.validate_location(&cmd.source_id)?;

        let current_qty = self.read_balance(&cmd.item_id, &cmd.source_id)?;
        if current_qty < cmd.quantity {
            return Err(CoreError::InsufficientStock {
                requested: cmd.quantity,
                available: current_qty,
            });
        }

        let previous_balances = self
            .conn
            .prepare(
                "SELECT item_id,location_id,qty,updated_at FROM stock_balances WHERE item_id=?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map([cmd.item_id.to_string()], row_to_balance)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let op_id = OperationId::new();
        let mv_id = MovementId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        // 1. Insert operation
        tx.execute(
            "INSERT INTO operations (id, kind, actor_id, source, reason, idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op_id.0.to_string(),
                OperationKind::Issue.to_string(),
                cmd.actor_id.0.to_string(),
                cmd.source.to_string(),
                cmd.reason.as_deref(),
                cmd.idempotency_key.as_deref(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. Insert movement (-quantity)
        tx.execute(
            "INSERT INTO stock_movements (id, operation_id, item_id, location_id, delta_qty, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mv_id.0.to_string(),
                op_id.0.to_string(),
                cmd.item_id.0.to_string(),
                cmd.source_id.0.to_string(),
                -(cmd.quantity as i64),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 3. Decrement balance
        tx.execute(
            "UPDATE stock_balances
             SET qty = qty - ?3, updated_at = ?4
             WHERE item_id = ?1 AND location_id = ?2",
            params![
                cmd.item_id.0.to_string(),
                cmd.source_id.0.to_string(),
                cmd.quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        let new_qty: i64 = tx
            .query_row(
                "SELECT qty FROM stock_balances WHERE item_id = ?1 AND location_id = ?2",
                params![cmd.item_id.0.to_string(), cmd.source_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 4. Outbox event
        let event_payload = serde_json::json!({
            "schema_version": 1,
            "operation_id": op_id.to_string(),
            "item_id": cmd.item_id.to_string(),
            "location_id": cmd.source_id.to_string(),
            "quantity": cmd.quantity,
            "resulting_qty": new_qty,
        });

        tx.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockIssued', ?2, ?3, ?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.item_id.0.to_string(),
                event_payload.to_string(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        self.check_threshold_and_enqueue_outbox(
            &cmd.item_id,
            &cmd.source_id,
            previous_balances
                .iter()
                .find(|b| b.location_id == cmd.source_id)
                .map(|b| b.qty),
            new_qty as u32,
            &now_str,
        )?;

        let movement = StockMovement {
            id: mv_id,
            operation_id: op_id,
            item_id: cmd.item_id,
            location_id: cmd.source_id,
            delta_qty: -(cmd.quantity as i64),
            created_at: now,
        };

        let balance = StockBalance {
            item_id: cmd.item_id,
            location_id: cmd.source_id,
            qty: new_qty as u32,
            updated_at: now,
        };

        let saved_receipt = OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Issue,
            movements: vec![movement.clone()],
            resulting_balances: vec![balance.clone()],
        };
        let receipt_json =
            serde_json::to_string(&saved_receipt).map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "UPDATE operations SET request_fingerprint=?1,receipt_json=?3 WHERE id=?2",
            params![fingerprint, op_id.to_string(), receipt_json],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "INSERT INTO audit_events (id,actor_id,action,details,created_at)
            VALUES (?1,?2,'stock.changed',?3,?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.actor_id.to_string(),
                event_payload.to_string(),
                now_str
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Issue,
            movements: vec![movement],
            resulting_balances: vec![balance],
        })
    }

    fn transfer(&self, cmd: TransferStock) -> CoreResult<OperationReceipt> {
        cmd.validate()?;
        let fingerprint = format!("{cmd:?}");
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        if let Some(ref key) = cmd.idempotency_key
            && let Some(receipt) = self.check_idempotency(key, &fingerprint)?
        {
            return Ok(receipt);
        }

        let role = self.validate_actor(&cmd.actor_id)?;
        if !role.can_transact() {
            return Err(CoreError::Unauthorized(format!(
                "actor role '{}' does not have permission to transfer stock",
                role
            )));
        }
        self.validate_item(&cmd.item_id)?;
        self.validate_location(&cmd.from_id)?;
        self.validate_location(&cmd.to_id)?;

        let source_qty = self.read_balance(&cmd.item_id, &cmd.from_id)?;
        if source_qty < cmd.quantity {
            return Err(CoreError::InsufficientStock {
                requested: cmd.quantity,
                available: source_qty,
            });
        }

        let previous_balances = self
            .conn
            .prepare(
                "SELECT item_id,location_id,qty,updated_at FROM stock_balances WHERE item_id=?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map([cmd.item_id.to_string()], row_to_balance)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let op_id = OperationId::new();
        let mv_src_id = MovementId::new();
        let mv_dst_id = MovementId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        // 1. Insert operation
        tx.execute(
            "INSERT INTO operations (id, kind, actor_id, source, reason, idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op_id.0.to_string(),
                OperationKind::Transfer.to_string(),
                cmd.actor_id.0.to_string(),
                cmd.source.to_string(),
                cmd.reason.as_deref(),
                cmd.idempotency_key.as_deref(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. Insert two movements: source (-qty), destination (+qty)
        tx.execute(
            "INSERT INTO stock_movements (id, operation_id, item_id, location_id, delta_qty, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mv_src_id.0.to_string(),
                op_id.0.to_string(),
                cmd.item_id.0.to_string(),
                cmd.from_id.0.to_string(),
                -(cmd.quantity as i64),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        tx.execute(
            "INSERT INTO stock_movements (id, operation_id, item_id, location_id, delta_qty, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mv_dst_id.0.to_string(),
                op_id.0.to_string(),
                cmd.item_id.0.to_string(),
                cmd.to_id.0.to_string(),
                cmd.quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 3. Update source balance
        tx.execute(
            "UPDATE stock_balances
             SET qty = qty - ?3, updated_at = ?4
             WHERE item_id = ?1 AND location_id = ?2",
            params![
                cmd.item_id.0.to_string(),
                cmd.from_id.0.to_string(),
                cmd.quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 4. Upsert destination balance
        tx.execute(
            "INSERT INTO stock_balances (item_id, location_id, qty, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(item_id, location_id) DO UPDATE SET
                 qty = qty + ?3,
                 updated_at = ?4",
            params![
                cmd.item_id.0.to_string(),
                cmd.to_id.0.to_string(),
                cmd.quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        let new_src_qty: i64 = tx
            .query_row(
                "SELECT qty FROM stock_balances WHERE item_id = ?1 AND location_id = ?2",
                params![cmd.item_id.0.to_string(), cmd.from_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let new_dst_qty: i64 = tx
            .query_row(
                "SELECT qty FROM stock_balances WHERE item_id = ?1 AND location_id = ?2",
                params![cmd.item_id.0.to_string(), cmd.to_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 5. Outbox event
        let event_payload = serde_json::json!({
            "schema_version": 1,
            "operation_id": op_id.to_string(),
            "item_id": cmd.item_id.to_string(),
            "from_id": cmd.from_id.to_string(),
            "to_id": cmd.to_id.to_string(),
            "quantity": cmd.quantity,
            "resulting_from_qty": new_src_qty,
            "resulting_to_qty": new_dst_qty,
        });

        tx.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockTransferred', ?2, ?3, ?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.item_id.0.to_string(),
                event_payload.to_string(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        self.check_threshold_and_enqueue_outbox(
            &cmd.item_id,
            &cmd.from_id,
            previous_balances
                .iter()
                .find(|b| b.location_id == cmd.from_id)
                .map(|b| b.qty),
            new_src_qty as u32,
            &now_str,
        )?;

        self.check_threshold_and_enqueue_outbox(
            &cmd.item_id,
            &cmd.to_id,
            previous_balances
                .iter()
                .find(|b| b.location_id == cmd.to_id)
                .map(|b| b.qty),
            new_dst_qty as u32,
            &now_str,
        )?;
        let movements = vec![
            StockMovement {
                id: mv_src_id,
                operation_id: op_id,
                item_id: cmd.item_id,
                location_id: cmd.from_id,
                delta_qty: -(cmd.quantity as i64),
                created_at: now,
            },
            StockMovement {
                id: mv_dst_id,
                operation_id: op_id,
                item_id: cmd.item_id,
                location_id: cmd.to_id,
                delta_qty: cmd.quantity as i64,
                created_at: now,
            },
        ];

        let resulting_balances = vec![
            StockBalance {
                item_id: cmd.item_id,
                location_id: cmd.from_id,
                qty: new_src_qty as u32,
                updated_at: now,
            },
            StockBalance {
                item_id: cmd.item_id,
                location_id: cmd.to_id,
                qty: new_dst_qty as u32,
                updated_at: now,
            },
        ];

        let saved_receipt = OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Transfer,
            movements: movements.clone(),
            resulting_balances: resulting_balances.clone(),
        };
        let receipt_json =
            serde_json::to_string(&saved_receipt).map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "UPDATE operations SET request_fingerprint=?1,receipt_json=?3 WHERE id=?2",
            params![fingerprint, op_id.to_string(), receipt_json],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "INSERT INTO audit_events (id,actor_id,action,details,created_at)
            VALUES (?1,?2,'stock.changed',?3,?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.actor_id.to_string(),
                event_payload.to_string(),
                now_str
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Transfer,
            movements,
            resulting_balances,
        })
    }

    fn adjust(&self, cmd: AdjustStock) -> CoreResult<OperationReceipt> {
        cmd.validate()?;
        let fingerprint = format!("{cmd:?}");
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        if let Some(ref key) = cmd.idempotency_key
            && let Some(receipt) = self.check_idempotency(key, &fingerprint)?
        {
            return Ok(receipt);
        }

        let role = self.validate_actor(&cmd.actor_id)?;
        if !role.can_adjust() {
            return Err(CoreError::Unauthorized(format!(
                "actor role '{}' does not have permission to adjust inventory (Manager or Admin required)",
                role
            )));
        }
        self.validate_item(&cmd.item_id)?;
        self.validate_location(&cmd.location_id)?;

        let current_qty = self.read_balance(&cmd.item_id, &cmd.location_id)?;
        let delta = (cmd.new_quantity as i64) - (current_qty as i64);

        let previous_balances = self
            .conn
            .prepare(
                "SELECT item_id,location_id,qty,updated_at FROM stock_balances WHERE item_id=?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map([cmd.item_id.to_string()], row_to_balance)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let op_id = OperationId::new();
        let mv_id = MovementId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        // 1. Insert operation
        tx.execute(
            "INSERT INTO operations (id, kind, actor_id, source, reason, idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op_id.0.to_string(),
                OperationKind::Adjust.to_string(),
                cmd.actor_id.0.to_string(),
                cmd.source.to_string(),
                Some(cmd.reason.as_str()),
                cmd.idempotency_key.as_deref(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. Insert movement with delta
        tx.execute(
            "INSERT INTO stock_movements (id, operation_id, item_id, location_id, delta_qty, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mv_id.0.to_string(),
                op_id.0.to_string(),
                cmd.item_id.0.to_string(),
                cmd.location_id.0.to_string(),
                delta,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 3. Upsert balance to absolute new_quantity
        tx.execute(
            "INSERT INTO stock_balances (item_id, location_id, qty, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(item_id, location_id) DO UPDATE SET
                 qty = ?3,
                 updated_at = ?4",
            params![
                cmd.item_id.0.to_string(),
                cmd.location_id.0.to_string(),
                cmd.new_quantity as i64,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 4. Outbox event
        let event_payload = serde_json::json!({
            "schema_version": 1,
            "operation_id": op_id.to_string(),
            "item_id": cmd.item_id.to_string(),
            "location_id": cmd.location_id.to_string(),
            "previous_qty": current_qty,
            "new_qty": cmd.new_quantity,
            "delta": delta,
            "reason": cmd.reason,
        });

        tx.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockAdjusted', ?2, ?3, ?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.item_id.0.to_string(),
                event_payload.to_string(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        self.check_threshold_and_enqueue_outbox(
            &cmd.item_id,
            &cmd.location_id,
            previous_balances
                .iter()
                .find(|b| b.location_id == cmd.location_id)
                .map(|b| b.qty),
            cmd.new_quantity,
            &now_str,
        )?;

        let movement = StockMovement {
            id: mv_id,
            operation_id: op_id,
            item_id: cmd.item_id,
            location_id: cmd.location_id,
            delta_qty: delta,
            created_at: now,
        };

        let balance = StockBalance {
            item_id: cmd.item_id,
            location_id: cmd.location_id,
            qty: cmd.new_quantity,
            updated_at: now,
        };

        let saved_receipt = OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Adjust,
            movements: vec![movement.clone()],
            resulting_balances: vec![balance.clone()],
        };
        let receipt_json =
            serde_json::to_string(&saved_receipt).map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "UPDATE operations SET request_fingerprint=?1,receipt_json=?3 WHERE id=?2",
            params![fingerprint, op_id.to_string(), receipt_json],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.execute(
            "INSERT INTO audit_events (id,actor_id,action,details,created_at)
            VALUES (?1,?2,'stock.changed',?3,?4)",
            params![
                Uuid::new_v4().to_string(),
                cmd.actor_id.to_string(),
                event_payload.to_string(),
                now_str
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(OperationReceipt {
            operation_id: op_id,
            kind: OperationKind::Adjust,
            movements: vec![movement],
            resulting_balances: vec![balance],
        })
    }

    fn get_balance(
        &self,
        item_id: &ItemId,
        location_id: &LocationId,
    ) -> CoreResult<Option<StockBalance>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, location_id, qty, updated_at FROM stock_balances WHERE item_id = ?1 AND location_id = ?2")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(
            [item_id.0.to_string(), location_id.0.to_string()],
            row_to_balance,
        );

        match res {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn get_item_balances(&self, item_id: &ItemId) -> CoreResult<Vec<StockBalance>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, location_id, qty, updated_at FROM stock_balances WHERE item_id = ?1 AND qty > 0 ORDER BY qty DESC")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([item_id.0.to_string()], row_to_balance)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut balances = Vec::new();
        for row in rows {
            balances.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(balances)
    }

    fn get_location_balances(&self, location_id: &LocationId) -> CoreResult<Vec<StockBalance>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, location_id, qty, updated_at FROM stock_balances WHERE location_id = ?1 AND qty > 0 ORDER BY qty DESC")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([location_id.0.to_string()], row_to_balance)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut balances = Vec::new();
        for row in rows {
            balances.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(balances)
    }

    fn get_low_stock(&self) -> CoreResult<Vec<LowStockAlert>> {
        // Find all (item, location) where balance <= min_qty
        let sql = "
            WITH item_locations AS (
                SELECT item_id, stock_location_id AS location_id FROM stock_thresholds WHERE stock_location_id IS NOT NULL
                UNION
                SELECT item_id, location_id FROM stock_balances
                UNION
                SELECT st.item_id, '00000000-0000-0000-0000-000000000000'
                FROM stock_thresholds st WHERE st.stock_location_id IS NULL
                AND NOT EXISTS (SELECT 1 FROM stock_balances b WHERE b.item_id=st.item_id)
                AND NOT EXISTS (SELECT 1 FROM stock_thresholds t WHERE t.item_id=st.item_id AND t.stock_location_id IS NOT NULL)
            ),
            effective_thresholds AS (
                SELECT
                    il.item_id,
                    il.location_id,
                    COALESCE(specific.min_qty, global.min_qty) AS min_qty
                FROM item_locations il
                LEFT JOIN stock_thresholds specific
                    ON specific.item_id = il.item_id AND specific.stock_location_id = il.location_id
                LEFT JOIN stock_thresholds global
                    ON global.item_id = il.item_id AND global.stock_location_id IS NULL
                WHERE COALESCE(specific.min_qty, global.min_qty) IS NOT NULL
            )
            SELECT
                i.id AS item_id,
                i.name AS item_name,
                i.sku AS item_sku,
                et.location_id,
                COALESCE(l.name, 'Unassigned') AS location_name,
                COALESCE(b.qty, 0) AS on_hand,
                et.min_qty
            FROM effective_thresholds et
            JOIN items i ON i.id = et.item_id AND i.archived_at IS NULL
            LEFT JOIN locations l ON l.id = et.location_id
            LEFT JOIN stock_balances b ON b.item_id = et.item_id AND b.location_id = et.location_id
            WHERE COALESCE(b.qty, 0) <= et.min_qty
            ORDER BY (COALESCE(b.qty, 0) - et.min_qty) ASC, i.name ASC;
        ";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let item_id_str: String = row.get(0)?;
                let item_name: String = row.get(1)?;
                let item_sku: Option<String> = row.get(2)?;
                let loc_id_str: String = row.get(3)?;
                let location_name: String = row.get(4)?;
                let on_hand: i64 = row.get(5)?;
                let min_qty: u32 = row.get(6)?;

                Ok(LowStockAlert {
                    item_id: ItemId(Uuid::parse_str(&item_id_str).unwrap_or_default()),
                    item_name,
                    item_sku,
                    location_id: LocationId(Uuid::parse_str(&loc_id_str).unwrap_or_default()),
                    location_name,
                    on_hand: on_hand.max(0) as u32,
                    min_qty,
                })
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut alerts = Vec::new();
        for row in rows {
            alerts.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(alerts)
    }
}

/// SQLite implementation of history queries.
pub struct SqliteHistoryQuery<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteHistoryQuery<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn fetch_movements(&self, op_id: &str) -> CoreResult<Vec<StockMovement>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, operation_id, item_id, location_id, delta_qty, created_at
                 FROM stock_movements WHERE operation_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([op_id], row_to_movement)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut movements = Vec::new();
        for row in rows {
            movements.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(movements)
    }
}

impl HistoryQuery for SqliteHistoryQuery<'_> {
    fn by_item(&self, item_id: &ItemId, limit: usize) -> CoreResult<Vec<OperationSummary>> {
        let sql = "
            SELECT DISTINCT o.id, o.kind, o.actor_id, o.source, o.reason, o.idempotency_key, o.created_at
            FROM operations o
            JOIN stock_movements m ON m.operation_id = o.id
            WHERE m.item_id = ?1
            ORDER BY o.created_at DESC
            LIMIT ?2
        ";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![item_id.0.to_string(), limit as i64],
                row_to_operation,
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut summaries = Vec::new();
        for row in rows {
            let op = row.map_err(|e| CoreError::Storage(e.to_string()))?;
            let movements = self.fetch_movements(&op.id.0.to_string())?;
            summaries.push(OperationSummary {
                operation: op,
                movements,
            });
        }
        Ok(summaries)
    }

    fn by_location(
        &self,
        location_id: &LocationId,
        limit: usize,
    ) -> CoreResult<Vec<OperationSummary>> {
        let sql = "
            SELECT DISTINCT o.id, o.kind, o.actor_id, o.source, o.reason, o.idempotency_key, o.created_at
            FROM operations o
            JOIN stock_movements m ON m.operation_id = o.id
            WHERE m.location_id = ?1
            ORDER BY o.created_at DESC
            LIMIT ?2
        ";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![location_id.0.to_string(), limit as i64],
                row_to_operation,
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut summaries = Vec::new();
        for row in rows {
            let op = row.map_err(|e| CoreError::Storage(e.to_string()))?;
            let movements = self.fetch_movements(&op.id.0.to_string())?;
            summaries.push(OperationSummary {
                operation: op,
                movements,
            });
        }
        Ok(summaries)
    }

    fn by_actor(&self, actor_id: &ActorId, limit: usize) -> CoreResult<Vec<OperationSummary>> {
        let sql = "
            SELECT id, kind, actor_id, source, reason, idempotency_key, created_at
            FROM operations
            WHERE actor_id = ?1
            ORDER BY created_at DESC
            LIMIT ?2
        ";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(
                params![actor_id.0.to_string(), limit as i64],
                row_to_operation,
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut summaries = Vec::new();
        for row in rows {
            let op = row.map_err(|e| CoreError::Storage(e.to_string()))?;
            let movements = self.fetch_movements(&op.id.0.to_string())?;
            summaries.push(OperationSummary {
                operation: op,
                movements,
            });
        }
        Ok(summaries)
    }

    fn by_operation(&self, op_id: &OperationId) -> CoreResult<Option<OperationDetail>> {
        let sql = "
            SELECT o.id, o.kind, o.actor_id, o.source, o.reason, o.idempotency_key, o.created_at, a.display_name
            FROM operations o
            JOIN actors a ON a.id = o.actor_id
            WHERE o.id = ?1
        ";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let res = stmt.query_row([op_id.0.to_string()], |row| {
            let op = row_to_operation(row)?;
            let actor_name: String = row.get(7)?;
            Ok((op, actor_name))
        });

        match res {
            Ok((op, actor_name)) => {
                let movements = self.fetch_movements(&op.id.0.to_string())?;
                Ok(Some(OperationDetail {
                    operation: op,
                    movements,
                    actor_name,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }
}

/// SQLite implementation of the reconciliation service.
pub struct SqliteReconciliationService<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteReconciliationService<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl ReconciliationService for SqliteReconciliationService<'_> {
    fn check(&self) -> CoreResult<ReconciliationReport> {
        let total_balances: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM stock_balances", [], |row| row.get(0))
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let sql = "
            WITH ledger AS (
                SELECT item_id, location_id, SUM(delta_qty) AS ledger_qty
                FROM stock_movements
                GROUP BY item_id, location_id
            ),
            all_pairs AS (
                SELECT item_id, location_id FROM stock_balances
                UNION
                SELECT item_id, location_id FROM ledger
            )
            SELECT
                p.item_id,
                p.location_id,
                COALESCE(b.qty, 0) AS cached_qty,
                COALESCE(l.ledger_qty, 0) AS ledger_qty
            FROM all_pairs p
            LEFT JOIN stock_balances b ON b.item_id = p.item_id AND b.location_id = p.location_id
            LEFT JOIN ledger l ON l.item_id = p.item_id AND l.location_id = p.location_id
            WHERE COALESCE(b.qty, 0) != COALESCE(l.ledger_qty, 0);
        ";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let item_str: String = row.get(0)?;
                let loc_str: String = row.get(1)?;
                let cached_qty: i64 = row.get(2)?;
                let ledger_qty: i64 = row.get(3)?;

                Ok(ReconciliationMismatch {
                    item_id: ItemId(Uuid::parse_str(&item_str).unwrap_or_default()),
                    location_id: LocationId(Uuid::parse_str(&loc_str).unwrap_or_default()),
                    cached_qty,
                    ledger_qty,
                })
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut mismatches = Vec::new();
        for row in rows {
            mismatches.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }

        Ok(ReconciliationReport {
            checked_at: OffsetDateTime::now_utc(),
            total_balances,
            mismatches,
            repaired: false,
        })
    }

    fn repair(&self, actor_id: &ActorId, reason: &str) -> CoreResult<ReconciliationReport> {
        if !SqliteInventoryService::new(self.conn)
            .validate_actor(actor_id)?
            .can_adjust()
        {
            return Err(CoreError::Unauthorized(
                "Manager or Admin required for repair".into(),
            ));
        }
        if reason.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "A repair reason is required".into(),
            ));
        }
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 1. Clear current cached balances
        let invalid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stock_movements GROUP BY item_id,location_id HAVING SUM(delta_qty)<0 OR SUM(delta_qty)>4294967295)",[],|r|r.get(0))
            .map_err(|e|CoreError::Storage(e.to_string()))?;
        if invalid {
            return Err(CoreError::Storage(
                "Ledger quantities are outside the supported range; repair was cancelled".into(),
            ));
        }
        tx.execute("DELETE FROM stock_balances", [])
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. Rebuild from ledger sums where sum >= 0
        tx.execute(
            "INSERT INTO stock_balances (item_id, location_id, qty, updated_at)
             SELECT item_id, location_id, SUM(delta_qty), ?1
             FROM stock_movements
             GROUP BY item_id, location_id
             HAVING SUM(delta_qty) >= 0",
            [&now_str],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 3. Record repair in event outbox
        let payload = serde_json::json!({
            "schema_version": 1,
            "actor_id": actor_id.to_string(),
            "reason": reason,
            "repaired_at": now_str,
        });

        tx.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'DatabaseReconciliationRepaired', ?2, ?3, ?4)",
            params![
                Uuid::new_v4().to_string(),
                actor_id.0.to_string(),
                payload.to_string(),
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut report = self.check()?;
        report.repaired = true;
        Ok(report)
    }
}

/// SQLite implementation of the threshold repository.
pub struct SqliteThresholdRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteThresholdRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl ThresholdRepository for SqliteThresholdRepo<'_> {
    fn set(&self, threshold: &StockThreshold) -> CoreResult<()> {
        let id = Uuid::new_v4().to_string();
        let loc_id_str = threshold.stock_location_id.map(|l| l.0.to_string());

        self.conn
            .execute(
                "INSERT INTO stock_thresholds (id, item_id, stock_location_id, min_qty)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT DO UPDATE SET min_qty = excluded.min_qty",
                params![
                    id,
                    threshold.item_id.0.to_string(),
                    loc_id_str,
                    threshold.min_qty
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(())
    }

    fn get_for_item(&self, item_id: &ItemId) -> CoreResult<Vec<StockThreshold>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, stock_location_id, min_qty FROM stock_thresholds WHERE item_id = ?1")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([item_id.0.to_string()], |row| {
                let item_str: String = row.get(0)?;
                let loc_str: Option<String> = row.get(1)?;
                let min_qty: u32 = row.get(2)?;

                Ok(StockThreshold {
                    item_id: ItemId(Uuid::parse_str(&item_str).unwrap_or_default()),
                    stock_location_id: loc_str
                        .map(|s| LocationId(Uuid::parse_str(&s).unwrap_or_default())),
                    min_qty,
                })
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut thresholds = Vec::new();
        for row in rows {
            thresholds.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(thresholds)
    }

    fn remove(&self, item_id: &ItemId, location_id: Option<&LocationId>) -> CoreResult<()> {
        let loc_str = location_id.map(|l| l.0.to_string());
        self.conn
            .execute(
                "DELETE FROM stock_thresholds WHERE item_id = ?1 AND stock_location_id IS ?2",
                params![item_id.0.to_string(), loc_str],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }
}

// ── Helpers ──

fn row_to_movement(row: &rusqlite::Row<'_>) -> rusqlite::Result<StockMovement> {
    let id_str: String = row.get(0)?;
    let op_str: String = row.get(1)?;
    let item_str: String = row.get(2)?;
    let loc_str: String = row.get(3)?;
    let delta_qty: i64 = row.get(4)?;
    let created_str: String = row.get(5)?;

    Ok(StockMovement {
        id: MovementId(Uuid::parse_str(&id_str).unwrap_or_default()),
        operation_id: OperationId(Uuid::parse_str(&op_str).unwrap_or_default()),
        item_id: ItemId(Uuid::parse_str(&item_str).unwrap_or_default()),
        location_id: LocationId(Uuid::parse_str(&loc_str).unwrap_or_default()),
        delta_qty,
        created_at: parse_time(&created_str),
    })
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Operation> {
    let id_str: String = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let actor_str: String = row.get(2)?;
    let source_str: String = row.get(3)?;
    let reason: Option<String> = row.get(4)?;
    let idempotency_key: Option<String> = row.get(5)?;
    let created_str: String = row.get(6)?;

    Ok(Operation {
        id: OperationId(Uuid::parse_str(&id_str).unwrap_or_default()),
        kind: kind_str.parse().unwrap_or(OperationKind::Receive),
        actor_id: ActorId(Uuid::parse_str(&actor_str).unwrap_or_default()),
        source: source_str.parse().unwrap_or(OperationSource::Cli),
        reason,
        idempotency_key,
        created_at: parse_time(&created_str),
    })
}

fn row_to_balance(row: &rusqlite::Row<'_>) -> rusqlite::Result<StockBalance> {
    let item_str: String = row.get(0)?;
    let loc_str: String = row.get(1)?;
    let qty: i64 = row.get(2)?;
    let updated_str: String = row.get(3)?;

    Ok(StockBalance {
        item_id: ItemId(Uuid::parse_str(&item_str).unwrap_or_default()),
        location_id: LocationId(Uuid::parse_str(&loc_str).unwrap_or_default()),
        qty: u32::try_from(qty).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, qty))?,
        updated_at: parse_time(&updated_str),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;
    use crate::repo::actor::SqliteActorRepo;
    use crate::repo::item::SqliteItemRepo;
    use crate::repo::location::SqliteLocationRepo;
    use slim_core::model::actor::NewActor;
    use slim_core::model::item::NewItem;
    use slim_core::model::location::{LocationKind, NewLocation};
    use slim_core::service::{ActorRepository, ItemRepository, LocationRepository};

    struct TestFixture {
        conn: Connection,
        actor_id: ActorId,
        item_id: ItemId,
        loc_a: LocationId,
        loc_b: LocationId,
    }

    fn setup_fixture() -> TestFixture {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();

        let actor = SqliteActorRepo::new(&conn)
            .create(
                &NewActor::new("Alice Operator")
                    .with_os_username("alice")
                    .with_role(Role::Manager),
            )
            .unwrap();

        let item = SqliteItemRepo::new(&conn)
            .create(&NewItem {
                sku: Some("M3X10".to_string()),
                name: "M3x10 Screw".to_string(),
                unit: "each".to_string(),
                description: None,
                metadata: None,
            })
            .unwrap();

        let loc_repo = SqliteLocationRepo::new(&conn);
        let room = loc_repo
            .create(&NewLocation {
                parent_id: None,
                name: "Workshop".to_string(),
                kind: LocationKind::Room,
            })
            .unwrap();

        let loc_a = loc_repo
            .create(&NewLocation {
                parent_id: Some(room.id),
                name: "Drawer 1".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap();

        let loc_b = loc_repo
            .create(&NewLocation {
                parent_id: Some(room.id),
                name: "Drawer 2".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap();

        TestFixture {
            conn,
            actor_id: actor.id,
            item_id: item.id,
            loc_a: loc_a.id,
            loc_b: loc_b.id,
        }
    }

    #[test]
    fn test_receive_creates_movement_and_balance() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        let receipt = svc
            .receive(ReceiveStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                destination_id: f.loc_a,
                quantity: 100,
                source: OperationSource::Cli,
                reason: Some("Initial shipment".to_string()),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements.len(), 1);
        assert_eq!(receipt.movements[0].delta_qty, 100);
        assert_eq!(receipt.resulting_balances.len(), 1);
        assert_eq!(receipt.resulting_balances[0].qty, 100);

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 100);
    }

    #[test]
    fn test_issue_decrements_balance() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .issue(IssueStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                source_id: f.loc_a,
                quantity: 6,
                source: OperationSource::Cli,
                reason: Some("Assembly".to_string()),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements[0].delta_qty, -6);
        assert_eq!(receipt.resulting_balances[0].qty, 94);

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 94);
    }

    #[test]
    fn test_issue_rejects_insufficient_stock() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 94,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let res = svc.issue(IssueStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            source_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });

        assert!(matches!(
            res,
            Err(CoreError::InsufficientStock {
                requested: 100,
                available: 94
            })
        ));

        // Balance remains unchanged
        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 94);
    }

    #[test]
    fn test_issue_rejects_zero_balance() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        let res = svc.issue(IssueStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            source_id: f.loc_a,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });

        assert!(matches!(
            res,
            Err(CoreError::InsufficientStock {
                requested: 10,
                available: 0
            })
        ));
    }

    #[test]
    fn test_transfer_is_atomic_two_movements() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .transfer(TransferStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                from_id: f.loc_a,
                to_id: f.loc_b,
                quantity: 20,
                source: OperationSource::Cli,
                reason: Some("Relocation".to_string()),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements.len(), 2);
        assert_eq!(receipt.movements[0].delta_qty, -20);
        assert_eq!(receipt.movements[1].delta_qty, 20);

        let bal_a = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        let bal_b = svc.get_balance(&f.item_id, &f.loc_b).unwrap().unwrap();
        assert_eq!(bal_a.qty, 80);
        assert_eq!(bal_b.qty, 20);
    }

    #[test]
    fn test_transfer_conserves_total_quantity() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        svc.transfer(TransferStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            from_id: f.loc_a,
            to_id: f.loc_b,
            quantity: 45,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let bal_a = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap().qty;
        let bal_b = svc.get_balance(&f.item_id, &f.loc_b).unwrap().unwrap().qty;
        assert_eq!(bal_a + bal_b, 100);
    }

    #[test]
    fn test_transfer_rejects_insufficient_source() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let res = svc.transfer(TransferStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            from_id: f.loc_a,
            to_id: f.loc_b,
            quantity: 50,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });

        assert!(matches!(
            res,
            Err(CoreError::InsufficientStock {
                requested: 50,
                available: 10
            })
        ));
    }

    #[test]
    fn test_adjust_to_higher_quantity() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 50,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .adjust(AdjustStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                location_id: f.loc_a,
                new_quantity: 65,
                source: OperationSource::Cli,
                reason: "Found extra box".to_string(),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements[0].delta_qty, 15);
        assert_eq!(receipt.resulting_balances[0].qty, 65);

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 65);
    }

    #[test]
    fn test_adjust_to_lower_quantity() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 50,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .adjust(AdjustStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                location_id: f.loc_a,
                new_quantity: 40,
                source: OperationSource::Cli,
                reason: "Damaged parts discarded".to_string(),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements[0].delta_qty, -10);
        assert_eq!(receipt.resulting_balances[0].qty, 40);
    }

    #[test]
    fn test_adjust_to_zero() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 50,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .adjust(AdjustStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                location_id: f.loc_a,
                new_quantity: 0,
                source: OperationSource::Cli,
                reason: "Audited count is zero".to_string(),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements[0].delta_qty, -50);
        assert_eq!(receipt.resulting_balances[0].qty, 0);

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 0);
    }

    #[test]
    fn test_idempotency_key_prevents_duplicate() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        let cmd = ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: Some("IDEMPOTENT-001".to_string()),
        };

        let receipt1 = svc.receive(cmd.clone()).unwrap();
        let receipt2 = svc.receive(cmd).unwrap();

        assert_eq!(receipt1.operation_id, receipt2.operation_id);

        // Quantity should still be 100, NOT 200
        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 100);
    }

    #[test]
    fn test_get_balance_returns_correct_value() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap();
        assert!(bal.is_none());

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 42,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 42);
    }

    #[test]
    fn test_get_item_balances_multiple_locations() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 30,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_b,
            quantity: 70,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let balances = svc.get_item_balances(&f.item_id).unwrap();
        assert_eq!(balances.len(), 2);
        assert_eq!(balances[0].qty, 70); // ordered desc
        assert_eq!(balances[1].qty, 30);
    }

    #[test]
    fn test_get_low_stock_returns_below_threshold() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let thresh_repo = SqliteThresholdRepo::new(&f.conn);

        // Set global threshold: 50
        thresh_repo
            .set(&StockThreshold {
                item_id: f.item_id,
                stock_location_id: None,
                min_qty: 50,
            })
            .unwrap();

        // Receive 40 at loc_a (< 50)
        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 40,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let alerts = svc.get_low_stock().unwrap();
        assert!(!alerts.is_empty());
        let alert = alerts.iter().find(|a| a.location_id == f.loc_a).unwrap();
        assert_eq!(alert.on_hand, 40);
        assert_eq!(alert.min_qty, 50);
    }

    #[test]
    fn test_history_by_item_ordered_descending() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let hist = SqliteHistoryQuery::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: Some("First".to_string()),
            idempotency_key: None,
        })
        .unwrap();

        svc.issue(IssueStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            source_id: f.loc_a,
            quantity: 10,
            source: OperationSource::Cli,
            reason: Some("Second".to_string()),
            idempotency_key: None,
        })
        .unwrap();

        let items = hist.by_item(&f.item_id, 10).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].operation.kind, OperationKind::Issue);
        assert_eq!(items[1].operation.kind, OperationKind::Receive);
    }

    #[test]
    fn test_history_by_operation_includes_actor_name() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let hist = SqliteHistoryQuery::new(&f.conn);

        let receipt = svc
            .receive(ReceiveStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                destination_id: f.loc_a,
                quantity: 100,
                source: OperationSource::Cli,
                reason: Some("Test".to_string()),
                idempotency_key: None,
            })
            .unwrap();

        let detail = hist.by_operation(&receipt.operation_id).unwrap().unwrap();
        assert_eq!(detail.actor_name, "Alice Operator");
        assert_eq!(detail.movements.len(), 1);
    }

    #[test]
    fn test_reconcile_clean_database() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let recon = SqliteReconciliationService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let report = recon.check().unwrap();
        assert_eq!(report.mismatches.len(), 0);
    }

    #[test]
    fn test_reconcile_detects_mismatch() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let recon = SqliteReconciliationService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        // Corrupt the cached balance manually
        f.conn
            .execute(
                "UPDATE stock_balances SET qty = 42 WHERE item_id = ?1 AND location_id = ?2",
                params![f.item_id.0.to_string(), f.loc_a.0.to_string()],
            )
            .unwrap();

        let report = recon.check().unwrap();
        assert_eq!(report.mismatches.len(), 1);
        assert_eq!(report.mismatches[0].cached_qty, 42);
        assert_eq!(report.mismatches[0].ledger_qty, 100);
    }

    #[test]
    fn test_reconcile_repair_fixes_cache() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let recon = SqliteReconciliationService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        // Corrupt cache
        f.conn
            .execute(
                "UPDATE stock_balances SET qty = 0 WHERE item_id = ?1 AND location_id = ?2",
                params![f.item_id.0.to_string(), f.loc_a.0.to_string()],
            )
            .unwrap();

        let report = recon.repair(&f.actor_id, "Fix corrupted cache").unwrap();
        assert!(report.repaired);
        assert_eq!(report.mismatches.len(), 0);

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 100);
    }

    #[test]
    fn test_outbox_event_created_on_receive() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let count: i64 = f
            .conn
            .query_row(
                "SELECT COUNT(*) FROM event_outbox WHERE event_type = 'StockReceived'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_inactive_actor_rejected() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        // Deactivate actor
        f.conn
            .execute(
                "UPDATE actors SET active = 0 WHERE id = ?1",
                [f.actor_id.0.to_string()],
            )
            .unwrap();

        let res = svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });
        assert!(matches!(res, Err(CoreError::InvalidInput(_))));
    }

    #[test]
    fn test_archived_item_rejected() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        // Archive item
        f.conn
            .execute(
                "UPDATE items SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
                [f.item_id.0.to_string()],
            )
            .unwrap();

        let res = svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });
        assert!(matches!(res, Err(CoreError::InvalidInput(_))));
    }

    #[test]
    fn test_non_stock_holding_location_rejected() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        // Create a room (cannot hold stock)
        let room = SqliteLocationRepo::new(&f.conn)
            .create(&NewLocation {
                parent_id: None,
                name: "Hallway".to_string(),
                kind: LocationKind::Room,
            })
            .unwrap();

        let res = svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: room.id,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });
        assert!(matches!(res, Err(CoreError::InvalidInput(_))));
    }

    #[test]
    fn test_adjust_same_quantity_delta_zero() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 50,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .adjust(AdjustStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                location_id: f.loc_a,
                new_quantity: 50,
                source: OperationSource::Cli,
                reason: "Confirmed exact match".to_string(),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.movements[0].delta_qty, 0);
        assert_eq!(receipt.resulting_balances[0].qty, 50);
    }

    #[test]
    fn test_multiple_receives_accumulate_balance() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 30,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 70,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 100);
    }

    #[test]
    fn test_history_by_location_ordered_descending() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let hist = SqliteHistoryQuery::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: Some("First".to_string()),
            idempotency_key: None,
        })
        .unwrap();

        svc.issue(IssueStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            source_id: f.loc_a,
            quantity: 25,
            source: OperationSource::Cli,
            reason: Some("Second".to_string()),
            idempotency_key: None,
        })
        .unwrap();

        let items = hist.by_location(&f.loc_a, 10).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].operation.kind, OperationKind::Issue);
        assert_eq!(items[1].operation.kind, OperationKind::Receive);
    }

    #[test]
    fn test_history_by_actor_ordered_descending() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let hist = SqliteHistoryQuery::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let items = hist.by_actor(&f.actor_id, 10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].operation.kind, OperationKind::Receive);
    }

    #[test]
    fn test_history_by_operation_nonexistent() {
        let f = setup_fixture();
        let hist = SqliteHistoryQuery::new(&f.conn);
        let missing = hist.by_operation(&OperationId::new()).unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_threshold_crud_and_query() {
        let f = setup_fixture();
        let thresh_repo = SqliteThresholdRepo::new(&f.conn);

        thresh_repo
            .set(&StockThreshold {
                item_id: f.item_id,
                stock_location_id: None,
                min_qty: 30,
            })
            .unwrap();

        let list = thresh_repo.get_for_item(&f.item_id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].min_qty, 30);
        assert!(list[0].stock_location_id.is_none());

        thresh_repo.remove(&f.item_id, None).unwrap();
        let list_after = thresh_repo.get_for_item(&f.item_id).unwrap();
        assert!(list_after.is_empty());
    }

    #[test]
    fn test_threshold_location_override() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let thresh_repo = SqliteThresholdRepo::new(&f.conn);

        // Global threshold: 10
        thresh_repo
            .set(&StockThreshold {
                item_id: f.item_id,
                stock_location_id: None,
                min_qty: 10,
            })
            .unwrap();

        // Location A override: 50
        thresh_repo
            .set(&StockThreshold {
                item_id: f.item_id,
                stock_location_id: Some(f.loc_a),
                min_qty: 50,
            })
            .unwrap();

        // Receive 40 at Location A (below 50 override, but above 10 global)
        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 40,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let alerts = svc.get_low_stock().unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].location_id, f.loc_a);
        assert_eq!(alerts[0].min_qty, 50);
        assert_eq!(alerts[0].on_hand, 40);
    }

    #[test]
    fn test_transfer_between_three_locations() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        // Create loc_c
        let loc_c = SqliteLocationRepo::new(&f.conn)
            .create(&NewLocation {
                parent_id: None,
                name: "Drawer 3".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap()
            .id;

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        // Transfer 30 from A to B
        svc.transfer(TransferStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            from_id: f.loc_a,
            to_id: f.loc_b,
            quantity: 30,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        // Transfer 10 from B to C
        svc.transfer(TransferStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            from_id: f.loc_b,
            to_id: loc_c,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let bal_a = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap().qty;
        let bal_b = svc.get_balance(&f.item_id, &f.loc_b).unwrap().unwrap().qty;
        let bal_c = svc.get_balance(&f.item_id, &loc_c).unwrap().unwrap().qty;

        assert_eq!(bal_a, 70);
        assert_eq!(bal_b, 20);
        assert_eq!(bal_c, 10);
        assert_eq!(bal_a + bal_b + bal_c, 100);
    }

    #[test]
    fn test_issue_all_stock_to_zero() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 25,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        let receipt = svc
            .issue(IssueStock {
                actor_id: f.actor_id,
                item_id: f.item_id,
                source_id: f.loc_a,
                quantity: 25,
                source: OperationSource::Cli,
                reason: Some("Consumed all".to_string()),
                idempotency_key: None,
            })
            .unwrap();

        assert_eq!(receipt.resulting_balances[0].qty, 0);
        let bal = svc.get_balance(&f.item_id, &f.loc_a).unwrap().unwrap();
        assert_eq!(bal.qty, 0);
    }

    #[test]
    fn test_reconcile_check_after_multiple_operations() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let recon = SqliteReconciliationService::new(&f.conn);

        svc.receive(ReceiveStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 100,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        svc.issue(IssueStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            source_id: f.loc_a,
            quantity: 15,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        svc.transfer(TransferStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            from_id: f.loc_a,
            to_id: f.loc_b,
            quantity: 25,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        svc.adjust(AdjustStock {
            actor_id: f.actor_id,
            item_id: f.item_id,
            location_id: f.loc_a,
            new_quantity: 55,
            source: OperationSource::Cli,
            reason: "Audit count".to_string(),
            idempotency_key: None,
        })
        .unwrap();

        let report = recon.check().unwrap();
        assert_eq!(report.mismatches.len(), 0);
    }

    #[test]
    fn test_viewer_role_cannot_transact() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let actor_repo = SqliteActorRepo::new(&f.conn);

        let viewer = actor_repo
            .create(&NewActor::new("Victor Viewer").with_role(Role::Viewer))
            .unwrap();

        let res = svc.receive(ReceiveStock {
            actor_id: viewer.id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        });

        assert!(matches!(res, Err(CoreError::Unauthorized(_))));
    }

    #[test]
    fn test_operator_role_cannot_adjust() {
        let f = setup_fixture();
        let svc = SqliteInventoryService::new(&f.conn);
        let actor_repo = SqliteActorRepo::new(&f.conn);

        let operator = actor_repo
            .create(&NewActor::new("Ollie Operator").with_role(Role::Operator))
            .unwrap();

        // Operator CAN receive
        svc.receive(ReceiveStock {
            actor_id: operator.id,
            item_id: f.item_id,
            destination_id: f.loc_a,
            quantity: 20,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();

        // Operator CANNOT adjust
        let res = svc.adjust(AdjustStock {
            actor_id: operator.id,
            item_id: f.item_id,
            location_id: f.loc_a,
            new_quantity: 50,
            source: OperationSource::Cli,
            reason: "Attempted adjust".to_string(),
            idempotency_key: None,
        });

        assert!(matches!(res, Err(CoreError::Unauthorized(_))));
    }
}
