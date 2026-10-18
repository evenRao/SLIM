//! Item subcommands (add, list, show, archive).

use anyhow::Result;
use rusqlite::Connection;
use slim_core::model::item::{ItemFilter, NewItem};
use slim_core::service::{
    IdentifierRepository, InventoryService, ItemRepository, LocationRepository,
};
use slim_db::repo::identifier::SqliteIdentifierRepo;
use slim_db::repo::inventory::SqliteInventoryService;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::resolve_item;
use crate::cli::ItemCommands;

pub fn handle_item(
    conn: &Connection,
    cmd: ItemCommands,
    json: bool,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
) -> Result<()> {
    let repo = SqliteItemRepo::new(conn);

    match cmd {
        ItemCommands::Add {
            sku,
            name,
            unit,
            description,
        } => {
            let item = repo.create(&NewItem {
                sku,
                name,
                unit,
                description,
                metadata: None,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&item)?);
            } else {
                println!("Created item '{}' (ID: {})", item.name, item.id);
                if let Some(ref s) = item.sku {
                    println!("  SKU:  {}", s);
                }
                println!("  Unit: {}", item.unit);
            }
        }
        ItemCommands::List { all, search } => {
            let items = repo.list(&ItemFilter {
                search,
                include_archived: all,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else if items.is_empty() {
                println!("No items found.");
            } else {
                println!(
                    "{:<16} {:<32} {:<8} {:<10}",
                    "SKU", "Name", "Unit", "Status"
                );
                println!("{}", "─".repeat(70));
                for it in &items {
                    let status = if it.archived_at.is_some() {
                        "archived"
                    } else {
                        "active"
                    };
                    println!(
                        "{:<16} {:<32} {:<8} {:<10}",
                        it.sku.as_deref().unwrap_or("-"),
                        it.name,
                        it.unit,
                        status
                    );
                }
            }
        }
        ItemCommands::Show { item } => {
            let it = resolve_item(conn, &item)?;
            let inv = SqliteInventoryService::new(conn);
            let balances = inv.get_item_balances(&it.id)?;
            let id_repo = SqliteIdentifierRepo::new(conn);
            let idents = id_repo.list_for_item(&it.id)?;
            let loc_repo = SqliteLocationRepo::new(conn);

            if json {
                let show_json = serde_json::json!({
                    "item": it,
                    "balances": balances,
                    "identifiers": idents,
                });
                println!("{}", serde_json::to_string_pretty(&show_json)?);
            } else {
                println!("Item Details");
                println!("─────────────────────────────────────────");
                println!("ID:          {}", it.id);
                println!("Name:        {}", it.name);
                println!("SKU:         {}", it.sku.as_deref().unwrap_or("-"));
                println!("Unit:        {}", it.unit);
                if let Some(ref d) = it.description {
                    println!("Description: {}", d);
                }
                if let Some(a) = it.archived_at {
                    println!("Status:      ARCHIVED at {}", a);
                } else {
                    println!("Status:      Active");
                }

                if !idents.is_empty() {
                    println!("\nIdentifiers:");
                    for id in &idents {
                        println!("  - [{}] {}", id.kind, id.value);
                    }
                }

                println!("\nStock Balances:");
                if balances.is_empty() {
                    println!("  No stock on hand in any location.");
                } else {
                    let total: u64 = balances.iter().map(|b| u64::from(b.qty)).sum();
                    for b in &balances {
                        let loc_name = loc_repo
                            .get_path(&b.location_id)
                            .map(|path| {
                                path.iter()
                                    .map(|p| p.name.clone())
                                    .collect::<Vec<_>>()
                                    .join(" > ")
                            })
                            .unwrap_or_else(|_| b.location_id.to_string());
                        println!("  - {}: {} {}", loc_name, b.qty, it.unit);
                    }
                    println!("  Total: {} {}", total, it.unit);
                }
            }
        }
        ItemCommands::Archive { item } => {
            let it = resolve_item(conn, &item)?;
            let actor = super::resolve::resolve_actor(conn, cli_actor, config_actor)?;
            repo.delete_as_admin(&it.id, &actor.id)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "archived": it.id.to_string(), "status": "ok" })
                );
            } else {
                println!(
                    "Archived item '{}' ({})",
                    it.name,
                    it.sku.as_deref().unwrap_or("no SKU")
                );
            }
        }
        ItemCommands::Restore { ref item } | ItemCommands::Purge { ref item, .. } => {
            let confirmation = match &cmd {
                ItemCommands::Purge { confirm, .. } => Some(confirm.as_str()),
                _ => None,
            };
            let matches: Vec<_> = repo
                .list(&ItemFilter {
                    search: None,
                    include_archived: true,
                })?
                .into_iter()
                .filter(|i| {
                    i.id.to_string() == *item
                        || i.sku.as_deref() == Some(item.as_str())
                        || i.name.eq_ignore_ascii_case(item)
                })
                .collect();
            anyhow::ensure!(matches.len() == 1, "Specify a unique item SKU or UUID");
            let actor = super::resolve::resolve_actor(conn, cli_actor, config_actor)?;
            if let Some(confirmation) = confirmation {
                repo.permanently_delete_as_admin(&matches[0].id, &actor.id, confirmation)?;
            } else {
                repo.restore_as_admin(&matches[0].id, &actor.id)?;
            }
            if json {
                println!(
                    "{}",
                    if confirmation.is_some() {
                        serde_json::json!({"permanently_deleted":matches[0].id,"status":"ok"})
                    } else {
                        serde_json::json!({"restored":matches[0].id,"status":"ok"})
                    }
                );
            } else {
                println!(
                    "{} '{}'",
                    if confirmation.is_some() {
                        "Permanently deleted"
                    } else {
                        "Restored"
                    },
                    matches[0].name
                );
            }
        }
    }
    Ok(())
}
