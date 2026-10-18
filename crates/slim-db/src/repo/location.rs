//! Location repository — SQLite implementation.

use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::location::*;
use slim_core::service::LocationRepository;
use time::OffsetDateTime;

use super::actor::{format_time, parse_time};

pub struct SqliteLocationRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteLocationRepo<'a> {
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

impl LocationRepository for SqliteLocationRepo<'_> {
    fn create(&self, location: &NewLocation) -> CoreResult<Location> {
        location.validate()?;

        let id = LocationId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);
        let can_hold_stock = location.kind.can_hold_stock();

        self.conn
            .execute(
                "INSERT INTO locations (id, parent_id, name, kind, can_hold_stock, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                rusqlite::params![
                    id.0.to_string(),
                    location.parent_id.map(|p| p.0.to_string()),
                    location.name.trim(),
                    location.kind.to_string(),
                    can_hold_stock,
                    now_str,
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("UNIQUE constraint failed") =>
                {
                    CoreError::InvalidInput(format!(
                        "location '{}' already exists under this parent",
                        location.name
                    ))
                }
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("FOREIGN KEY constraint failed") =>
                {
                    CoreError::NotFound {
                        entity: "parent location",
                        id: location
                            .parent_id
                            .map(|p| p.to_string())
                            .unwrap_or_default(),
                    }
                }
                other => CoreError::Storage(other.to_string()),
            })?;

        Ok(Location {
            id,
            parent_id: location.parent_id,
            name: location.name.trim().to_string(),
            kind: location.kind.clone(),
            can_hold_stock,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_by_id(&self, id: &LocationId) -> CoreResult<Option<Location>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, parent_id, name, kind, can_hold_stock, created_at, updated_at
                 FROM locations WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        stmt.query_row([id.0.to_string()], row_to_location)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))
    }

    fn find_by_name(
        &self,
        name: &str,
        parent_id: Option<&LocationId>,
    ) -> CoreResult<Option<Location>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, parent_id, name, kind, can_hold_stock, created_at, updated_at
                 FROM locations WHERE name = ?1 AND (parent_id IS ?2) AND archived_at IS NULL",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        stmt.query_row(
            rusqlite::params![name, parent_id.map(|p| p.0.to_string())],
            row_to_location,
        )
        .optional()
        .map_err(|e| CoreError::Storage(e.to_string()))
    }

    fn list_children(&self, parent_id: Option<&LocationId>) -> CoreResult<Vec<Location>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, parent_id, name, kind, can_hold_stock, created_at, updated_at
                 FROM locations WHERE parent_id IS ?1 AND archived_at IS NULL ORDER BY name",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([parent_id.map(|p| p.0.to_string())], row_to_location)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut locs = Vec::new();
        for row in rows {
            locs.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(locs)
    }

    fn get_path(&self, id: &LocationId) -> CoreResult<Vec<Location>> {
        let mut path = Vec::new();
        let mut current_id = Some(*id);
        let mut visited = std::collections::HashSet::new();

        while let Some(cid) = current_id {
            if !visited.insert(cid) {
                return Err(CoreError::InvalidInput(
                    "Location hierarchy contains a cycle".into(),
                ));
            }
            let loc = self.get_by_id(&cid)?.ok_or_else(|| CoreError::NotFound {
                entity: "location",
                id: cid.to_string(),
            })?;
            current_id = loc.parent_id;
            path.push(loc);
        }

        path.reverse();
        Ok(path)
    }
}

fn row_to_location(row: &rusqlite::Row<'_>) -> rusqlite::Result<Location> {
    let id_str: String = row.get(0)?;
    let parent_str: Option<String> = row.get(1)?;
    let name: String = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let can_hold_stock: bool = row.get(4)?;
    let created_str: String = row.get(5)?;
    let updated_str: String = row.get(6)?;

    let id = LocationId(uuid::Uuid::parse_str(&id_str).unwrap_or_default());
    let parent_id = parent_str.map(|s| LocationId(uuid::Uuid::parse_str(&s).unwrap_or_default()));
    let kind: LocationKind = kind_str.parse().unwrap_or(LocationKind::Storage);

    Ok(Location {
        id,
        parent_id,
        name,
        kind,
        can_hold_stock,
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
    fn create_and_get_location() {
        let conn = setup();
        let repo = SqliteLocationRepo::new(&conn);

        let loc = repo
            .create(&NewLocation {
                parent_id: None,
                name: "Workshop".to_string(),
                kind: LocationKind::Room,
            })
            .unwrap();

        assert_eq!(loc.name, "Workshop");
        assert!(!loc.can_hold_stock); // rooms don't hold stock

        let found = repo.get_by_id(&loc.id).unwrap().unwrap();
        assert_eq!(found.name, "Workshop");
    }

    #[test]
    fn nested_locations() {
        let conn = setup();
        let repo = SqliteLocationRepo::new(&conn);

        let room = repo
            .create(&NewLocation {
                parent_id: None,
                name: "Workshop".to_string(),
                kind: LocationKind::Room,
            })
            .unwrap();

        let cabinet = repo
            .create(&NewLocation {
                parent_id: Some(room.id),
                name: "Tool Chest".to_string(),
                kind: LocationKind::Cabinet,
            })
            .unwrap();

        let drawer = repo
            .create(&NewLocation {
                parent_id: Some(cabinet.id),
                name: "Drawer 1".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap();

        assert!(drawer.can_hold_stock);

        let children = repo.list_children(Some(&room.id)).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Tool Chest");
    }

    #[test]
    fn get_path() {
        let conn = setup();
        let repo = SqliteLocationRepo::new(&conn);

        let room = repo
            .create(&NewLocation {
                parent_id: None,
                name: "Workshop".to_string(),
                kind: LocationKind::Room,
            })
            .unwrap();

        let drawer = repo
            .create(&NewLocation {
                parent_id: Some(room.id),
                name: "Drawer 1".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap();

        let path = repo.get_path(&drawer.id).unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].name, "Workshop");
        assert_eq!(path[1].name, "Drawer 1");
    }

    #[test]
    fn duplicate_sibling_rejected() {
        let conn = setup();
        let repo = SqliteLocationRepo::new(&conn);

        repo.create(&NewLocation {
            parent_id: None,
            name: "Workshop".to_string(),
            kind: LocationKind::Room,
        })
        .unwrap();

        let result = repo.create(&NewLocation {
            parent_id: None,
            name: "Workshop".to_string(),
            kind: LocationKind::Room,
        });
        assert!(result.is_err());
    }

    #[test]
    fn find_by_name() {
        let conn = setup();
        let repo = SqliteLocationRepo::new(&conn);

        repo.create(&NewLocation {
            parent_id: None,
            name: "Workshop".to_string(),
            kind: LocationKind::Room,
        })
        .unwrap();

        let found = repo.find_by_name("Workshop", None).unwrap();
        assert!(found.is_some());

        let missing = repo.find_by_name("Nonexistent", None).unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn invalid_parent_rejected() {
        let conn = setup();
        let repo = SqliteLocationRepo::new(&conn);

        let fake_id = LocationId::new();
        let result = repo.create(&NewLocation {
            parent_id: Some(fake_id),
            name: "Orphan".to_string(),
            kind: LocationKind::Drawer,
        });
        assert!(result.is_err());
    }
}
