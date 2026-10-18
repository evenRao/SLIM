//! Find subcommand handler (unified search across items, identifiers, locations).

use anyhow::Result;
use rusqlite::Connection;
use slim_core::model::identifier::IdentifierKind;
use slim_core::model::item::ItemFilter;
use slim_core::service::{
    IdentifierRepository, InventoryService, ItemRepository, LocationRepository,
};
use slim_db::repo::identifier::SqliteIdentifierRepo;
use slim_db::repo::inventory::SqliteInventoryService;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;

use crate::cli::FindArgs;

pub fn handle_find(conn: &Connection, args: FindArgs, json: bool) -> Result<()> {
    let item_repo = SqliteItemRepo::new(conn);
    let id_repo = SqliteIdentifierRepo::new(conn);
    let inv_svc = SqliteInventoryService::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);

    let query = args.query.trim();

    // 1. Search items by SKU / name
    let items = item_repo.list(&ItemFilter {
        search: Some(query.to_string()),
        include_archived: false,
    })?;

    // 2. Search identifiers
    let mut identifier_matched_items = Vec::new();
    for kind in [
        IdentifierKind::Barcode,
        IdentifierKind::Qr,
        IdentifierKind::Nfc,
        IdentifierKind::Rfid,
        IdentifierKind::Manual,
    ] {
        if let Some(slim_core::model::identifier::IdentifierTarget::Item(iid)) =
            id_repo.resolve(&kind, query)?
            && let Some(it) = item_repo.get_by_id(&iid)?
            && !items.iter().any(|existing| existing.id == it.id)
        {
            identifier_matched_items.push(it);
        }
    }

    let mut all_matched = items;
    all_matched.extend(identifier_matched_items);

    if json {
        let mut results = Vec::new();
        for it in &all_matched {
            let balances = inv_svc.get_item_balances(&it.id)?;
            results.push(serde_json::json!({
                "item": it,
                "balances": balances,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if all_matched.is_empty() {
        println!("No items found matching '{}'", query);
    } else {
        println!("Found {} item(s) matching '{}':", all_matched.len(), query);
        for it in &all_matched {
            println!(
                "\n▶ {} (SKU: {})",
                it.name,
                it.sku.as_deref().unwrap_or("none")
            );
            if let Some(ref desc) = it.description {
                println!("  Description: {}", desc);
            }
            let balances = inv_svc.get_item_balances(&it.id)?;
            if balances.is_empty() {
                println!("  Stock: 0 {} on hand", it.unit);
            } else {
                let total: u64 = balances.iter().map(|b| u64::from(b.qty)).sum();
                println!("  Total stock: {} {}", total, it.unit);
                for b in &balances {
                    let path = loc_repo
                        .get_path(&b.location_id)
                        .map(|p| {
                            p.iter()
                                .map(|l| l.name.clone())
                                .collect::<Vec<_>>()
                                .join(" > ")
                        })
                        .unwrap_or_else(|_| b.location_id.to_string());
                    println!("    - {}: {} {}", path, b.qty, it.unit);
                }
            }
        }
    }

    // 3. If --locate flag was passed, illuminate dock LEDs for the found item/bin
    if args.locate && !all_matched.is_empty() {
        let led_repo = slim_db::repo::led::SqliteLedRepo::new(conn);
        let now = time::OffsetDateTime::now_utc();
        match led_repo.locate_target(query, args.ttl, now) {
            Ok(states) => {
                if !json {
                    let duration = args
                        .ttl
                        .unwrap_or(slim_core::model::led::DEFAULT_LOCATE_TTL_SECS);
                    println!(
                        "\n💡 Physical dock LEDs illuminated for '{}' (TTL: {}s):",
                        query, duration
                    );
                    for st in &states {
                        println!("  - Dock {} [RGB: {:?}]", st.dock_id, st.rgb());
                    }
                }
            }
            Err(e) => {
                if !json {
                    println!("\n⚠️ Could not illuminate dock LEDs: {}", e);
                }
            }
        }
    }

    Ok(())
}
