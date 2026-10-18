//! History subcommand handler.

use anyhow::{Result, bail};
use rusqlite::Connection;
use slim_core::service::{HistoryQuery, LocationRepository};
use slim_db::repo::inventory::SqliteHistoryQuery;
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::{resolve_item, resolve_location};
use crate::cli::HistoryArgs;

pub fn handle_history(conn: &Connection, args: HistoryArgs, json: bool) -> Result<()> {
    let hist = SqliteHistoryQuery::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);

    if let Some(ref op_id_str) = args.op {
        let op_id = slim_core::model::operation::OperationId(uuid::Uuid::parse_str(op_id_str)?);
        let detail = hist.by_operation(&op_id)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&detail)?);
        } else if let Some(d) = detail {
            println!("Operation Details: {}", d.operation.id);
            println!("─────────────────────────────────────────");
            println!("Kind:       {}", d.operation.kind);
            println!("Actor:      {} ({})", d.actor_name, d.operation.actor_id);
            println!("Source:     {}", d.operation.source);
            println!("Timestamp:  {}", d.operation.created_at);
            if let Some(ref r) = d.operation.reason {
                println!("Reason:     {}", r);
            }
            if let Some(ref k) = d.operation.idempotency_key {
                println!("Idempotency: {}", k);
            }
            println!("\nMovements:");
            for m in &d.movements {
                let loc_name = loc_repo
                    .get_path(&m.location_id)
                    .map(|p| {
                        p.iter()
                            .map(|l| l.name.clone())
                            .collect::<Vec<_>>()
                            .join(" > ")
                    })
                    .unwrap_or_else(|_| m.location_id.to_string());
                let delta_str = if m.delta_qty >= 0 {
                    format!("+{}", m.delta_qty)
                } else {
                    m.delta_qty.to_string()
                };
                println!("  - Item {}: {} at {}", m.item_id, delta_str, loc_name);
            }
        } else {
            println!("Operation '{}' not found", op_id_str);
        }
        return Ok(());
    }

    if let Some(ref item_str) = args.item {
        let it = resolve_item(conn, item_str)?;
        let summaries = hist.by_item(&it.id, args.limit)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&summaries)?);
        } else if summaries.is_empty() {
            println!("No movement history for item '{}'.", it.name);
        } else {
            println!(
                "History for '{}' ({})",
                it.name,
                it.sku.as_deref().unwrap_or("no SKU")
            );
            println!(
                "{:<20} {:<10} {:>10} {:<24} {:<16}",
                "Timestamp", "Kind", "Delta", "Location", "Reason"
            );
            println!("{}", "─".repeat(84));
            for s in &summaries {
                for m in &s.movements {
                    if m.item_id == it.id {
                        let loc_name = loc_repo
                            .get_path(&m.location_id)
                            .map(|p| {
                                p.iter()
                                    .map(|l| l.name.clone())
                                    .collect::<Vec<_>>()
                                    .join(" > ")
                            })
                            .unwrap_or_else(|_| m.location_id.to_string());
                        let delta_str = if m.delta_qty >= 0 {
                            format!("+{}", m.delta_qty)
                        } else {
                            m.delta_qty.to_string()
                        };
                        let ts = s
                            .operation
                            .created_at
                            .format(&time::format_description::well_known::Rfc3339)
                            .unwrap_or_default();
                        println!(
                            "{:<20} {:<10} {:>10} {:<24} {:<16}",
                            &ts[..19.min(ts.len())],
                            s.operation.kind,
                            delta_str,
                            loc_name,
                            s.operation.reason.as_deref().unwrap_or("-")
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    if let Some(ref loc_str) = args.location {
        let loc = resolve_location(conn, loc_str)?;
        let summaries = hist.by_location(&loc.id, args.limit)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&summaries)?);
        } else if summaries.is_empty() {
            println!("No movement history for location '{}'.", loc.name);
        } else {
            println!("History for Location '{}'", loc.name);
            println!(
                "{:<20} {:<10} {:>10} {:<36} {:<16}",
                "Timestamp", "Kind", "Delta", "Item ID", "Reason"
            );
            println!("{}", "─".repeat(96));
            for s in &summaries {
                for m in &s.movements {
                    if m.location_id == loc.id {
                        let delta_str = if m.delta_qty >= 0 {
                            format!("+{}", m.delta_qty)
                        } else {
                            m.delta_qty.to_string()
                        };
                        let ts = s
                            .operation
                            .created_at
                            .format(&time::format_description::well_known::Rfc3339)
                            .unwrap_or_default();
                        println!(
                            "{:<20} {:<10} {:>10} {:<36} {:<16}",
                            &ts[..19.min(ts.len())],
                            s.operation.kind,
                            delta_str,
                            m.item_id,
                            s.operation.reason.as_deref().unwrap_or("-")
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    bail!("Please specify --item <SKU>, --location <LOC>, or --op <OP_ID> to query history");
}
