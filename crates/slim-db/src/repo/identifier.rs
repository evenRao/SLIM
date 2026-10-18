//! Identifier repository — SQLite implementation.

use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::identifier::*;
use slim_core::model::item::ItemId;
use slim_core::model::location::LocationId;
use slim_core::service::IdentifierRepository;
use time::OffsetDateTime;

use super::actor::{format_time, parse_time};

pub struct SqliteIdentifierRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteIdentifierRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
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

impl IdentifierRepository for SqliteIdentifierRepo<'_> {
    fn create(&self, ident: &NewIdentifier) -> CoreResult<Identifier> {
        ident.validate()?;

        let id = IdentifierId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        let (item_id, location_id) = match &ident.target {
            IdentifierTarget::Item(iid) => (Some(iid.0.to_string()), None),
            IdentifierTarget::Location(lid) => (None, Some(lid.0.to_string())),
        };

        self.conn
            .execute(
                "INSERT INTO identifiers (id, kind, value, item_id, location_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id.0.to_string(),
                    ident.kind.to_string(),
                    ident.value.trim(),
                    item_id,
                    location_id,
                    now_str,
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("UNIQUE constraint failed") =>
                {
                    CoreError::DuplicateIdentifier {
                        kind: ident.kind.to_string(),
                        value: ident.value.clone(),
                    }
                }
                other => CoreError::Storage(other.to_string()),
            })?;

        Ok(Identifier {
            id,
            kind: ident.kind.clone(),
            value: ident.value.trim().to_string(),
            target: ident.target.clone(),
            created_at: now,
        })
    }

    fn resolve(&self, kind: &IdentifierKind, value: &str) -> CoreResult<Option<IdentifierTarget>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, location_id FROM identifiers WHERE kind = ?1 AND value = ?2 AND archived_at IS NULL")
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(rusqlite::params![kind.to_string(), value], |row| {
                let item_str: Option<String> = row.get(0)?;
                let loc_str: Option<String> = row.get(1)?;
                Ok((item_str, loc_str))
            })
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result.map(|(item_str, loc_str)| {
            if let Some(s) = item_str {
                IdentifierTarget::Item(ItemId(uuid::Uuid::parse_str(&s).unwrap_or_default()))
            } else if let Some(s) = loc_str {
                IdentifierTarget::Location(LocationId(
                    uuid::Uuid::parse_str(&s).unwrap_or_default(),
                ))
            } else {
                // Should not happen due to CHECK constraint
                IdentifierTarget::Item(ItemId(uuid::Uuid::nil()))
            }
        }))
    }

    fn list_for_item(&self, item_id: &ItemId) -> CoreResult<Vec<Identifier>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, value, item_id, location_id, created_at
                 FROM identifiers WHERE item_id = ?1 AND archived_at IS NULL ORDER BY kind, value",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([item_id.0.to_string()], row_to_identifier)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut idents = Vec::new();
        for row in rows {
            idents.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(idents)
    }

    fn list_for_location(&self, location_id: &LocationId) -> CoreResult<Vec<Identifier>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, value, item_id, location_id, created_at
                 FROM identifiers WHERE location_id = ?1 AND archived_at IS NULL ORDER BY kind, value",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([location_id.0.to_string()], row_to_identifier)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut idents = Vec::new();
        for row in rows {
            idents.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(idents)
    }
}

fn row_to_identifier(row: &rusqlite::Row<'_>) -> rusqlite::Result<Identifier> {
    let id_str: String = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let value: String = row.get(2)?;
    let item_str: Option<String> = row.get(3)?;
    let loc_str: Option<String> = row.get(4)?;
    let created_str: String = row.get(5)?;

    let id = IdentifierId(uuid::Uuid::parse_str(&id_str).unwrap_or_default());
    let kind: IdentifierKind = kind_str.parse().unwrap_or(IdentifierKind::Manual);

    let target = if let Some(s) = item_str {
        IdentifierTarget::Item(ItemId(uuid::Uuid::parse_str(&s).unwrap_or_default()))
    } else if let Some(s) = loc_str {
        IdentifierTarget::Location(LocationId(uuid::Uuid::parse_str(&s).unwrap_or_default()))
    } else {
        IdentifierTarget::Item(ItemId(uuid::Uuid::nil()))
    };

    Ok(Identifier {
        id,
        kind,
        value,
        target,
        created_at: parse_time(&created_str),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;
    use crate::repo::item::SqliteItemRepo;
    use slim_core::model::item::NewItem;
    use slim_core::service::ItemRepository;

    fn setup() -> Connection {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn create_test_item(conn: &Connection) -> ItemId {
        let repo = SqliteItemRepo::new(conn);
        repo.create(&NewItem {
            sku: Some("TEST-IDENT".to_string()),
            name: "Test Item".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        })
        .unwrap()
        .id
    }

    #[test]
    fn create_and_resolve_identifier() {
        let conn = setup();
        let item_id = create_test_item(&conn);
        let repo = SqliteIdentifierRepo::new(&conn);

        repo.create(&NewIdentifier {
            kind: IdentifierKind::Qr,
            value: "INV:ITEM:TEST".to_string(),
            target: IdentifierTarget::Item(item_id),
        })
        .unwrap();

        let resolved = repo.resolve(&IdentifierKind::Qr, "INV:ITEM:TEST").unwrap();
        assert!(matches!(resolved, Some(IdentifierTarget::Item(id)) if id == item_id));
    }

    #[test]
    fn duplicate_identifier_rejected() {
        let conn = setup();
        let item_id = create_test_item(&conn);
        let repo = SqliteIdentifierRepo::new(&conn);

        repo.create(&NewIdentifier {
            kind: IdentifierKind::Barcode,
            value: "12345".to_string(),
            target: IdentifierTarget::Item(item_id),
        })
        .unwrap();

        let result = repo.create(&NewIdentifier {
            kind: IdentifierKind::Barcode,
            value: "12345".to_string(),
            target: IdentifierTarget::Item(item_id),
        });
        assert!(matches!(result, Err(CoreError::DuplicateIdentifier { .. })));
    }

    #[test]
    fn list_for_item() {
        let conn = setup();
        let item_id = create_test_item(&conn);
        let repo = SqliteIdentifierRepo::new(&conn);

        repo.create(&NewIdentifier {
            kind: IdentifierKind::Barcode,
            value: "BC-001".to_string(),
            target: IdentifierTarget::Item(item_id),
        })
        .unwrap();

        repo.create(&NewIdentifier {
            kind: IdentifierKind::Qr,
            value: "QR-001".to_string(),
            target: IdentifierTarget::Item(item_id),
        })
        .unwrap();

        let idents = repo.list_for_item(&item_id).unwrap();
        assert_eq!(idents.len(), 2);
    }

    #[test]
    fn unresolved_identifier_returns_none() {
        let conn = setup();
        let repo = SqliteIdentifierRepo::new(&conn);

        let result = repo
            .resolve(&IdentifierKind::Barcode, "NONEXISTENT")
            .unwrap();
        assert!(result.is_none());
    }
}
