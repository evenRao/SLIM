//! Identifier subcommands (add, resolve).

use anyhow::{Result, bail};
use rusqlite::Connection;
use slim_core::model::identifier::{IdentifierKind, IdentifierTarget, NewIdentifier};
use slim_core::service::{IdentifierRepository, ItemRepository, LocationRepository};
use slim_db::repo::identifier::SqliteIdentifierRepo;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::{resolve_item, resolve_location};
use crate::cli::IdentifierCommands;

pub fn handle_identifier(conn: &Connection, cmd: IdentifierCommands, json: bool) -> Result<()> {
    let repo = SqliteIdentifierRepo::new(conn);

    match cmd {
        IdentifierCommands::Add {
            kind,
            value,
            item,
            location,
        } => {
            let ident_kind: IdentifierKind = kind.parse()?;

            let target = match (item, location) {
                (Some(it_name), None) => {
                    let it = resolve_item(conn, &it_name)?;
                    IdentifierTarget::Item(it.id)
                }
                (None, Some(loc_name)) => {
                    let loc = resolve_location(conn, &loc_name)?;
                    IdentifierTarget::Location(loc.id)
                }
                (Some(_), Some(_)) => bail!("Specify either --item or --location, not both"),
                (None, None) => {
                    bail!("Specify --item or --location to associate this identifier with")
                }
            };

            let ident = repo.create(&NewIdentifier {
                kind: ident_kind,
                value,
                target,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&ident)?);
            } else {
                println!("Associated [{}] '{}' with target", ident.kind, ident.value);
            }
        }
        IdentifierCommands::Resolve { kind, value } => {
            let ident_kind: IdentifierKind = kind.parse()?;
            let target = repo.resolve(&ident_kind, &value)?;

            match target {
                Some(IdentifierTarget::Item(item_id)) => {
                    let it_repo = SqliteItemRepo::new(conn);
                    let it = it_repo.get_by_id(&item_id)?;
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "type": "item",
                                "target_id": item_id.to_string(),
                                "item": it,
                            })
                        );
                    } else if let Some(item) = it {
                        println!(
                            "Resolved to Item: {} (SKU: {})",
                            item.name,
                            item.sku.as_deref().unwrap_or("-")
                        );
                    } else {
                        println!(
                            "Resolved to Item ID: {} (item not found in database)",
                            item_id
                        );
                    }
                }
                Some(IdentifierTarget::Location(loc_id)) => {
                    let loc_repo = SqliteLocationRepo::new(conn);
                    let loc = loc_repo.get_by_id(&loc_id)?;
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "type": "location",
                                "target_id": loc_id.to_string(),
                                "location": loc,
                            })
                        );
                    } else if let Some(l) = loc {
                        let path = loc_repo.get_path(&loc_id).unwrap_or_default();
                        let path_str = path
                            .iter()
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>()
                            .join(" > ");
                        println!("Resolved to Location: {} [{}]", path_str, l.kind);
                    } else {
                        println!("Resolved to Location ID: {}", loc_id);
                    }
                }
                None => {
                    if json {
                        println!("{}", serde_json::json!({ "resolved": false }));
                    } else {
                        println!("No entity found matching [{}] '{}'", ident_kind, value);
                    }
                }
            }
        }
    }
    Ok(())
}
