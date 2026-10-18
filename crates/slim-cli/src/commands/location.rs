//! Location subcommands (add, list).

use anyhow::Result;
use rusqlite::Connection;
use slim_core::model::location::{Location, LocationKind, NewLocation};
use slim_core::service::LocationRepository;
use slim_db::repo::location::SqliteLocationRepo;

use super::resolve::resolve_location;
use crate::cli::LocationCommands;

pub fn handle_location(conn: &Connection, cmd: LocationCommands, json: bool) -> Result<()> {
    let repo = SqliteLocationRepo::new(conn);

    match cmd {
        LocationCommands::Add { name, kind, parent } => {
            let parent_id = if let Some(ref p_name) = parent {
                let parent_loc = resolve_location(conn, p_name)?;
                Some(parent_loc.id)
            } else {
                None
            };

            let loc_kind: LocationKind = kind.parse()?;

            let loc = repo.create(&NewLocation {
                parent_id,
                name,
                kind: loc_kind,
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&loc)?);
            } else {
                let path = repo.get_path(&loc.id)?;
                let full_path = path
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(" > ");
                println!(
                    "Created location '{}' [{}] (ID: {})",
                    full_path, loc.kind, loc.id
                );
                println!(
                    "  Can hold stock: {}",
                    if loc.can_hold_stock { "yes" } else { "no" }
                );
            }
        }
        LocationCommands::List => {
            let roots = repo.list_children(None)?;

            if json {
                let mut all = Vec::new();
                fn collect_all(
                    repo: &SqliteLocationRepo,
                    parent_id: Option<&slim_core::model::location::LocationId>,
                    out: &mut Vec<Location>,
                ) -> Result<()> {
                    let children = repo.list_children(parent_id)?;
                    for c in children {
                        let cid = c.id;
                        out.push(c);
                        collect_all(repo, Some(&cid), out)?;
                    }
                    Ok(())
                }
                collect_all(&repo, None, &mut all)?;
                println!("{}", serde_json::to_string_pretty(&all)?);
            } else if roots.is_empty() {
                println!(
                    "No locations defined. Add one with 'slim location add --name Workshop --kind room'."
                );
            } else {
                println!("Locations Hierarchy");
                println!("─────────────────────────────────────────");
                fn print_tree(
                    repo: &SqliteLocationRepo,
                    parent_id: Option<&slim_core::model::location::LocationId>,
                    indent: usize,
                ) -> Result<()> {
                    let children = repo.list_children(parent_id)?;
                    for c in children {
                        let prefix = "  ".repeat(indent);
                        let stock_indicator = if c.can_hold_stock { "●" } else { "○" };
                        println!(
                            "{}{} {} [{}] ({})",
                            prefix, stock_indicator, c.name, c.kind, c.id
                        );
                        print_tree(repo, Some(&c.id), indent + 1)?;
                    }
                    Ok(())
                }
                print_tree(&repo, None, 0)?;
                println!("\nLegend: ● Holds stock  ○ Structural only");
            }
        }
    }
    Ok(())
}
