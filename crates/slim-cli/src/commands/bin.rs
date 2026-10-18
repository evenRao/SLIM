//! Gridfinity bin subcommands (create, list, show, place, remove).

use anyhow::{Result, bail};
use rusqlite::Connection;
use slim_core::model::gridfinity::{GRIDFINITY_PITCH_MM, NewGridfinityBin, PlaceBinCmd, Rotation};
use slim_core::model::location::{LocationKind, NewLocation};
use slim_core::service::{GridRepository, InventoryService, ItemRepository, LocationRepository};
use slim_db::repo::grid::SqliteGridRepo;
use slim_db::repo::inventory::SqliteInventoryService;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::resolve_location;
use crate::cli::BinCommands;

pub fn handle_bin(conn: &Connection, cmd: BinCommands, json: bool) -> Result<()> {
    let repo = SqliteGridRepo::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);
    let inv_service = SqliteInventoryService::new(conn);
    let item_repo = SqliteItemRepo::new(conn);

    match cmd {
        BinCommands::Create {
            code,
            width,
            depth,
            height,
            compartments,
            parent,
        } => {
            let parent_id = match parent {
                Some(ref p) => Some(resolve_location(conn, p)?.id),
                None => None,
            };

            let tx = conn.unchecked_transaction()?;
            // 1. Create location row for this bin (ADR-01: Bins are location subtypes)
            let loc = loc_repo.create(&NewLocation {
                parent_id,
                name: code.clone(),
                kind: LocationKind::Bin,
            })?;

            // 2. Create Gridfinity bin metadata row
            let bin = repo.create_bin(
                &NewGridfinityBin {
                    bin_code: code.clone(),
                    width_units: width,
                    depth_units: depth,
                    height_units: height,
                    compartment_count: compartments,
                    parent_location_id: parent_id,
                },
                &loc.id,
            )?;

            tx.commit()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&bin)?);
            } else {
                println!(
                    "Created Gridfinity bin '{}' ({}x{}x{}U, {} compartments)",
                    bin.bin_code,
                    bin.width_units,
                    bin.depth_units,
                    bin.height_units,
                    bin.compartment_count
                );
                println!(
                    "Physical dimensions: {:.1} x {:.1} x {:.1} mm",
                    bin.width_mm(GRIDFINITY_PITCH_MM),
                    bin.depth_mm(GRIDFINITY_PITCH_MM),
                    bin.height_mm()
                );
                println!("Location ID: {}", loc.id);
            }
        }
        BinCommands::List => {
            let bins = repo.list_bins()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&bins)?);
            } else if bins.is_empty() {
                println!("No Gridfinity bins registered. Create one with 'slim bin create'.");
            } else {
                println!(
                    "{:<12} {:<12} {:<14} {:<16} Placement",
                    "Bin Code", "Units (WxDxH)", "Physical (mm)", "Compartments"
                );
                println!("{}", "─".repeat(74));
                for b in &bins {
                    let placement_str = match repo.get_active_placement_for_bin(&b.id)? {
                        Some(p) => {
                            let base_code = repo
                                .get_base_by_id(&p.placement.base_id)?
                                .map(|base| base.code)
                                .unwrap_or_else(|| p.placement.base_id.to_string());
                            format!(
                                "{} at ({},{}) [rot {}°]",
                                base_code, p.placement.x, p.placement.y, p.placement.rotation
                            )
                        }
                        None => "(unplaced)".to_string(),
                    };

                    println!(
                        "{:<12} {:<12} {:<14} {:<16} {}",
                        b.bin_code,
                        format!("{}x{}x{}", b.width_units, b.depth_units, b.height_units),
                        format!(
                            "{:.0}x{:.0}x{:.0}",
                            b.width_mm(GRIDFINITY_PITCH_MM),
                            b.depth_mm(GRIDFINITY_PITCH_MM),
                            b.height_mm()
                        ),
                        b.compartment_count,
                        placement_str
                    );
                }
            }
        }
        BinCommands::Show { bin } => {
            let b = match repo.get_bin_by_code(&bin)? {
                Some(b) => b,
                None => bail!("Gridfinity bin '{}' not found", bin),
            };

            let placement_info = repo.get_active_placement_for_bin(&b.id)?;
            let balances = inv_service.get_location_balances(&b.location_id)?;

            if json {
                let output = serde_json::json!({
                    "bin": b,
                    "placement": placement_info,
                    "balances": balances,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Gridfinity Bin:      {}", b.bin_code);
                println!(
                    "Grid Units:          {}W x {}D x {}H ({} compartments)",
                    b.width_units, b.depth_units, b.height_units, b.compartment_count
                );
                println!(
                    "Physical Dimensions: {:.1} x {:.1} x {:.1} mm",
                    b.width_mm(GRIDFINITY_PITCH_MM),
                    b.depth_mm(GRIDFINITY_PITCH_MM),
                    b.height_mm()
                );

                if let Some(ref p) = placement_info {
                    let base_code = repo
                        .get_base_by_id(&p.placement.base_id)?
                        .map(|base| base.code)
                        .unwrap_or_else(|| p.placement.base_id.to_string());
                    println!(
                        "Current Placement:   Baseplate '{}' at X={}, Y={} (rotation {}°)",
                        base_code, p.placement.x, p.placement.y, p.placement.rotation
                    );
                    let (ew, ed) = p.effective_dimensions();
                    println!("Occupied Footprint:  {} x {} cells", ew, ed);
                } else {
                    println!("Current Placement:   (unplaced / in transit)");
                }

                println!();
                println!("Contained Stock Balances:");
                if balances.is_empty() {
                    println!("  (No stock currently held in this bin)");
                } else {
                    for bal in &balances {
                        let item_label = match item_repo.get_by_id(&bal.item_id)? {
                            Some(it) => it.sku.unwrap_or(it.name),
                            None => bal.item_id.to_string(),
                        };
                        println!("  - {}: {} units", item_label, bal.qty);
                    }
                }
            }
        }
        BinCommands::Place {
            bin,
            grid,
            x,
            y,
            rotation,
        } => {
            let b = match repo.get_bin_by_code(&bin)? {
                Some(b) => b,
                None => bail!("Gridfinity bin '{}' not found", bin),
            };

            let base = match repo.get_base_by_code(&grid)? {
                Some(base) => base,
                None => bail!("Gridfinity baseplate '{}' not found", grid),
            };

            let rot: Rotation = rotation.parse()?;

            let placement = repo.place_bin(&PlaceBinCmd {
                bin_id: b.id,
                base_id: base.id,
                x,
                y,
                rotation: rot,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&placement)?);
            } else {
                let (ew, ed) = b.effective_dimensions(rot);
                println!(
                    "Placed bin '{}' onto baseplate '{}' at coordinate ({}, {}) with rotation {}°",
                    b.bin_code, base.code, x, y, rot
                );
                println!("Effective footprint: {} x {} cells", ew, ed);
            }
        }
        BinCommands::Remove { bin } => {
            let b = match repo.get_bin_by_code(&bin)? {
                Some(b) => b,
                None => bail!("Gridfinity bin '{}' not found", bin),
            };

            repo.remove_bin(&b.id)?;

            if json {
                println!(r#"{{"status": "removed", "bin": "{}"}}"#, b.bin_code);
            } else {
                println!(
                    "Removed bin '{}' from its active baseplate placement.",
                    b.bin_code
                );
                println!(
                    "Note: Stock balances associated with '{}' remain intact.",
                    b.bin_code
                );
            }
        }
    }

    Ok(())
}
