//! Scanner subcommands and interactive scan loop.

use anyhow::Result;
use rusqlite::Connection;
use std::io::{self, BufRead, Write};

use slim_core::model::operation::{IssueStock, OperationSource, ReceiveStock};
use slim_core::model::scan::ScanTarget;
use slim_core::service::{
    GridRepository, InventoryService, ItemRepository, LocationRepository, ScanResolver,
};
use slim_db::repo::grid::SqliteGridRepo;
use slim_db::repo::inventory::SqliteInventoryService;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;
use slim_db::repo::scan::SqliteScanResolver;

use super::resolve::resolve_actor;
use crate::cli::ScanCommands;

pub fn handle_scan(
    conn: &Connection,
    cmd: Option<ScanCommands>,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
    json: bool,
) -> Result<()> {
    match cmd {
        Some(ScanCommands::Resolve { code }) => handle_resolve(conn, &code, json),
        None => run_interactive_scanner(conn, cli_actor, config_actor),
    }
}

pub fn handle_resolve(conn: &Connection, code: &str, json: bool) -> Result<()> {
    let resolver = SqliteScanResolver::new(conn);
    let target = resolver.resolve_scan(code)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&target)?);
        return Ok(());
    }

    let inv_service = SqliteInventoryService::new(conn);
    let item_repo = SqliteItemRepo::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);
    let grid_repo = SqliteGridRepo::new(conn);

    match target {
        ScanTarget::Item(item) => {
            println!("Target:               Catalog Item");
            println!("Name:                 {}", item.name);
            println!(
                "SKU:                  {}",
                item.sku.as_deref().unwrap_or("(none)")
            );
            println!("Base Unit:            {}", item.unit);
            if let Some(desc) = item.description {
                println!("Description:          {}", desc);
            }
            println!();

            let balances = inv_service.get_item_balances(&item.id)?;
            println!("Stock Distribution:");
            if balances.is_empty() {
                println!("  (Zero stock recorded in any location)");
            } else {
                for b in balances {
                    let loc_name = loc_repo
                        .get_by_id(&b.location_id)?
                        .map(|l| l.name)
                        .unwrap_or_else(|| b.location_id.to_string());
                    println!("  - {}: {} {}", loc_name, b.qty, item.unit);
                }
            }
        }
        ScanTarget::Bin { bin, location } => {
            println!("Target:               Gridfinity Bin");
            println!("Bin Code:             {}", bin.bin_code);
            println!(
                "Dimensions:           {}W x {}D x {}H ({} compartments)",
                bin.width_units, bin.depth_units, bin.height_units, bin.compartment_count
            );
            println!(
                "Physical:             {:.1} x {:.1} x {:.1} mm",
                bin.width_mm(slim_core::model::gridfinity::GRIDFINITY_PITCH_MM),
                bin.depth_mm(slim_core::model::gridfinity::GRIDFINITY_PITCH_MM),
                bin.height_mm()
            );

            if let Some(placement_info) = grid_repo.get_active_placement_for_bin(&bin.id)? {
                let base_code = grid_repo
                    .get_base_by_id(&placement_info.placement.base_id)?
                    .map(|b| b.code)
                    .unwrap_or_else(|| placement_info.placement.base_id.to_string());
                println!(
                    "Current Placement:    Baseplate '{}' at X={}, Y={} (rot {}°)",
                    base_code,
                    placement_info.placement.x,
                    placement_info.placement.y,
                    placement_info.placement.rotation
                );
            } else {
                println!("Current Placement:    (unplaced / in transit)");
            }

            println!();
            println!("Contained Stock Balances:");
            let balances = inv_service.get_location_balances(&location.id)?;
            if balances.is_empty() {
                println!("  (No stock currently held in this bin)");
            } else {
                for b in balances {
                    let it_name = item_repo
                        .get_by_id(&b.item_id)?
                        .map(|it| it.name)
                        .unwrap_or_else(|| b.item_id.to_string());
                    println!("  - {}: {} units", it_name, b.qty);
                }
            }
        }
        ScanTarget::Location(location) => {
            println!("Target:               Storage Location");
            println!("Name:                 {}", location.name);
            println!("Kind:                 {}", location.kind);
            println!(
                "Stock Allowed:        {}",
                if location.can_hold_stock { "yes" } else { "no" }
            );
            println!();

            println!("Contained Stock Balances:");
            let balances = inv_service.get_location_balances(&location.id)?;
            if balances.is_empty() {
                println!("  (No stock currently held in this location)");
            } else {
                for b in balances {
                    let it_name = item_repo
                        .get_by_id(&b.item_id)?
                        .map(|it| it.name)
                        .unwrap_or_else(|| b.item_id.to_string());
                    println!("  - {}: {} units", it_name, b.qty);
                }
            }
        }
        ScanTarget::Unregistered { scanned_code } => {
            println!("Target:               Unregistered Code");
            println!("Scanned Input:        '{}'", scanned_code);
            println!(
                "Note: This code does not match any SKU, bin, location, or registered barcode."
            );
            println!(
                "To associate it, use 'slim identifier add --kind barcode --value \"{}\" --item <SKU>'.",
                scanned_code
            );
        }
    }

    Ok(())
}

