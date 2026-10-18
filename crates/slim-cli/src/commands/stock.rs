//! Stock operation commands (receive, issue, transfer, adjust, balance, low, threshold).

use anyhow::Result;
use rusqlite::Connection;
use slim_core::model::operation::*;
use slim_core::model::stock::StockThreshold;
use slim_core::service::{InventoryService, LocationRepository, ThresholdRepository};
use slim_db::repo::inventory::{SqliteInventoryService, SqliteThresholdRepo};
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::{resolve_actor, resolve_item, resolve_location};
use crate::cli::StockCommands;

pub fn handle_stock(
    conn: &Connection,
    cmd: StockCommands,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
    json: bool,
) -> Result<()> {
    let inv = SqliteInventoryService::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);

    match cmd {
        StockCommands::Receive {
            item,
            qty,
            to,
            reason,
            idempotency_key,
        } => {
            let actor = resolve_actor(conn, cli_actor, config_actor)?;
            let it = resolve_item(conn, &item)?;
            let dest = resolve_location(conn, &to)?;

            let receipt = inv.receive(ReceiveStock {
                actor_id: actor.id,
                item_id: it.id,
                destination_id: dest.id,
                quantity: qty,
                source: OperationSource::Cli,
                reason,
                idempotency_key,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            } else {
                let new_bal = receipt
                    .resulting_balances
                    .first()
                    .map(|b| b.qty)
                    .unwrap_or(0);
                println!(
                    "Received {} {} of '{}' into '{}' (New balance: {})",
                    qty, it.unit, it.name, dest.name, new_bal
                );
                println!("Operation ID: {}", receipt.operation_id);
            }
        }
        StockCommands::Issue {
            item,
            qty,
            from,
            reason,
            idempotency_key,
        } => {
            let actor = resolve_actor(conn, cli_actor, config_actor)?;
            let it = resolve_item(conn, &item)?;
            let src = resolve_location(conn, &from)?;

            let receipt = inv.issue(IssueStock {
                actor_id: actor.id,
                item_id: it.id,
                source_id: src.id,
                quantity: qty,
                source: OperationSource::Cli,
                reason,
                idempotency_key,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            } else {
                let new_bal = receipt
                    .resulting_balances
                    .first()
                    .map(|b| b.qty)
                    .unwrap_or(0);
                println!(
                    "Issued {} {} of '{}' from '{}' (Remaining balance: {})",
                    qty, it.unit, it.name, src.name, new_bal
                );
                println!("Operation ID: {}", receipt.operation_id);
            }
        }
        StockCommands::Transfer {
            item,
            qty,
            from,
            to,
            reason,
            idempotency_key,
        } => {
            let actor = resolve_actor(conn, cli_actor, config_actor)?;
            let it = resolve_item(conn, &item)?;
            let src = resolve_location(conn, &from)?;
            let dest = resolve_location(conn, &to)?;

            let receipt = inv.transfer(TransferStock {
                actor_id: actor.id,
                item_id: it.id,
                from_id: src.id,
                to_id: dest.id,
                quantity: qty,
                source: OperationSource::Cli,
                reason,
                idempotency_key,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            } else {
                println!(
                    "Transferred {} {} of '{}' from '{}' to '{}'",
                    qty, it.unit, it.name, src.name, dest.name
                );
                for b in &receipt.resulting_balances {
                    let loc_name = if b.location_id == src.id {
                        &src.name
                    } else {
                        &dest.name
                    };
                    println!("  {}: {} {}", loc_name, b.qty, it.unit);
                }
                println!("Operation ID: {}", receipt.operation_id);
            }
        }
        StockCommands::Adjust {
            item,
            set,
            at,
            reason,
            idempotency_key,
        } => {
            let actor = resolve_actor(conn, cli_actor, config_actor)?;
            let it = resolve_item(conn, &item)?;
            let loc = resolve_location(conn, &at)?;

            let receipt = inv.adjust(AdjustStock {
                actor_id: actor.id,
                item_id: it.id,
                location_id: loc.id,
                new_quantity: set,
                source: OperationSource::Cli,
                reason: reason.clone(),
                idempotency_key,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            } else {
                let delta = receipt.movements.first().map(|m| m.delta_qty).unwrap_or(0);
                let delta_str = if delta >= 0 {
                    format!("+{}", delta)
                } else {
                    delta.to_string()
                };
                println!(
                    "Adjusted '{}' at '{}' to {} {} (delta: {}, reason: '{}')",
                    it.name, loc.name, set, it.unit, delta_str, reason
                );
                println!("Operation ID: {}", receipt.operation_id);
            }
        }
        StockCommands::Balance { item } => {
            let it = resolve_item(conn, &item)?;
            let balances = inv.get_item_balances(&it.id)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&balances)?);
            } else if balances.is_empty() {
                println!(
                    "No stock on hand for '{}' ({})",
                    it.name,
                    it.sku.as_deref().unwrap_or("no SKU")
                );
            } else {
                let total: u64 = balances.iter().map(|b| u64::from(b.qty)).sum();
                println!(
                    "Stock Balances for '{}' (Total: {} {})",
                    it.name, total, it.unit
                );
                println!("{}", "─".repeat(60));
                for b in &balances {
                    let loc_path = loc_repo
                        .get_path(&b.location_id)
                        .map(|p| {
                            p.iter()
                                .map(|l| l.name.clone())
                                .collect::<Vec<_>>()
                                .join(" > ")
                        })
                        .unwrap_or_else(|_| b.location_id.to_string());
                    println!("{:<40} {:>8} {}", loc_path, b.qty, it.unit);
                }
            }
        }
        StockCommands::Low => {
            let alerts = inv.get_low_stock()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&alerts)?);
            } else if alerts.is_empty() {
                println!("All items are currently at or above their minimum stock thresholds.");
            } else {
                println!(
                    "{:<16} {:<24} {:<24} {:>8} {:>8}",
                    "SKU", "Item", "Location", "On Hand", "Min Qty"
                );
                println!("{}", "─".repeat(84));
                for a in &alerts {
                    println!(
                        "{:<16} {:<24} {:<24} {:>8} {:>8}",
                        a.item_sku.as_deref().unwrap_or("-"),
                        a.item_name,
                        a.location_name,
                        a.on_hand,
                        a.min_qty
                    );
                }
            }
        }
        StockCommands::Threshold {
            item,
            min,
            location,
        } => {
            let it = resolve_item(conn, &item)?;
            let loc_id = if let Some(ref loc_name) = location {
                Some(resolve_location(conn, loc_name)?.id)
            } else {
                None
            };

            let thresh_repo = SqliteThresholdRepo::new(conn);
            thresh_repo.set(&StockThreshold {
                item_id: it.id,
                stock_location_id: loc_id,
                min_qty: min,
            })?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "item_id": it.id.to_string(),
                        "stock_location_id": loc_id.map(|l| l.to_string()),
                        "min_qty": min,
                        "status": "set"
                    })
                );
            } else {
                let scope = match location {
                    Some(l) => format!("at location '{}'", l),
                    None => "globally".to_string(),
                };
                println!(
                    "Set minimum threshold for '{}' to {} {}",
                    it.name, min, scope
                );
            }
        }
    }
    Ok(())
}
