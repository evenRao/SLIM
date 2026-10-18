//! Label generation subcommands (QR, item, bin, location).

use anyhow::{Result, bail};
use rusqlite::Connection;
use std::fs;

use slim_core::label::{
    generate_bin_label, generate_item_label, generate_location_label, generate_svg_qr,
    generate_terminal_qr,
};
use slim_core::service::{GridRepository, LocationRepository};
use slim_db::repo::grid::SqliteGridRepo;
use slim_db::repo::location::SqliteLocationRepo;

use crate::cli::LabelCommands;

pub fn handle_label(conn: &Connection, cmd: LabelCommands, json: bool) -> Result<()> {
    match cmd {
        LabelCommands::Qr { payload, svg, file } => {
            if svg {
                let svg_content = generate_svg_qr(&payload)?;
                if let Some(path) = file {
                    fs::write(&path, &svg_content)?;
                    println!("Saved SVG QR code to '{}'", path.display());
                } else {
                    println!("{}", svg_content);
                }
            } else {
                let qr = generate_terminal_qr(&payload)?;
                if let Some(path) = file {
                    fs::write(&path, &qr)?;
                    println!("Saved terminal QR code to '{}'", path.display());
                } else if json {
                    println!("{}", serde_json::json!({ "payload": payload, "qr": qr }));
                } else {
                    println!("{}", qr);
                }
            }
        }
        LabelCommands::Item { sku, file } => {
            let item = super::resolve::resolve_item(conn, &sku)?;

            if let Some(path) = file {
                let payload = slim_core::model::scan::item_uri(&item.id.to_string());
                let svg_content = generate_svg_qr(&payload)?;
                fs::write(&path, &svg_content)?;
                println!("Saved item QR SVG to '{}'", path.display());
            } else if json {
                println!("{}", serde_json::to_string_pretty(&item)?);
            } else {
                let label = generate_item_label(&item)?;
                print!("{}", label);
            }
        }
        LabelCommands::Bin { bin, file } => {
            let grid_repo = SqliteGridRepo::new(conn);
            let loc_repo = SqliteLocationRepo::new(conn);

            let bin_entry = match grid_repo.get_bin_by_code(&bin)? {
                Some(b) => b,
                None => bail!("Gridfinity bin '{}' not found", bin),
            };

            let parent_name = loc_repo
                .get_by_id(&bin_entry.location_id)?
                .and_then(|l| l.parent_id)
                .and_then(|pid| loc_repo.get_by_id(&pid).ok().flatten())
                .map(|l| l.name);

            if let Some(path) = file {
                let payload = slim_core::model::scan::bin_uri(&bin_entry.bin_code);
                let svg_content = generate_svg_qr(&payload)?;
                fs::write(&path, &svg_content)?;
                println!("Saved bin QR SVG to '{}'", path.display());
            } else if json {
                println!("{}", serde_json::to_string_pretty(&bin_entry)?);
            } else {
                let label = generate_bin_label(&bin_entry, parent_name.as_deref())?;
                print!("{}", label);
            }
        }
        LabelCommands::Location { name, file } => {
            let loc_repo = SqliteLocationRepo::new(conn);
            let loc = match loc_repo.find_by_name(&name, None)? {
                Some(l) => l,
                None => bail!("Location '{}' not found", name),
            };

            if let Some(path) = file {
                let payload = slim_core::model::scan::location_uri(&loc.name);
                let svg_content = generate_svg_qr(&payload)?;
                fs::write(&path, &svg_content)?;
                println!("Saved location QR SVG to '{}'", path.display());
            } else if json {
                println!("{}", serde_json::to_string_pretty(&loc)?);
            } else {
                let label = generate_location_label(&loc)?;
                print!("{}", label);
            }
        }
    }

    Ok(())
}
