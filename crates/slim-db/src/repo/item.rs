//! Item repository — SQLite implementation.

use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::{ActorId, Role};
use slim_core::model::auth::NewAuditEvent;
use slim_core::model::item::*;
use slim_core::service::{ActorRepository, AuditRepository, ItemRepository};
use time::OffsetDateTime;

use super::actor::{format_time, parse_time};

pub struct SqliteItemRepo<'a> {
    pub(super) conn: &'a Connection,
}

impl<'a> SqliteItemRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Remove an item from the active catalog without erasing its ledger.
    pub fn delete_as_admin(&self, id: &ItemId, actor: &ActorId) -> CoreResult<()> {
        self.set_deleted(id, actor, true)
    }

    pub fn restore_as_admin(&self, id: &ItemId, actor: &ActorId) -> CoreResult<()> {
        self.set_deleted(id, actor, false)
    }

    fn set_deleted(&self, id: &ItemId, actor: &ActorId, deleted: bool) -> CoreResult<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        let current = super::actor::SqliteActorRepo::new(self.conn).get_by_id(actor)?;
        if !current.is_some_and(|a| a.active && a.role == Role::Admin) {
            return Err(CoreError::Unauthorized(
                "Only an active Admin can delete or restore inventory items".into(),
            ));
        }
        let item = self.get_by_id(id)?.ok_or_else(|| CoreError::NotFound {
            entity: "item",
            id: id.to_string(),
        })?;
        if item.archived_at.is_some() == deleted {
            return Err(CoreError::InvalidInput(
                if deleted {
                    "Item is already deleted"
                } else {
                    "Item is already active"
                }
                .into(),
            ));
        }
        if deleted {
            self.archive_in_transaction(id)?;
        } else {
            self.conn
                .execute(
                    "UPDATE items SET archived_at=NULL, updated_at=?1 WHERE id=?2",
                    rusqlite::params![format_time(OffsetDateTime::now_utc()), id.to_string()],
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;
        }
        super::audit::SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new(if deleted {
                "item.deleted"
            } else {
                "item.restored"
            })
            .with_actor(*actor)
            .with_details(format!("{} ({})", item.name, item.id)),
        )?;
        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn archive_in_transaction(&self, id: &ItemId) -> CoreResult<()> {
        let has_stock: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM stock_balances WHERE item_id=?1 AND qty>0)",
                [id.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        if has_stock {
            return Err(CoreError::InvalidInput(
                "Issue remaining stock or adjust its count to zero before deleting this item"
                    .into(),
            ));
        }
        let affected = self.conn.execute("UPDATE items SET archived_at=?1, updated_at=?1 WHERE id=?2 AND archived_at IS NULL",
            rusqlite::params![format_time(OffsetDateTime::now_utc()), id.to_string()])
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        if affected == 0 {
            return Err(CoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            });
        }
        Ok(())
    }
}

