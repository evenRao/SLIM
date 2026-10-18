//! Scanner resolution repository — SQLite implementation.

use rusqlite::Connection;
use uuid::Uuid;

use slim_core::error::CoreResult;
use slim_core::model::gridfinity::GridfinityBinId;
use slim_core::model::item::ItemId;
use slim_core::model::location::LocationId;
use slim_core::model::scan::{ParsedUri, ScanTarget};
use slim_core::service::{GridRepository, ItemRepository, LocationRepository, ScanResolver};

use super::grid::SqliteGridRepo;
use super::item::SqliteItemRepo;
use super::location::SqliteLocationRepo;

pub struct SqliteScanResolver<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteScanResolver<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl ScanResolver for SqliteScanResolver<'_> {
    fn resolve_scan(&self, code: &str) -> CoreResult<ScanTarget> {
        let trimmed = code.trim();
        if trimmed.is_empty() {
            return Ok(ScanTarget::Unregistered {
                scanned_code: String::new(),
            });
        }

        let item_repo = SqliteItemRepo::new(self.conn);
        let loc_repo = SqliteLocationRepo::new(self.conn);
        let grid_repo = SqliteGridRepo::new(self.conn);

        match ParsedUri::parse(trimmed) {
            ParsedUri::Item(val) => {
                // 1. Try SKU
                if let Some(item) = item_repo.get_by_sku(val)? {
                    return Ok(ScanTarget::Item(item));
                }
                // 2. Try UUID
                if let Ok(u) = Uuid::parse_str(val)
                    && let Some(item) = item_repo.get_by_id(&ItemId(u))?
                {
                    return Ok(ScanTarget::Item(item));
                }
                Ok(ScanTarget::Unregistered {
                    scanned_code: trimmed.to_string(),
                })
            }
            ParsedUri::Bin(val) => {
                // 1. Try bin_code
                if let Some(bin) = grid_repo.get_bin_by_code(val)?
                    && let Some(loc) = loc_repo.get_by_id(&bin.location_id)?
                {
                    return Ok(ScanTarget::Bin { bin, location: loc });
                }
                // 2. Try UUID
                if let Ok(u) = Uuid::parse_str(val)
                    && let Some(bin) = grid_repo.get_bin_by_id(&GridfinityBinId(u))?
                    && let Some(loc) = loc_repo.get_by_id(&bin.location_id)?
                {
                    return Ok(ScanTarget::Bin { bin, location: loc });
                }
                Ok(ScanTarget::Unregistered {
                    scanned_code: trimmed.to_string(),
                })
            }
            ParsedUri::Location(val) => {
                // 1. Try name
                if let Some(loc) = loc_repo.find_by_name(val, None)? {
                    if let Some(bin) = grid_repo.get_bin_by_location_id(&loc.id)? {
                        return Ok(ScanTarget::Bin { bin, location: loc });
                    }
                    return Ok(ScanTarget::Location(loc));
                }
                // 2. Try UUID
                if let Ok(u) = Uuid::parse_str(val)
                    && let Some(loc) = loc_repo.get_by_id(&LocationId(u))?
                {
                    if let Some(bin) = grid_repo.get_bin_by_location_id(&loc.id)? {
                        return Ok(ScanTarget::Bin { bin, location: loc });
                    }
                    return Ok(ScanTarget::Location(loc));
                }
                Ok(ScanTarget::Unregistered {
                    scanned_code: trimmed.to_string(),
                })
            }
            ParsedUri::Raw(val) => {
                // Step A: Look up in `identifiers` table (barcode, QR, manual, etc.)
                let mut ident_stmt = self
                    .conn
                    .prepare(
                        "SELECT item_id, location_id FROM identifiers WHERE value = ?1 COLLATE NOCASE AND archived_at IS NULL",
                    )
                    .map_err(|e| slim_core::error::CoreError::Storage(e.to_string()))?;

                let ident_row: Result<Option<(Option<String>, Option<String>)>, _> = ident_stmt
                    .query_row(rusqlite::params![val], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        err => Err(slim_core::error::CoreError::Storage(err.to_string())),
                    });

                if let Ok(Some((item_str, loc_str))) = ident_row {
                    if let Some(is) = item_str {
                        if let Ok(u) = Uuid::parse_str(&is)
                            && let Some(item) = item_repo.get_by_id(&ItemId(u))?
                        {
                            return Ok(ScanTarget::Item(item));
                        }
                    } else if let Some(ls) = loc_str
                        && let Ok(u) = Uuid::parse_str(&ls)
                    {
                        let loc_id = LocationId(u);
                        if let Some(loc) = loc_repo.get_by_id(&loc_id)? {
                            if let Some(bin) = grid_repo.get_bin_by_location_id(&loc.id)? {
                                return Ok(ScanTarget::Bin { bin, location: loc });
                            }
                            return Ok(ScanTarget::Location(loc));
                        }
                    }
                }

                // Step B: Direct SKU lookup
                if let Some(item) = item_repo.get_by_sku(val)? {
                    return Ok(ScanTarget::Item(item));
                }

                // Step C: Direct Gridfinity bin code lookup
                if let Some(bin) = grid_repo.get_bin_by_code(val)?
                    && let Some(loc) = loc_repo.get_by_id(&bin.location_id)?
                {
                    return Ok(ScanTarget::Bin { bin, location: loc });
                }

                // Step D: Direct location name lookup
                if let Some(loc) = loc_repo.find_by_name(val, None)? {
                    if let Some(bin) = grid_repo.get_bin_by_location_id(&loc.id)? {
                        return Ok(ScanTarget::Bin { bin, location: loc });
                    }
                    return Ok(ScanTarget::Location(loc));
                }

                // Step E: Try UUID lookup
                if let Ok(u) = Uuid::parse_str(val) {
                    if let Some(item) = item_repo.get_by_id(&ItemId(u))? {
                        return Ok(ScanTarget::Item(item));
                    }
                    if let Some(bin) = grid_repo.get_bin_by_id(&GridfinityBinId(u))?
                        && let Some(loc) = loc_repo.get_by_id(&bin.location_id)?
                    {
                        return Ok(ScanTarget::Bin { bin, location: loc });
                    }
                    if let Some(loc) = loc_repo.get_by_id(&LocationId(u))? {
                        if let Some(bin) = grid_repo.get_bin_by_location_id(&loc.id)? {
                            return Ok(ScanTarget::Bin { bin, location: loc });
                        }
                        return Ok(ScanTarget::Location(loc));
                    }
                }

                // Not found
                Ok(ScanTarget::Unregistered {
                    scanned_code: trimmed.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;
    use crate::repo::identifier::SqliteIdentifierRepo;
    use slim_core::model::gridfinity::NewGridfinityBin;
    use slim_core::model::identifier::{IdentifierKind, IdentifierTarget, NewIdentifier};
    use slim_core::model::item::NewItem;
    use slim_core::model::location::{LocationKind, NewLocation};
    use slim_core::service::IdentifierRepository;

    fn setup_test_db() -> Connection {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn test_scan_resolves_sku_directly() {
        let conn = setup_test_db();
        let item_repo = SqliteItemRepo::new(&conn);
        item_repo
            .create(&NewItem {
                sku: Some("M3-10".to_string()),
                name: "M3x10 Screw".to_string(),
                unit: "each".to_string(),
                description: None,
                metadata: None,
            })
            .unwrap();

        let resolver = SqliteScanResolver::new(&conn);
        let target = resolver.resolve_scan("M3-10").unwrap();
        assert!(target.is_item());
        if let ScanTarget::Item(item) = target {
            assert_eq!(item.name, "M3x10 Screw");
        } else {
            panic!("expected Item");
        }
    }

    #[test]
    fn test_scan_resolves_barcode_identifier_to_item() {
        let conn = setup_test_db();
        let item_repo = SqliteItemRepo::new(&conn);
        let item = item_repo
            .create(&NewItem {
                sku: Some("RES-1K".to_string()),
                name: "1k Resistor".to_string(),
                unit: "each".to_string(),
                description: None,
                metadata: None,
            })
            .unwrap();

        let ident_repo = SqliteIdentifierRepo::new(&conn);
        ident_repo
            .create(&NewIdentifier {
                kind: IdentifierKind::Barcode,
                value: "0123456789".to_string(),
                target: IdentifierTarget::Item(item.id),
            })
            .unwrap();

        let resolver = SqliteScanResolver::new(&conn);
        let target = resolver.resolve_scan("0123456789").unwrap();
        assert!(target.is_item());
        if let ScanTarget::Item(it) = target {
            assert_eq!(it.sku.unwrap(), "RES-1K");
        }
    }

    #[test]
    fn test_scan_resolves_gridfinity_bin() {
        let conn = setup_test_db();
        let loc_repo = SqliteLocationRepo::new(&conn);
        let grid_repo = SqliteGridRepo::new(&conn);

        let loc = loc_repo
            .create(&NewLocation {
                parent_id: None,
                name: "GF-BIN-101".to_string(),
                kind: LocationKind::Bin,
            })
            .unwrap();

        grid_repo
            .create_bin(
                &NewGridfinityBin {
                    bin_code: "GF-BIN-101".to_string(),
                    width_units: 1,
                    depth_units: 2,
                    height_units: 6,
                    compartment_count: 1,
                    parent_location_id: None,
                },
                &loc.id,
            )
            .unwrap();

        let resolver = SqliteScanResolver::new(&conn);
        // By URI
        let target = resolver.resolve_scan("slim:bin:GF-BIN-101").unwrap();
        assert!(target.is_bin());

        // By Raw code
        let target_raw = resolver.resolve_scan("GF-BIN-101").unwrap();
        assert!(target_raw.is_bin());
    }

    #[test]
    fn test_scan_unregistered_code() {
        let conn = setup_test_db();
        let resolver = SqliteScanResolver::new(&conn);
        let target = resolver.resolve_scan("NOT-FOUND-999").unwrap();
        assert!(!target.is_registered());
        if let ScanTarget::Unregistered { scanned_code } = target {
            assert_eq!(scanned_code, "NOT-FOUND-999");
        }
    }
}
