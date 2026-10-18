//! Database subcommands (status, reconcile, backup).

use anyhow::{Result, bail};
use rusqlite::Connection;
use slim_core::service::ReconciliationService;
use slim_db::repo::inventory::SqliteReconciliationService;
use std::path::Path;

use super::resolve::resolve_actor;
use crate::cli::{DbCommands, ReconcileArgs};

pub fn handle_db(
    conn: &Connection,
    db_path: &Path,
    cmd: DbCommands,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
    json: bool,
) -> Result<()> {
    match cmd {
        DbCommands::Status => {
            let version = slim_db::migrate::current_version(conn)?;
            let up_to_date = slim_db::migrate::is_up_to_date(conn)?;
            let latest = slim_db::migrate::latest_version();

            let total_items: i64 =
                conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
            let total_locations: i64 =
                conn.query_row("SELECT COUNT(*) FROM locations", [], |r| r.get(0))?;
            let total_movements: i64 =
                conn.query_row("SELECT COUNT(*) FROM stock_movements", [], |r| r.get(0))?;
            let total_balances: i64 = conn.query_row(
                "SELECT COUNT(*) FROM stock_balances WHERE qty > 0",
                [],
                |r| r.get(0),
            )?;

            if json {
                let status_json = serde_json::json!({
                    "db_path": db_path.display().to_string(),
                    "schema_version": version,
                    "latest_version": latest,
                    "up_to_date": up_to_date,
                    "counts": {
                        "items": total_items,
                        "locations": total_locations,
                        "movements": total_movements,
                        "active_balances": total_balances,
                    }
                });
                println!("{}", serde_json::to_string_pretty(&status_json)?);
            } else {
                println!("SLIM Database Status");
                println!("─────────────────────────────────────────");
                println!("Database path:      {}", db_path.display());
                println!("Schema version:     {} (latest: {})", version, latest);
                println!(
                    "Status:             {}",
                    if up_to_date {
                        "Up to date"
                    } else {
                        "Pending migrations"
                    }
                );
                println!("Total items:        {}", total_items);
                println!("Total locations:    {}", total_locations);
                println!("Stock movements:    {}", total_movements);
                println!("Active balances:    {}", total_balances);
            }
        }
        DbCommands::Reconcile(ReconcileArgs { repair, reason }) => {
            let recon = SqliteReconciliationService::new(conn);

            if repair {
                let reason_str = match reason {
                    Some(r) if !r.trim().is_empty() => r,
                    _ => bail!(
                        "A mandatory --reason must be provided when repairing database reconciliation"
                    ),
                };

                let actor = resolve_actor(conn, cli_actor, config_actor)?;
                let report = recon.repair(&actor.id, &reason_str)?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!("Reconciliation repair complete.");
                    println!("Repaired by:  {}", actor.display_name);
                    println!("Reason:       {}", reason_str);
                    println!("Total balances verified: {}", report.total_balances);
                    println!("Remaining mismatches:    {}", report.mismatches.len());
                }
            } else {
                let report = recon.check()?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else if report.mismatches.is_empty() {
                    println!(
                        "Reconciliation OK: All {} cached balance(s) match the movement ledger.",
                        report.total_balances
                    );
                } else {
                    eprintln!(
                        "WARNING: Found {} balance mismatch(es)!",
                        report.mismatches.len()
                    );
                    eprintln!(
                        "{:<36} {:<36} {:>10} {:>10}",
                        "Item ID", "Location ID", "Cached", "Ledger"
                    );
                    eprintln!("{}", "─".repeat(96));
                    for m in &report.mismatches {
                        eprintln!(
                            "{:<36} {:<36} {:>10} {:>10}",
                            m.item_id, m.location_id, m.cached_qty, m.ledger_qty
                        );
                    }
                    eprintln!(
                        "\nRun 'slim db reconcile --repair --reason \"<REASON>\"' to rebuild the cache from the ledger."
                    );
                }
            }
        }
        DbCommands::Backup(args) => {
            if let Some(parent) = args.destination.parent()
                && !parent.exists()
            {
                std::fs::create_dir_all(parent)?;
            }

            let mut dst = Connection::open(&args.destination)?;
            let backup = rusqlite::backup::Backup::new(conn, &mut dst)?;
            backup.run_to_completion(100, std::time::Duration::from_millis(5), None)?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "backup_path": args.destination.display().to_string(),
                        "source": db_path.display().to_string(),
                        "status": "success"
                    })
                );
            } else {
                println!(
                    "Database successfully backed up to: {}",
                    args.destination.display()
                );
            }
        }
    }
    Ok(())
}
