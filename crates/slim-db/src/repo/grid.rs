//! SQLite repository implementation for Gridfinity baseplates, bins, and spatial placements.

use rusqlite::{Connection, Row, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use slim_core::error::{CoreError, CoreResult};
use slim_core::model::gridfinity::{
    ActivePlacementInfo, BinPlacement, GRIDFINITY_PITCH_MM, GridBase, GridBaseId, GridMap,
    GridfinityBin, GridfinityBinId, NewGridBase, NewGridfinityBin, PlaceBinCmd, PlacementId,
    Rotation, geometry,
};
use slim_core::model::location::LocationId;
use slim_core::service::GridRepository;

pub struct SqliteGridRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteGridRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

fn format_time(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

fn row_to_base(row: &Row) -> rusqlite::Result<GridBase> {
    let id_str: String = row.get(0)?;
    let loc_id_str: String = row.get(1)?;
    let code: String = row.get(2)?;
    let cols: u32 = row.get(3)?;
    let rows: u32 = row.get(4)?;
    let pitch_mm: f64 = row.get(5)?;
    let created_at_str: String = row.get(6)?;

    let id = GridBaseId(Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let location_id =
        LocationId(Uuid::parse_str(&loc_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let created_at = OffsetDateTime::parse(&created_at_str, &Rfc3339)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;

    Ok(GridBase {
        id,
        location_id,
        code,
        cols,
        rows,
        pitch_mm,
        created_at,
    })
}

fn row_to_bin(row: &Row) -> rusqlite::Result<GridfinityBin> {
    let id_str: String = row.get(0)?;
    let loc_id_str: String = row.get(1)?;
    let bin_code: String = row.get(2)?;
    let width_units: u32 = row.get(3)?;
    let depth_units: u32 = row.get(4)?;
    let height_units: u32 = row.get(5)?;
    let compartment_count: u32 = row.get(6)?;
    let created_at_str: String = row.get(7)?;

    let id = GridfinityBinId(Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let location_id =
        LocationId(Uuid::parse_str(&loc_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let created_at = OffsetDateTime::parse(&created_at_str, &Rfc3339)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;

    Ok(GridfinityBin {
        id,
        location_id,
        bin_code,
        width_units,
        depth_units,
        height_units,
        compartment_count,
        created_at,
    })
}

fn row_to_placement(row: &Row) -> rusqlite::Result<BinPlacement> {
    let id_str: String = row.get(0)?;
    let bin_id_str: String = row.get(1)?;
    let base_id_str: String = row.get(2)?;
    let x: u32 = row.get(3)?;
    let y: u32 = row.get(4)?;
    let rot_val: u32 = row.get(5)?;
    let placed_at_str: String = row.get(6)?;
    let removed_at_str: Option<String> = row.get(7)?;

    let id = PlacementId(Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let bin_id =
        GridfinityBinId(Uuid::parse_str(&bin_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let base_id =
        GridBaseId(Uuid::parse_str(&base_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?);
    let rotation = if rot_val == 90 {
        Rotation::Rot90
    } else {
        Rotation::Rot0
    };
    let placed_at = OffsetDateTime::parse(&placed_at_str, &Rfc3339)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let removed_at = removed_at_str.and_then(|s| OffsetDateTime::parse(&s, &Rfc3339).ok());

    Ok(BinPlacement {
        id,
        bin_id,
        base_id,
        x,
        y,
        rotation,
        placed_at,
        removed_at,
    })
}

impl GridRepository for SqliteGridRepo<'_> {
    fn create_base(&self, base: &NewGridBase) -> CoreResult<GridBase> {
        base.validate()?;

        let id = GridBaseId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);
        let pitch = base.pitch_mm.unwrap_or(GRIDFINITY_PITCH_MM);

        self.conn
            .execute(
                "INSERT INTO grid_bases (id, location_id, code, cols, rows, pitch_mm, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.0.to_string(),
                    base.location_id.0.to_string(),
                    base.code.trim(),
                    base.cols,
                    base.rows,
                    pitch,
                    now_str,
                ],
            )
            .map_err(|e| {
                if let rusqlite::Error::SqliteFailure(ref f, _) = e
                    && f.code == rusqlite::ErrorCode::ConstraintViolation
                {
                    return CoreError::InvalidInput(format!(
                        "baseplate code '{}' already exists",
                        base.code
                    ));
                }
                CoreError::Storage(e.to_string())
            })?;

        Ok(GridBase {
            id,
            location_id: base.location_id,
            code: base.code.trim().to_string(),
            cols: base.cols,
            rows: base.rows,
            pitch_mm: pitch,
            created_at: now,
        })
    }

    fn get_base_by_id(&self, id: &GridBaseId) -> CoreResult<Option<GridBase>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, location_id, code, cols, rows, pitch_mm, created_at
                 FROM grid_bases WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(params![id.0.to_string()], row_to_base);
        match res {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn get_base_by_code(&self, code: &str) -> CoreResult<Option<GridBase>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, location_id, code, cols, rows, pitch_mm, created_at
                 FROM grid_bases WHERE code = ?1 COLLATE NOCASE AND archived_at IS NULL",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(params![code.trim()], row_to_base);
        match res {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn list_bases(&self, location_id: Option<&LocationId>) -> CoreResult<Vec<GridBase>> {
        let mut bases = Vec::new();

        if let Some(loc_id) = location_id {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, location_id, code, cols, rows, pitch_mm, created_at
                     FROM grid_bases WHERE location_id = ?1 AND archived_at IS NULL ORDER BY code ASC",
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            let rows = stmt
                .query_map(params![loc_id.0.to_string()], row_to_base)
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            for r in rows {
                bases.push(r.map_err(|e| CoreError::Storage(e.to_string()))?);
            }
        } else {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, location_id, code, cols, rows, pitch_mm, created_at
                     FROM grid_bases WHERE archived_at IS NULL ORDER BY code ASC",
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            let rows = stmt
                .query_map([], row_to_base)
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            for r in rows {
                bases.push(r.map_err(|e| CoreError::Storage(e.to_string()))?);
            }
        }

        Ok(bases)
    }

    fn create_bin(
        &self,
        bin: &NewGridfinityBin,
        location_id: &LocationId,
    ) -> CoreResult<GridfinityBin> {
        bin.validate()?;

        let id = GridfinityBinId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        self.conn
            .execute(
                "INSERT INTO gridfinity_bins (id, location_id, bin_code, width_units, depth_units, height_units, compartment_count, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id.0.to_string(),
                    location_id.0.to_string(),
                    bin.bin_code.trim(),
                    bin.width_units,
                    bin.depth_units,
                    bin.height_units,
                    bin.compartment_count,
                    now_str,
                ],
            )
            .map_err(|e| {
                if let rusqlite::Error::SqliteFailure(ref f, _) = e
                    && f.code == rusqlite::ErrorCode::ConstraintViolation {
                        return CoreError::InvalidInput(format!("bin code '{}' already exists", bin.bin_code));
                    }
                CoreError::Storage(e.to_string())
            })?;

        Ok(GridfinityBin {
            id,
            location_id: *location_id,
            bin_code: bin.bin_code.trim().to_string(),
            width_units: bin.width_units,
            depth_units: bin.depth_units,
            height_units: bin.height_units,
            compartment_count: bin.compartment_count,
            created_at: now,
        })
    }

    fn get_bin_by_id(&self, id: &GridfinityBinId) -> CoreResult<Option<GridfinityBin>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, location_id, bin_code, width_units, depth_units, height_units, compartment_count, created_at
                 FROM gridfinity_bins WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(params![id.0.to_string()], row_to_bin);
        match res {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn get_bin_by_code(&self, code: &str) -> CoreResult<Option<GridfinityBin>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, location_id, bin_code, width_units, depth_units, height_units, compartment_count, created_at
                 FROM gridfinity_bins WHERE bin_code = ?1 COLLATE NOCASE AND archived_at IS NULL",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(params![code.trim()], row_to_bin);
        match res {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn get_bin_by_location_id(
        &self,
        location_id: &LocationId,
    ) -> CoreResult<Option<GridfinityBin>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, location_id, bin_code, width_units, depth_units, height_units, compartment_count, created_at
                 FROM gridfinity_bins WHERE location_id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(params![location_id.0.to_string()], row_to_bin);
        match res {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn list_bins(&self) -> CoreResult<Vec<GridfinityBin>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, location_id, bin_code, width_units, depth_units, height_units, compartment_count, created_at
                 FROM gridfinity_bins WHERE archived_at IS NULL ORDER BY bin_code ASC",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_bin)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut bins = Vec::new();
        for r in rows {
            bins.push(r.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(bins)
    }

    fn place_bin(&self, cmd: &PlaceBinCmd) -> CoreResult<BinPlacement> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        let base = self
            .get_base_by_id(&cmd.base_id)?
            .ok_or_else(|| CoreError::NotFound {
                entity: "grid_base",
                id: cmd.base_id.to_string(),
            })?;

        let bin = self
            .get_bin_by_id(&cmd.bin_id)?
            .ok_or_else(|| CoreError::NotFound {
                entity: "gridfinity_bin",
                id: cmd.bin_id.to_string(),
            })?;

        // 1. Fetch current active placements on this base
        let active_placements = self.get_active_placements(&cmd.base_id)?;

        // 2. Run spatial geometry bounds and collision checks
        geometry::validate_placement(&base, &bin, cmd, &active_placements)?;

        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        // 3. Deactivate any previous active placement for this bin
        tx.execute(
            "UPDATE bin_placements SET removed_at = ?1 WHERE bin_id = ?2 AND removed_at IS NULL",
            params![now_str, bin.id.0.to_string()],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 4. Insert new placement
        let placement_id = PlacementId::new();
        tx.execute(
            "INSERT INTO bin_placements (id, bin_id, base_id, x, y, rotation, placed_at, removed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                placement_id.0.to_string(),
                bin.id.0.to_string(),
                base.id.0.to_string(),
                cmd.x,
                cmd.y,
                cmd.rotation as u32,
                now_str,
            ],
        )
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        tx.commit().map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(BinPlacement {
            id: placement_id,
            bin_id: bin.id,
            base_id: base.id,
            x: cmd.x,
            y: cmd.y,
            rotation: cmd.rotation,
            placed_at: now,
            removed_at: None,
        })
    }

    fn remove_bin(&self, bin_id: &GridfinityBinId) -> CoreResult<()> {
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        let rows_affected = self
            .conn
            .execute(
                "UPDATE bin_placements SET removed_at = ?1 WHERE bin_id = ?2 AND removed_at IS NULL",
                params![now_str, bin_id.0.to_string()],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows_affected == 0 {
            return Err(CoreError::NotFound {
                entity: "active_bin_placement",
                id: bin_id.to_string(),
            });
        }

        Ok(())
    }

    fn get_active_placements(&self, base_id: &GridBaseId) -> CoreResult<Vec<ActivePlacementInfo>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.bin_id, p.base_id, p.x, p.y, p.rotation, p.placed_at, p.removed_at,
                        b.bin_code, b.width_units, b.depth_units, b.height_units
                 FROM bin_placements p
                 JOIN gridfinity_bins b ON p.bin_id = b.id
                 WHERE p.base_id = ?1 AND p.removed_at IS NULL
                 ORDER BY p.y ASC, p.x ASC",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(params![base_id.0.to_string()], |row| {
                let placement = row_to_placement(row)?;
                let bin_code: String = row.get(8)?;
                let width_units: u32 = row.get(9)?;
                let depth_units: u32 = row.get(10)?;
                let height_units: u32 = row.get(11)?;

                Ok(ActivePlacementInfo {
                    placement,
                    bin_code,
                    width_units,
                    depth_units,
                    height_units,
                })
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(results)
    }

    fn get_active_placement_for_bin(
        &self,
        bin_id: &GridfinityBinId,
    ) -> CoreResult<Option<ActivePlacementInfo>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.bin_id, p.base_id, p.x, p.y, p.rotation, p.placed_at, p.removed_at,
                        b.bin_code, b.width_units, b.depth_units, b.height_units
                 FROM bin_placements p
                 JOIN gridfinity_bins b ON p.bin_id = b.id
                 WHERE p.bin_id = ?1 AND p.removed_at IS NULL",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let res = stmt.query_row(params![bin_id.0.to_string()], |row| {
            let placement = row_to_placement(row)?;
            let bin_code: String = row.get(8)?;
            let width_units: u32 = row.get(9)?;
            let depth_units: u32 = row.get(10)?;
            let height_units: u32 = row.get(11)?;

            Ok(ActivePlacementInfo {
                placement,
                bin_code,
                width_units,
                depth_units,
                height_units,
            })
        });

        match res {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage(e.to_string())),
        }
    }

    fn render_grid_map(&self, base_id: &GridBaseId) -> CoreResult<GridMap> {
        let base = self
            .get_base_by_id(base_id)?
            .ok_or_else(|| CoreError::NotFound {
                entity: "grid_base",
                id: base_id.to_string(),
            })?;

        let placements = self.get_active_placements(base_id)?;
        Ok(GridMap::new(base, placements))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;
    use crate::repo::location::SqliteLocationRepo;
    use slim_core::model::location::{LocationKind, NewLocation};
    use slim_core::service::LocationRepository;

    fn setup_db() -> Connection {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn create_test_location(conn: &Connection) -> slim_core::model::location::Location {
        let loc_repo = SqliteLocationRepo::new(conn);
        loc_repo
            .create(&NewLocation {
                parent_id: None,
                name: "Electronics Drawer".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap()
    }

    fn create_bin_location(conn: &Connection, name: &str) -> slim_core::model::location::Location {
        let loc_repo = SqliteLocationRepo::new(conn);
        loc_repo
            .create(&NewLocation {
                parent_id: None,
                name: name.to_string(),
                kind: LocationKind::Bin,
            })
            .unwrap()
    }

    #[test]
    fn test_create_and_get_grid_base() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-A".to_string(),
                cols: 6,
                rows: 4,
                pitch_mm: None,
            })
            .unwrap();

        assert_eq!(base.code, "BASE-A");
        assert_eq!(base.cols, 6);
        assert_eq!(base.rows, 4);
        assert_eq!(base.pitch_mm, GRIDFINITY_PITCH_MM);

        let fetched = repo.get_base_by_id(&base.id).unwrap().unwrap();
        assert_eq!(fetched.code, "BASE-A");

        let fetched_code = repo.get_base_by_code("base-a").unwrap().unwrap();
        assert_eq!(fetched_code.id, base.id);

        // Duplicate code rejected
        let dup = repo.create_base(&NewGridBase {
            location_id: loc.id,
            code: "BASE-A".to_string(),
            cols: 2,
            rows: 2,
            pitch_mm: None,
        });
        assert!(dup.is_err());
    }

    #[test]
    fn test_create_and_get_grid_bin() {
        let conn = setup_db();
        let bin_loc = create_bin_location(&conn, "Bin 001");
        let repo = SqliteGridRepo::new(&conn);

        let bin = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 2,
                    depth_units: 1,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin_loc.id,
            )
            .unwrap();

        assert_eq!(bin.bin_code, "B-001");
        assert_eq!(bin.width_units, 2);
        assert_eq!(bin.depth_units, 1);
        assert_eq!(bin.height_units, 3);

        let fetched = repo.get_bin_by_code("b-001").unwrap().unwrap();
        assert_eq!(fetched.id, bin.id);
    }

    #[test]
    fn test_place_bin_valid_fit() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let bin_loc = create_bin_location(&conn, "Bin 001");
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-A".to_string(),
                cols: 6,
                rows: 4,
                pitch_mm: None,
            })
            .unwrap();

        let bin = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 2,
                    depth_units: 2,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin_loc.id,
            )
            .unwrap();

        let placement = repo
            .place_bin(&PlaceBinCmd {
                bin_id: bin.id,
                base_id: base.id,
                x: 1,
                y: 1,
                rotation: Rotation::Rot0,
            })
            .unwrap();

        assert_eq!(placement.x, 1);
        assert_eq!(placement.y, 1);
        assert!(placement.is_active());

        let active = repo.get_active_placements(&base.id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].bin_code, "B-001");
        assert!(active[0].covers(1, 1));
        assert!(active[0].covers(2, 2));
        assert!(!active[0].covers(3, 3));
    }

    #[test]
    fn test_place_bin_rotation_swaps_dimensions() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let bin_loc = create_bin_location(&conn, "Bin 001");
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-A".to_string(),
                cols: 6,
                rows: 4,
                pitch_mm: None,
            })
            .unwrap();

        // 3x1 bin
        let bin = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 3,
                    depth_units: 1,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin_loc.id,
            )
            .unwrap();

        // Rotated 90: effective footprint is 1x3
        let _placement = repo
            .place_bin(&PlaceBinCmd {
                bin_id: bin.id,
                base_id: base.id,
                x: 0,
                y: 0,
                rotation: Rotation::Rot90,
            })
            .unwrap();

        let active = repo.get_active_placements(&base.id).unwrap();
        assert_eq!(active[0].effective_dimensions(), (1, 3));
        assert!(active[0].covers(0, 0));
        assert!(active[0].covers(0, 1));
        assert!(active[0].covers(0, 2));
        assert!(!active[0].covers(1, 0));
    }

    #[test]
    fn test_place_bin_rejects_out_of_bounds() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let bin_loc = create_bin_location(&conn, "Bin 001");
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-A".to_string(),
                cols: 4,
                rows: 4,
                pitch_mm: None,
            })
            .unwrap();

        let bin = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 2,
                    depth_units: 2,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin_loc.id,
            )
            .unwrap();

        // X=3 with width 2 -> X=3..4 on 4-col base -> exceeds!
        let res_x = repo.place_bin(&PlaceBinCmd {
            bin_id: bin.id,
            base_id: base.id,
            x: 3,
            y: 0,
            rotation: Rotation::Rot0,
        });
        assert!(res_x.is_err());

        // Y=3 with depth 2 -> Y=3..4 on 4-row base -> exceeds!
        let res_y = repo.place_bin(&PlaceBinCmd {
            bin_id: bin.id,
            base_id: base.id,
            x: 0,
            y: 3,
            rotation: Rotation::Rot0,
        });
        assert!(res_y.is_err());
    }

    #[test]
    fn test_place_bin_rejects_spatial_collision() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let bin1_loc = create_bin_location(&conn, "Bin 001");
        let bin2_loc = create_bin_location(&conn, "Bin 002");
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-A".to_string(),
                cols: 6,
                rows: 6,
                pitch_mm: None,
            })
            .unwrap();

        let bin1 = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 2,
                    depth_units: 2,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin1_loc.id,
            )
            .unwrap();

        let bin2 = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-002".to_string(),
                    width_units: 2,
                    depth_units: 2,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin2_loc.id,
            )
            .unwrap();

        // Place bin1 at (0, 0) -> occupies (0,0), (1,0), (0,1), (1,1)
        repo.place_bin(&PlaceBinCmd {
            bin_id: bin1.id,
            base_id: base.id,
            x: 0,
            y: 0,
            rotation: Rotation::Rot0,
        })
        .unwrap();

        // Attempt to place bin2 at (1, 1) -> overlaps at (1, 1)!
        let err = repo.place_bin(&PlaceBinCmd {
            bin_id: bin2.id,
            base_id: base.id,
            x: 1,
            y: 1,
            rotation: Rotation::Rot0,
        });
        assert!(err.is_err());
        let err_msg = err.unwrap_err().to_string();
        assert!(err_msg.contains("collision"));

        // Place bin2 at (2, 0) -> adjacent, no collision!
        let ok = repo.place_bin(&PlaceBinCmd {
            bin_id: bin2.id,
            base_id: base.id,
            x: 2,
            y: 0,
            rotation: Rotation::Rot0,
        });
        assert!(ok.is_ok());
    }

    #[test]
    fn test_move_bin_updates_placement() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let bin_loc = create_bin_location(&conn, "Bin 001");
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-A".to_string(),
                cols: 6,
                rows: 6,
                pitch_mm: None,
            })
            .unwrap();

        let bin = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 1,
                    depth_units: 1,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin_loc.id,
            )
            .unwrap();

        // Place at (0, 0)
        repo.place_bin(&PlaceBinCmd {
            bin_id: bin.id,
            base_id: base.id,
            x: 0,
            y: 0,
            rotation: Rotation::Rot0,
        })
        .unwrap();

        // Move to (3, 3)
        repo.place_bin(&PlaceBinCmd {
            bin_id: bin.id,
            base_id: base.id,
            x: 3,
            y: 3,
            rotation: Rotation::Rot0,
        })
        .unwrap();

        let active = repo.get_active_placements(&base.id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].placement.x, 3);
        assert_eq!(active[0].placement.y, 3);

        // Remove bin
        repo.remove_bin(&bin.id).unwrap();
        let after_remove = repo.get_active_placements(&base.id).unwrap();
        assert_eq!(after_remove.len(), 0);
    }

    #[test]
    fn test_render_grid_map_ascii() {
        let conn = setup_db();
        let loc = create_test_location(&conn);
        let bin_loc = create_bin_location(&conn, "Bin 001");
        let repo = SqliteGridRepo::new(&conn);

        let base = repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "DRAWER-BASE".to_string(),
                cols: 4,
                rows: 3,
                pitch_mm: None,
            })
            .unwrap();

        let bin = repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "B-001".to_string(),
                    width_units: 2,
                    depth_units: 1,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &bin_loc.id,
            )
            .unwrap();

        repo.place_bin(&PlaceBinCmd {
            bin_id: bin.id,
            base_id: base.id,
            x: 0,
            y: 0,
            rotation: Rotation::Rot0,
        })
        .unwrap();

        let grid_map = repo.render_grid_map(&base.id).unwrap();
        let ascii = grid_map.render_ascii();
        assert!(ascii.contains("DRAWER-BASE"));
        assert!(ascii.contains("4x3 cells"));
        assert!(ascii.contains("X0"));
        assert!(ascii.contains("Y0"));
        assert!(ascii.contains("[ . ]"));
        assert!(ascii.contains("B-0"));
    }
}
