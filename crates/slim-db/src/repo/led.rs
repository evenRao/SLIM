//! SQLite implementation of LedRepository and dock LED priority arbitration.

use rusqlite::{Connection, params};
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::gridfinity::{GridBaseId, GridfinityBinId, Rotation};
use slim_core::model::led::*;
use slim_core::service::LedRepository;
use time::OffsetDateTime;
use uuid::Uuid;

use super::actor::{OptionalExt, format_time, parse_time};

/// SQLite-backed LED and dock presence repository.
pub struct SqliteLedRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteLedRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// High-level find & locate: routes an item SKU or bin code to its physical
    /// baseplate coordinates and applies a time-limited `Locate` pulse across all
    /// docks in the bin's footprint.
    pub fn locate_target(
        &self,
        target: &str,
        ttl_secs: Option<u64>,
        now: OffsetDateTime,
    ) -> CoreResult<Vec<DockLedState>> {
        let ttl = ttl_secs.unwrap_or(DEFAULT_LOCATE_TTL_SECS);
        let expires_at = now + time::Duration::seconds(ttl as i64);
        let expires_str = format_time(expires_at);
        let now_str = format_time(now);

        // 1. Try finding by bin_code first
        let mut bin_info: Option<(GridfinityBinId, String, u32, u32)> = self
            .conn
            .query_row(
                "SELECT id, bin_code, width_units, depth_units
                 FROM gridfinity_bins WHERE bin_code = ?1",
                params![target],
                |row| {
                    let id_str: String = row.get(0)?;
                    let code: String = row.get(1)?;
                    let w: u32 = row.get(2)?;
                    let d: u32 = row.get(3)?;
                    let id = GridfinityBinId(Uuid::parse_str(&id_str).unwrap_or_default());
                    Ok((id, code, w, d))
                },
            )
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. If not a bin code, try looking up as an item SKU or Name to find its stocked bin
        if bin_info.is_none() {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT b.id, b.bin_code, b.width_units, b.depth_units
                     FROM items i
                     JOIN stock_balances sb ON i.id = sb.item_id AND sb.qty > 0
                     JOIN gridfinity_bins b ON sb.location_id = b.location_id
                     WHERE (i.sku = ?1 OR i.name = ?1) AND i.archived_at IS NULL
                     LIMIT 1",
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            bin_info = stmt
                .query_row(params![target], |row| {
                    let id_str: String = row.get(0)?;
                    let code: String = row.get(1)?;
                    let w: u32 = row.get(2)?;
                    let d: u32 = row.get(3)?;
                    let id = GridfinityBinId(Uuid::parse_str(&id_str).unwrap_or_default());
                    Ok((id, code, w, d))
                })
                .optional()
                .map_err(|e| CoreError::Storage(e.to_string()))?;
        }

        // 3. Fallback: check if the item is an alias or scanned identifier mapped to item or bin
        if bin_info.is_none() {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT b.id, b.bin_code, b.width_units, b.depth_units
                     FROM identifiers idn
                     JOIN gridfinity_bins b ON idn.bin_id = b.id
                     WHERE idn.value = ?1
                     LIMIT 1",
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            bin_info = stmt
                .query_row(params![target], |row| {
                    let id_str: String = row.get(0)?;
                    let code: String = row.get(1)?;
                    let w: u32 = row.get(2)?;
                    let d: u32 = row.get(3)?;
                    let id = GridfinityBinId(Uuid::parse_str(&id_str).unwrap_or_default());
                    Ok((id, code, w, d))
                })
                .optional()
                .map_err(|e| CoreError::Storage(e.to_string()))?;
        }

        let (bin_id, bin_code, width_units, depth_units) = match bin_info {
            Some(info) => info,
            None => {
                return Err(CoreError::NotFound {
                    entity: "placed bin or stocked item",
                    id: target.to_string(),
                });
            }
        };

        // 4. Find the active placement of this bin on a baseplate
        let placement_info: Option<(String, String, u32, u32, i64)> = self
            .conn
            .query_row(
                "SELECT bp.base_id, gb.code, bp.x, bp.y, bp.rotation
                 FROM bin_placements bp
                 JOIN grid_bases gb ON bp.base_id = gb.id
                 WHERE bp.bin_id = ?1 AND bp.removed_at IS NULL",
                params![bin_id.0.to_string()],
                |row| {
                    let base_id: String = row.get(0)?;
                    let base_code: String = row.get(1)?;
                    let x: u32 = row.get(2)?;
                    let y: u32 = row.get(3)?;
                    let rot: i64 = row.get(4)?;
                    Ok((base_id, base_code, x, y, rot))
                },
            )
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let (base_id_str, base_code, anchor_x, anchor_y, rot_int) = match placement_info {
            Some(p) => p,
            None => {
                return Err(CoreError::InvalidInput(format!(
                    "bin '{}' is not actively placed on any baseplate",
                    bin_code
                )));
            }
        };

        let base_id = GridBaseId(Uuid::parse_str(&base_id_str).unwrap_or_default());
        let rotation = if rot_int == 90 {
            Rotation::Rot90
        } else {
            Rotation::Rot0
        };

        // 5. Calculate all docks covered by this bin's footprint
        let covered = calculate_covered_docks(
            &base_code,
            anchor_x,
            anchor_y,
            width_units,
            depth_units,
            rotation,
        );

        let mut updated_states = Vec::new();
        let reason = format!("Locate bin '{bin_code}' (TTL: {ttl}s)");

        for (dock_id, x, y) in covered {
            self.conn
                .execute(
                    "INSERT INTO dock_led_states (
                        dock_id, base_id, x, y, mode, r, g, b, expires_at, reason, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, 'locate', ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(dock_id) DO UPDATE SET
                        base_id = ?2, x = ?3, y = ?4, mode = 'locate',
                        r = ?5, g = ?6, b = ?7, expires_at = ?8, reason = ?9, updated_at = ?10",
                    params![
                        dock_id,
                        base_id.0.to_string(),
                        x,
                        y,
                        COLOR_LOCATE[0],
                        COLOR_LOCATE[1],
                        COLOR_LOCATE[2],
                        expires_str,
                        reason,
                        now_str,
                    ],
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            updated_states.push(DockLedState {
                dock_id,
                base_id,
                x,
                y,
                mode: LedMode::Locate,
                r: COLOR_LOCATE[0],
                g: COLOR_LOCATE[1],
                b: COLOR_LOCATE[2],
                expires_at: Some(expires_at),
                reason: Some(reason.clone()),
                updated_at: now,
            });
        }

        Ok(updated_states)
    }

    /// Evaluate the priority state machine for an entire baseplate, updating all
    /// docks and resolving conflicts according to Section 9.3:
    /// Priority 1 (Missing) > Priority 2 (Locate) > Priority 3 (LowStock) > Priority 4 (Off).
    pub fn evaluate_base_leds(
        &self,
        base_id: &GridBaseId,
        now: OffsetDateTime,
    ) -> CoreResult<Vec<DockLedState>> {
        // Clear expired states first
        self.clear_expired_leds(now)?;

        let base_code: String = self
            .conn
            .query_row(
                "SELECT code FROM grid_bases WHERE id = ?1",
                params![base_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 1. Get all active bin placements on this base
        let mut stmt = self
            .conn
            .prepare(
                "SELECT bp.bin_id, b.bin_code, b.location_id, b.width_units, b.depth_units,
                        bp.x, bp.y, bp.rotation
                 FROM bin_placements bp
                 JOIN gridfinity_bins b ON bp.bin_id = b.id
                 WHERE bp.base_id = ?1 AND bp.removed_at IS NULL",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        struct ActivePlacement {
            _bin_id: String,
            _bin_code: String,
            location_id: String,
            width_units: u32,
            depth_units: u32,
            x: u32,
            y: u32,
            rotation: Rotation,
        }

        let placements = stmt
            .query_map(params![base_id.0.to_string()], |row| {
                let bin_id: String = row.get(0)?;
                let bin_code: String = row.get(1)?;
                let location_id: String = row.get(2)?;
                let width_units: u32 = row.get(3)?;
                let depth_units: u32 = row.get(4)?;
                let x: u32 = row.get(5)?;
                let y: u32 = row.get(6)?;
                let rot_int: i64 = row.get(7)?;
                let rotation = if rot_int == 90 {
                    Rotation::Rot90
                } else {
                    Rotation::Rot0
                };

                Ok(ActivePlacement {
                    _bin_id: bin_id,
                    _bin_code: bin_code,
                    location_id,
                    width_units,
                    depth_units,
                    x,
                    y,
                    rotation,
                })
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        // 2. Fetch presence states for base
        let presences = self.list_dock_presence(base_id)?;

        // 3. Check low stock locations
        use slim_core::service::InventoryService;
        let low_stock_locs: std::collections::HashSet<String> =
            super::inventory::SqliteInventoryService::new(self.conn)
                .get_low_stock()?
                .into_iter()
                .map(|alert| alert.location_id.to_string())
                .collect();

        let mut evaluated_states = Vec::new();
        let now_str = format_time(now);

        for p in placements.flatten() {
            let is_low_stock = low_stock_locs.contains(&p.location_id);
            let covered = calculate_covered_docks(
                &base_code,
                p.x,
                p.y,
                p.width_units,
                p.depth_units,
                p.rotation,
            );

            // Check if any covered dock is missing
            let is_missing = covered.iter().any(|(did, _, _)| {
                presences
                    .iter()
                    .any(|pr| pr.dock_id == *did && (!pr.occupied || pr.status == "missing"))
            });

            for (dock_id, x, y) in covered {
                // Check if existing dock has unexpired Locate
                let existing_led = self.get_dock_led(&dock_id)?;
                let locate_expires = existing_led
                    .filter(|l| l.mode == LedMode::Locate)
                    .and_then(|l| l.expires_at)
                    .filter(|exp| *exp > now);

                let (mode, rgb, expires_at, reason) =
                    resolve_dock_led_priority(is_missing, locate_expires, is_low_stock, now);

                let state = DockLedState {
                    dock_id: dock_id.clone(),
                    base_id: *base_id,
                    x,
                    y,
                    mode,
                    r: rgb[0],
                    g: rgb[1],
                    b: rgb[2],
                    expires_at,
                    reason,
                    updated_at: now,
                };

                self.conn
                    .execute(
                        "INSERT INTO dock_led_states (
                            dock_id, base_id, x, y, mode, r, g, b, expires_at, reason, updated_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                         ON CONFLICT(dock_id) DO UPDATE SET
                            base_id = ?2, x = ?3, y = ?4, mode = ?5,
                            r = ?6, g = ?7, b = ?8, expires_at = ?9, reason = ?10, updated_at = ?11",
                        params![
                            state.dock_id,
                            state.base_id.0.to_string(),
                            state.x,
                            state.y,
                            state.mode.to_string(),
                            state.r,
                            state.g,
                            state.b,
                            state.expires_at.map(format_time),
                            state.reason,
                            now_str,
                        ],
                    )
                    .map_err(|e| CoreError::Storage(e.to_string()))?;

                evaluated_states.push(state);
            }
        }

        Ok(evaluated_states)
    }
}

fn row_to_dock_led(row: &rusqlite::Row<'_>) -> rusqlite::Result<DockLedState> {
    let dock_id: String = row.get(0)?;
    let base_id_str: String = row.get(1)?;
    let x: u32 = row.get(2)?;
    let y: u32 = row.get(3)?;
    let mode_str: String = row.get(4)?;
    let r: u8 = row.get(5)?;
    let g: u8 = row.get(6)?;
    let b: u8 = row.get(7)?;
    let expires_str: Option<String> = row.get(8)?;
    let reason: Option<String> = row.get(9)?;
    let updated_str: String = row.get(10)?;

    let base_id = GridBaseId(Uuid::parse_str(&base_id_str).unwrap_or_default());
    let mode: LedMode = mode_str.parse().unwrap_or(LedMode::Off);
    let expires_at = expires_str.as_deref().map(parse_time);
    let updated_at = parse_time(&updated_str);

    Ok(DockLedState {
        dock_id,
        base_id,
        x,
        y,
        mode,
        r,
        g,
        b,
        expires_at,
        reason,
        updated_at,
    })
}

fn row_to_dock_presence(row: &rusqlite::Row<'_>) -> rusqlite::Result<DockPresenceState> {
    let dock_id: String = row.get(0)?;
    let base_id_str: String = row.get(1)?;
    let x: u32 = row.get(2)?;
    let y: u32 = row.get(3)?;
    let occupied_int: i64 = row.get(4)?;
    let last_seen_str: String = row.get(5)?;
    let status: String = row.get(6)?;

    let base_id = GridBaseId(Uuid::parse_str(&base_id_str).unwrap_or_default());
    let last_seen_at = parse_time(&last_seen_str);

    Ok(DockPresenceState {
        dock_id,
        base_id,
        x,
        y,
        occupied: occupied_int != 0,
        last_seen_at,
        status,
    })
}

impl LedRepository for SqliteLedRepo<'_> {
    fn set_dock_led(&self, state: &DockLedState) -> CoreResult<()> {
        let now = format_time(state.updated_at);
        let exp = state.expires_at.map(format_time);

        self.conn
            .execute(
                "INSERT INTO dock_led_states (
                    dock_id, base_id, x, y, mode, r, g, b, expires_at, reason, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(dock_id) DO UPDATE SET
                    base_id = ?2, x = ?3, y = ?4, mode = ?5,
                    r = ?6, g = ?7, b = ?8, expires_at = ?9, reason = ?10, updated_at = ?11",
                params![
                    state.dock_id,
                    state.base_id.0.to_string(),
                    state.x,
                    state.y,
                    state.mode.to_string(),
                    state.r,
                    state.g,
                    state.b,
                    exp,
                    state.reason,
                    now,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(())
    }

    fn get_dock_led(&self, dock_id: &str) -> CoreResult<Option<DockLedState>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT dock_id, base_id, x, y, mode, r, g, b, expires_at, reason, updated_at
                 FROM dock_led_states WHERE dock_id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![dock_id], row_to_dock_led)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn list_dock_leds(&self, base_id: Option<&GridBaseId>) -> CoreResult<Vec<DockLedState>> {
        let (query, params_vec): (String, Vec<rusqlite::types::Value>) = match base_id {
            Some(bid) => (
                "SELECT dock_id, base_id, x, y, mode, r, g, b, expires_at, reason, updated_at
                 FROM dock_led_states WHERE base_id = ?1 ORDER BY y ASC, x ASC"
                    .to_string(),
                vec![rusqlite::types::Value::Text(bid.0.to_string())],
            ),
            None => (
                "SELECT dock_id, base_id, x, y, mode, r, g, b, expires_at, reason, updated_at
                 FROM dock_led_states ORDER BY base_id, y ASC, x ASC"
                    .to_string(),
                Vec::new(),
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_vec), row_to_dock_led)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut states = Vec::new();
        for r in rows {
            states.push(r.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(states)
    }

    fn clear_dock_led(&self, dock_id: &str) -> CoreResult<()> {
        self.conn
            .execute(
                "DELETE FROM dock_led_states WHERE dock_id = ?1",
                params![dock_id],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn clear_expired_leds(&self, now: OffsetDateTime) -> CoreResult<usize> {
        let now_str = format_time(now);
        let count = self
            .conn
            .execute(
                "DELETE FROM dock_led_states WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                params![now_str],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(count)
    }

    fn set_dock_presence(&self, presence: &DockPresenceState) -> CoreResult<()> {
        let now_str = format_time(presence.last_seen_at);

        self.conn
            .execute(
                "INSERT INTO dock_presence_states (
                    dock_id, base_id, x, y, occupied, last_seen_at, status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(dock_id) DO UPDATE SET
                    base_id = ?2, x = ?3, y = ?4, occupied = ?5, last_seen_at = ?6, status = ?7",
                params![
                    presence.dock_id,
                    presence.base_id.0.to_string(),
                    presence.x,
                    presence.y,
                    if presence.occupied { 1 } else { 0 },
                    now_str,
                    presence.status,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(())
    }

    fn get_dock_presence(&self, dock_id: &str) -> CoreResult<Option<DockPresenceState>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT dock_id, base_id, x, y, occupied, last_seen_at, status
                 FROM dock_presence_states WHERE dock_id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![dock_id], row_to_dock_presence)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn list_dock_presence(&self, base_id: &GridBaseId) -> CoreResult<Vec<DockPresenceState>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT dock_id, base_id, x, y, occupied, last_seen_at, status
                 FROM dock_presence_states WHERE base_id = ?1 ORDER BY y ASC, x ASC",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(params![base_id.0.to_string()], row_to_dock_presence)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut states = Vec::new();
        for r in rows {
            states.push(r.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::run_migrations;
    use crate::repo::grid::SqliteGridRepo;
    use crate::repo::location::SqliteLocationRepo;
    use slim_core::model::gridfinity::{NewGridBase, NewGridfinityBin};
    use slim_core::model::location::{LocationKind, NewLocation};
    use slim_core::service::{GridRepository, LocationRepository};

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn test_led_crud_and_expiration() {
        let conn = setup_db();
        let loc_repo = SqliteLocationRepo::new(&conn);
        let grid_repo = SqliteGridRepo::new(&conn);
        let led_repo = SqliteLedRepo::new(&conn);

        let loc = loc_repo
            .create(&NewLocation {
                parent_id: None,
                name: "Drawer 1".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap();

        let base = grid_repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-1".to_string(),
                cols: 4,
                rows: 4,
                pitch_mm: None,
            })
            .unwrap();

        let now = OffsetDateTime::now_utc();
        let exp = now + time::Duration::seconds(10);

        let state = DockLedState {
            dock_id: "BASE-1:2:2".to_string(),
            base_id: base.id,
            x: 2,
            y: 2,
            mode: LedMode::Locate,
            r: COLOR_LOCATE[0],
            g: COLOR_LOCATE[1],
            b: COLOR_LOCATE[2],
            expires_at: Some(exp),
            reason: Some("Test locate".to_string()),
            updated_at: now,
        };

        led_repo.set_dock_led(&state).unwrap();

        let loaded = led_repo.get_dock_led("BASE-1:2:2").unwrap().unwrap();
        assert_eq!(loaded.mode, LedMode::Locate);
        assert_eq!(loaded.rgb(), COLOR_LOCATE);

        // Before expiration: count is 0
        let cleared = led_repo.clear_expired_leds(now).unwrap();
        assert_eq!(cleared, 0);

        // After expiration
        let cleared = led_repo
            .clear_expired_leds(now + time::Duration::seconds(15))
            .unwrap();
        assert_eq!(cleared, 1);
        assert!(led_repo.get_dock_led("BASE-1:2:2").unwrap().is_none());
    }

    #[test]
    fn test_locate_target_illuminates_multi_cell_footprint() {
        let conn = setup_db();
        let loc_repo = SqliteLocationRepo::new(&conn);
        let grid_repo = SqliteGridRepo::new(&conn);
        let led_repo = SqliteLedRepo::new(&conn);

        let loc = loc_repo
            .create(&NewLocation {
                parent_id: None,
                name: "Drawer 2".to_string(),
                kind: LocationKind::Drawer,
            })
            .unwrap();

        let base = grid_repo
            .create_base(&NewGridBase {
                location_id: loc.id,
                code: "BASE-B".to_string(),
                cols: 5,
                rows: 5,
                pitch_mm: None,
            })
            .unwrap();

        let bin = grid_repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "BIN-2X2".to_string(),
                    width_units: 2,
                    depth_units: 2,
                    height_units: 3,
                    compartment_count: 1,
                    parent_location_id: Some(loc.id),
                },
                &loc.id,
            )
            .unwrap();

        grid_repo
            .place_bin(&slim_core::model::gridfinity::PlaceBinCmd {
                bin_id: bin.id,
                base_id: base.id,
                x: 1,
                y: 1,
                rotation: Rotation::Rot0,
            })
            .unwrap();

        let now = OffsetDateTime::now_utc();
        let states = led_repo.locate_target("BIN-2X2", Some(45), now).unwrap();

        // 2x2 bin should illuminate 4 docks
        assert_eq!(states.len(), 4);
        let dock_ids: Vec<String> = states.into_iter().map(|s| s.dock_id).collect();
        assert!(dock_ids.contains(&"BASE-B:1:1".to_string()));
        assert!(dock_ids.contains(&"BASE-B:2:1".to_string()));
        assert!(dock_ids.contains(&"BASE-B:1:2".to_string()));
        assert!(dock_ids.contains(&"BASE-B:2:2".to_string()));
    }
}
