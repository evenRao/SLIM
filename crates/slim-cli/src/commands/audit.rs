//! Audit subcommands (list).

use anyhow::Result;
use rusqlite::Connection;
use slim_core::model::auth::AuditFilter;
use slim_core::service::{ActorRepository, AuditRepository};
use slim_db::repo::actor::SqliteActorRepo;
use slim_db::repo::audit::SqliteAuditRepo;

use crate::cli::AuditCommands;

pub fn handle_audit(conn: &Connection, cmd: AuditCommands, json: bool) -> Result<()> {
    let audit_repo = SqliteAuditRepo::new(conn);
    let actor_repo = SqliteActorRepo::new(conn);

    match cmd {
        AuditCommands::List {
            actor,
            action,
            limit,
        } => {
            let actor_id = if let Some(ref name) = actor {
                let resolved = actor_repo
                    .get_by_name(name)?
                    .or(actor_repo.get_by_os_username(name)?);
                match resolved {
                    Some(a) => Some(a.id),
                    None => anyhow::bail!("actor '{}' not found", name),
                }
            } else {
                None
            };

            let filter = AuditFilter {
                actor_id,
                action,
                limit: Some(limit),
            };

            let events = audit_repo.list(&filter)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&events)?);
            } else if events.is_empty() {
                println!("No audit events recorded matching filter.");
            } else {
                println!(
                    "{:<22} {:<24} {:<24} Details",
                    "Timestamp", "Actor", "Action"
                );
                println!("{}", "─".repeat(95));
                for ev in &events {
                    let actor_name = match ev.actor_id {
                        Some(aid) => actor_repo
                            .get_by_id(&aid)?
                            .map(|a| a.display_name)
                            .unwrap_or_else(|| aid.to_string()),
                        None => "System".to_string(),
                    };

                    let ts = ev
                        .created_at
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_default();
                    let date_short = if ts.len() >= 19 { &ts[..19] } else { &ts };

                    println!(
                        "{:<22} {:<24} {:<24} {}",
                        date_short,
                        actor_name,
                        ev.action,
                        ev.details.as_deref().unwrap_or("-")
                    );
                }
            }
        }
    }

    Ok(())
}