trait OptionalExt<T> {
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

impl ItemRepository for SqliteItemRepo<'_> {
    fn create(&self, item: &NewItem) -> CoreResult<Item> {
        item.validate()?;

        let id = ItemId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);
        let metadata_json = item
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default());

        self.conn
            .execute(
                "INSERT INTO items (id, sku, name, unit, description, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                rusqlite::params![
                    id.0.to_string(),
                    item.sku.as_deref(),
                    item.name.trim(),
                    item.unit.trim(),
                    item.description.as_deref(),
                    metadata_json.as_deref(),
                    now_str,
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("UNIQUE constraint failed") =>
                {
                    CoreError::DuplicateSku(item.sku.clone().unwrap_or_default())
                }
                other => CoreError::Storage(other.to_string()),
            })?;

        Ok(Item {
            id,
            sku: item.sku.clone(),
            name: item.name.trim().to_string(),
            unit: item.unit.trim().to_string(),
            description: item.description.clone(),
            metadata: item.metadata.clone(),
            archived_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_by_id(&self, id: &ItemId) -> CoreResult<Option<Item>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
                 FROM items WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        stmt.query_row([id.0.to_string()], row_to_item)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))
    }

    fn get_by_sku(&self, sku: &str) -> CoreResult<Option<Item>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
                 FROM items WHERE sku = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        stmt.query_row([sku], row_to_item)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))
    }

    fn list(&self, filter: &ItemFilter) -> CoreResult<Vec<Item>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if filter
            .include_archived
        {
            if let Some(ref search) = filter.search {
                (
                    "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
                     FROM items WHERE name LIKE ?1 OR sku LIKE ?1 ORDER BY name".to_string(),
                    vec![Box::new(format!("%{search}%"))],
                )
            } else {
                (
                    "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
                     FROM items ORDER BY name".to_string(),
                    vec![],
                )
            }
        } else if let Some(ref search) = filter.search {
            (
                "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
                 FROM items WHERE archived_at IS NULL AND (name LIKE ?1 OR sku LIKE ?1) ORDER BY name".to_string(),
                vec![Box::new(format!("%{search}%"))],
            )
        } else {
            (
                "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
                 FROM items WHERE archived_at IS NULL ORDER BY name".to_string(),
                vec![],
            )
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_item)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(items)
    }

    fn update(&self, id: &ItemId, update: &ItemUpdate) -> CoreResult<Item> {
        let existing = self.get_by_id(id)?.ok_or_else(|| CoreError::NotFound {
            entity: "item",
            id: id.to_string(),
        })?;
        if existing.archived_at.is_some() {
            return Err(CoreError::InvalidInput(
                "Restore the item before editing".into(),
            ));
        }

        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);
        let name = update.name.as_deref().unwrap_or(&existing.name);
        let unit = update.unit.as_deref().unwrap_or(&existing.unit);
        let sku = update
            .sku
            .as_ref()
            .unwrap_or(&existing.sku)
            .as_deref()
            .map(str::trim);
        if sku.is_some_and(str::is_empty) {
            return Err(CoreError::InvalidInput(
                "SKU cannot be blank; clear it instead".into(),
            ));
        }
        if name.trim().is_empty() || unit.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "name and unit must not be empty".into(),
            ));
        }
        let description = match &update.description {
            Some(d) => d.as_deref(),
            None => existing.description.as_deref(),
        };
        let metadata = match &update.metadata {
            Some(m) => m
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default()),
            None => existing
                .metadata
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default()),
        };

        self.conn
            .execute(
                "UPDATE items SET name = ?1, unit = ?2, description = ?3, metadata = ?4, updated_at = ?5, sku = ?7
                 WHERE id = ?6",
                rusqlite::params![
                    name,
                    unit,
                    description,
                    metadata.as_deref(),
                    now_str,
                    id.0.to_string(),
                    sku,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        self.get_by_id(id)?.ok_or_else(|| CoreError::NotFound {
            entity: "item",
            id: id.to_string(),
        })
    }

    fn archive(&self, id: &ItemId) -> CoreResult<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        self.archive_in_transaction(id)?;
        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    let id_str: String = row.get(0)?;
    let sku: Option<String> = row.get(1)?;
    let name: String = row.get(2)?;
    let unit: String = row.get(3)?;
    let description: Option<String> = row.get(4)?;
    let metadata_str: Option<String> = row.get(5)?;
    let archived_str: Option<String> = row.get(6)?;
    let created_str: String = row.get(7)?;
    let updated_str: String = row.get(8)?;

    let id = ItemId(uuid::Uuid::parse_str(&id_str).unwrap_or_default());
    let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());
    let archived_at = archived_str.map(|s| parse_time(&s));

    Ok(Item {
        id,
        sku,
        name,
        unit,
        description,
        metadata,
        archived_at,
        created_at: parse_time(&created_str),
        updated_at: parse_time(&updated_str),
    })
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
    fn create_and_get_item() {
        let conn = setup();
        let repo = SqliteItemRepo::new(&conn);

        let item = repo
            .create(&NewItem {
                sku: Some("M3X10".to_string()),
                name: "M3x10 Screw".to_string(),
                unit: "each".to_string(),
                description: Some("Stainless".to_string()),
                metadata: None,
            })
            .unwrap();

        assert_eq!(item.sku, Some("M3X10".to_string()));
        assert_eq!(item.name, "M3x10 Screw");

        let found = repo.get_by_id(&item.id).unwrap().unwrap();
        assert_eq!(found.name, "M3x10 Screw");
    }

    #[test]
    fn get_by_sku() {
        let conn = setup();
        let repo = SqliteItemRepo::new(&conn);

        repo.create(&NewItem {
            sku: Some("ESP32".to_string()),
            name: "ESP32 Dev Board".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        })
        .unwrap();

        let found = repo.get_by_sku("ESP32").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "ESP32 Dev Board");
    }

    #[test]
    fn duplicate_sku_rejected() {
        let conn = setup();
        let repo = SqliteItemRepo::new(&conn);

        repo.create(&NewItem {
            sku: Some("DUP".to_string()),
            name: "First".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        })
        .unwrap();

        let result = repo.create(&NewItem {
            sku: Some("DUP".to_string()),
            name: "Second".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        });
        assert!(matches!(result, Err(CoreError::DuplicateSku(_))));
    }

    #[test]
    fn list_excludes_archived() {
        let conn = setup();
        let repo = SqliteItemRepo::new(&conn);

        let item = repo
            .create(&NewItem {
                sku: Some("ARC".to_string()),
                name: "Archived Item".to_string(),
                unit: "each".to_string(),
                description: None,
                metadata: None,
            })
            .unwrap();

        repo.archive(&item.id).unwrap();

        let list = repo.list(&ItemFilter::default()).unwrap();
        assert!(list.is_empty());

        let list_all = repo
            .list(&ItemFilter {
                include_archived: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(list_all.len(), 1);
    }

    #[test]
    fn update_item() {
        let conn = setup();
        let repo = SqliteItemRepo::new(&conn);

        let item = repo
            .create(&NewItem {
                sku: Some("UPD".to_string()),
                name: "Original".to_string(),
                unit: "each".to_string(),
                description: None,
                metadata: None,
            })
            .unwrap();

        let updated = repo
            .update(
                &item.id,
                &ItemUpdate {
                    name: Some("Updated".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Updated");
    }

    #[test]
    fn null_sku_allowed_multiple() {
        let conn = setup();
        let repo = SqliteItemRepo::new(&conn);

        repo.create(&NewItem {
            sku: None,
            name: "No SKU 1".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        })
        .unwrap();

        let result = repo.create(&NewItem {
            sku: None,
            name: "No SKU 2".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        });
        assert!(result.is_ok(), "multiple NULL SKUs should be allowed");
    }
}
