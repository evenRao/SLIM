//! Gridfinity baseplate subcommands (create, list, show).

use anyhow::{Result, bail};
use rusqlite::Connection;
use slim_core::model::gridfinity::NewGridBase;
use slim_core::service::{GridRepository, LocationRepository};
use slim_db::repo::grid::SqliteGridRepo;
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::resolve_location;
use crate::cli::GridCommands;

pub fn handle_grid(conn: &Connection, cmd: GridCommands, json: bool) -> Result<()> {
    let repo = SqliteGridRepo::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);

    match cmd {
        GridCommands::Create {
            code,
            parent,
            cols,
            rows,
            pitch,
        } => {
            let parent_loc = resolve_location(conn, &parent)?;
            let base = repo.create_base(&NewGridBase {
                location_id: parent_loc.id,
                code: code.clone(),
                cols,
                rows,
                pitch_mm: pitch,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&base)?);
            } else {
                println!(
                    "Created Gridfinity baseplate '{}' ({}x{} cells, {:.1}mm pitch) in '{}'",
                    base.code, base.cols, base.rows, base.pitch_mm, parent_loc.name
                );
                println!("Base ID: {}", base.id);
            }
        }
        GridCommands::List => {
            let bases = repo.list_bases(None)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&bases)?);
            } else if bases.is_empty() {
                println!(
                    "No Gridfinity baseplates registered. Create one with 'slim grid create'."
                );
            } else {
                println!(
                    "{:<16} {:<12} {:<14} {:<24}",
                    "Base Code", "Grid (WxD)", "Pitch (mm)", "Mounted In"
                );
                println!("{}", "─".repeat(68));
                for b in &bases {
                    let loc_name = loc_repo
                        .get_id_or_name(&b.location_id)
                        .unwrap_or_else(|| b.location_id.to_string());

                    println!(
                        "{:<16} {:<12} {:<14.1} {:<24}",
                        b.code,
                        format!("{}x{}", b.cols, b.rows),
                        b.pitch_mm,
                        loc_name
                    );
                }
            }
        }
        GridCommands::Show { base } => {
            let b = match repo.get_base_by_code(&base)? {
                Some(b) => b,
                None => bail!("Gridfinity baseplate '{}' not found", base),
            };

            let grid_map = repo.render_grid_map(&b.id)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&grid_map)?);
            } else {
                let loc_name = loc_repo
                    .get_id_or_name(&b.location_id)
                    .unwrap_or_else(|| b.location_id.to_string());

                println!("Gridfinity Baseplate: {}", b.code);
                println!("Mounted In:           {}", loc_name);
                println!(
                    "Dimensions:           {} x {} units ({:.1} x {:.1} mm)",
                    b.cols,
                    b.rows,
                    b.width_mm(),
                    b.depth_mm()
                );
                println!("Pitch:                {:.1} mm", b.pitch_mm);
                println!("Active Bins Placed:   {}", grid_map.placed_bins.len());
                if !grid_map.placed_bins.is_empty() {
                    println!();
                    println!("Placed Bins:");
                    for pb in &grid_map.placed_bins {
                        let (ew, ed) = pb.effective_dimensions();
                        println!(
                            "  - {:<12} at ({}, {}) [{}x{} cells, rot {}°]",
                            pb.bin_code,
                            pb.placement.x,
                            pb.placement.y,
                            ew,
                            ed,
                            pb.placement.rotation
                        );
                    }
                }
                println!();
                println!("{}", grid_map.render_ascii());
            }
        }
    }

    Ok(())
}

trait LocationRepoExt {
    fn get_id_or_name(&self, id: &slim_core::model::location::LocationId) -> Option<String>;
}

impl LocationRepoExt for SqliteLocationRepo<'_> {
    fn get_id_or_name(&self, id: &slim_core::model::location::LocationId) -> Option<String> {
        self.get_by_id(id).ok().flatten().map(|l| l.name)
    }
}