fn run_interactive_scanner(
    conn: &Connection,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
) -> Result<()> {
    let actor = resolve_actor(conn, cli_actor, config_actor)?;

    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║              SLIM SCANNER ASSISTED STATION                ║");
    println!("║  Ready for USB HID scanner or manual barcode entry.       ║");
    println!("║  Press 'q' then Enter to return to command prompt.        ║");
    println!("╚═══════════════════════════════════════════════════════════╝");
    println!("Active Operator: {}", actor.display_name);
    println!();

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        print!("[SCAN] >> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break; // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("q") || trimmed.eq_ignore_ascii_case("quit") {
            println!("Exiting scanner station.");
            break;
        }

        let resolver = SqliteScanResolver::new(conn);
        let target = resolver.resolve_scan(trimmed)?;

        println!();
        println!("─── Scanned Entity: {} ───", target.display_label());

        match target {
            ScanTarget::Item(item) => {
                println!("Options: [r] Receive stock  [i] Issue stock  [enter] Next scan");
                print!("Action: ");
                io::stdout().flush()?;
                let mut action_line = String::new();
                reader.read_line(&mut action_line)?;
                let action = action_line.trim().to_lowercase();

                if action == "r" {
                    print!("Receive quantity: ");
                    io::stdout().flush()?;
                    let mut q_str = String::new();
                    reader.read_line(&mut q_str)?;
                    let qty: u32 = match q_str.trim().parse() {
                        Ok(q) if q > 0 => q,
                        _ => {
                            println!("Invalid quantity. Operation cancelled.\n");
                            continue;
                        }
                    };

                    print!("Destination location or bin name: ");
                    io::stdout().flush()?;
                    let mut loc_str = String::new();
                    reader.read_line(&mut loc_str)?;
                    let dest = super::resolve::resolve_location(conn, loc_str.trim())?;

                    let inv_service = SqliteInventoryService::new(conn);
                    let receipt = inv_service.receive(ReceiveStock {
                        actor_id: actor.id,
                        item_id: item.id,
                        destination_id: dest.id,
                        quantity: qty,
                        source: OperationSource::Scanner,
                        reason: Some("Scanner check-in".to_string()),
                        idempotency_key: None,
                    })?;

                    println!(
                        "✔ Received {} {} of '{}' into '{}' (Operation: {})",
                        qty, item.unit, item.name, dest.name, receipt.operation_id
                    );
                } else if action == "i" {
                    print!("Issue quantity: ");
                    io::stdout().flush()?;
                    let mut q_str = String::new();
                    reader.read_line(&mut q_str)?;
                    let qty: u32 = match q_str.trim().parse() {
                        Ok(q) if q > 0 => q,
                        _ => {
                            println!("Invalid quantity. Operation cancelled.\n");
                            continue;
                        }
                    };

                    print!("Source location or bin name: ");
                    io::stdout().flush()?;
                    let mut loc_str = String::new();
                    reader.read_line(&mut loc_str)?;
                    let src = super::resolve::resolve_location(conn, loc_str.trim())?;

                    let inv_service = SqliteInventoryService::new(conn);
                    let receipt = inv_service.issue(IssueStock {
                        actor_id: actor.id,
                        item_id: item.id,
                        source_id: src.id,
                        quantity: qty,
                        source: OperationSource::Scanner,
                        reason: Some("Scanner check-out".to_string()),
                        idempotency_key: None,
                    })?;

                    println!(
                        "✔ Issued {} {} of '{}' from '{}' (Operation: {})",
                        qty, item.unit, item.name, src.name, receipt.operation_id
                    );
                }
            }
            ScanTarget::Bin { bin, location } => {
                println!(
                    "Options: [r] Receive into this bin  [i] Issue from this bin  [enter] Next scan"
                );
                print!("Action: ");
                io::stdout().flush()?;
                let mut action_line = String::new();
                reader.read_line(&mut action_line)?;
                let action = action_line.trim().to_lowercase();

                if action == "r" {
                    print!("Item SKU or name to receive: ");
                    io::stdout().flush()?;
                    let mut item_str = String::new();
                    reader.read_line(&mut item_str)?;
                    let item = super::resolve::resolve_item(conn, item_str.trim())?;

                    print!("Quantity: ");
                    io::stdout().flush()?;
                    let mut q_str = String::new();
                    reader.read_line(&mut q_str)?;
                    let qty: u32 = match q_str.trim().parse() {
                        Ok(q) if q > 0 => q,
                        _ => {
                            println!("Invalid quantity. Operation cancelled.\n");
                            continue;
                        }
                    };

                    let inv_service = SqliteInventoryService::new(conn);
                    let receipt = inv_service.receive(ReceiveStock {
                        actor_id: actor.id,
                        item_id: item.id,
                        destination_id: location.id,
                        quantity: qty,
                        source: OperationSource::Scanner,
                        reason: Some(format!("Scanner receive into bin {}", bin.bin_code)),
                        idempotency_key: None,
                    })?;

                    println!(
                        "✔ Received {} {} of '{}' into bin '{}' (Operation: {})",
                        qty, item.unit, item.name, bin.bin_code, receipt.operation_id
                    );
                } else if action == "i" {
                    print!("Item SKU or name to issue: ");
                    io::stdout().flush()?;
                    let mut item_str = String::new();
                    reader.read_line(&mut item_str)?;
                    let item = super::resolve::resolve_item(conn, item_str.trim())?;

                    print!("Quantity: ");
                    io::stdout().flush()?;
                    let mut q_str = String::new();
                    reader.read_line(&mut q_str)?;
                    let qty: u32 = match q_str.trim().parse() {
                        Ok(q) if q > 0 => q,
                        _ => {
                            println!("Invalid quantity. Operation cancelled.\n");
                            continue;
                        }
                    };

                    let inv_service = SqliteInventoryService::new(conn);
                    let receipt = inv_service.issue(IssueStock {
                        actor_id: actor.id,
                        item_id: item.id,
                        source_id: location.id,
                        quantity: qty,
                        source: OperationSource::Scanner,
                        reason: Some(format!("Scanner issue from bin {}", bin.bin_code)),
                        idempotency_key: None,
                    })?;

                    println!(
                        "✔ Issued {} {} of '{}' from bin '{}' (Operation: {})",
                        qty, item.unit, item.name, bin.bin_code, receipt.operation_id
                    );
                }
            }
            ScanTarget::Location(location) => {
                println!(
                    "Location '{}' [{}]. Ready for next scan.",
                    location.name, location.kind
                );
            }
            ScanTarget::Unregistered { scanned_code } => {
                println!(
                    "Unregistered barcode/code '{}'. Register with 'slim identifier add'.",
                    scanned_code
                );
            }
        }

        println!();
    }

    Ok(())
}
