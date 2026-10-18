//! Actor subcommands (add, list, set-role, set-pin, set-badge).

use anyhow::{Context, Result};
use rusqlite::Connection;
use slim_core::model::actor::{ActorUpdate, NewActor, Role};
use slim_core::service::{ActorRepository, AuthService};
use slim_db::repo::actor::SqliteActorRepo;
use slim_db::repo::auth::SqliteAuthService;

use crate::cli::ActorCommands;

pub fn handle_actor(
    conn: &Connection,
    cmd: ActorCommands,
    json: bool,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
) -> Result<()> {
    let repo = SqliteActorRepo::new(conn);
    let auth = SqliteAuthService::new(conn);
    let admin = || super::resolve::resolve_actor(conn, cli_actor, config_actor);

    match cmd {
        ActorCommands::Add {
            name,
            os_user,
            role,
        } => {
            let parsed_role: Role = match role {
                Some(r) => r.parse().map_err(|e| anyhow::anyhow!("{}", e))?,
                None => Role::Operator,
            };

            let mut new_actor = NewActor::new(name).with_role(parsed_role);
            if let Some(u) = os_user {
                new_actor = new_actor.with_os_username(u);
            }

            let actor = if repo.list()?.is_empty() {
                repo.create(&new_actor)?
            } else {
                repo.create_as_admin(&admin()?.id, &new_actor, None)?
            };

            if json {
                println!("{}", serde_json::to_string_pretty(&actor)?);
            } else {
                println!(
                    "Created actor '{}' (Role: {}, ID: {})",
                    actor.display_name, actor.role, actor.id
                );
                if let Some(ref u) = actor.os_username {
                    println!("  Mapped to OS username: {}", u);
                }
            }
        }
        ActorCommands::List => {
            let actors = repo.list()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&actors)?);
            } else if actors.is_empty() {
                println!("No actors registered. Add one with 'slim actor add <NAME>'.");
            } else {
                println!(
                    "{:<36} {:<20} {:<12} {:<14} Active",
                    "Actor ID", "Display Name", "Role", "OS User"
                );
                println!("{}", "─".repeat(92));
                for a in &actors {
                    println!(
                        "{:<36} {:<20} {:<12} {:<14} {}",
                        a.id,
                        a.display_name,
                        a.role,
                        a.os_username.as_deref().unwrap_or("-"),
                        if a.active { "yes" } else { "no" }
                    );
                }
            }
        }
        ActorCommands::SetRole { name, role } => {
            let actor = repo
                .get_by_name(&name)?
                .or(repo.get_by_os_username(&name)?)
                .context(format!("actor '{}' not found", name))?;

            let new_role: Role = role.parse().map_err(|e| anyhow::anyhow!("{}", e))?;
            repo.update_as_admin(
                &admin()?.id,
                &actor.id,
                &ActorUpdate {
                    role: Some(new_role),
                    ..Default::default()
                },
            )?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "actor_id": actor.id.to_string(),
                        "display_name": actor.display_name,
                        "role": new_role.to_string(),
                    })
                );
            } else {
                println!(
                    "Updated role for actor '{}' to {}",
                    actor.display_name, new_role
                );
            }
        }
        ActorCommands::SetPin { name, pin } => {
            let actor = repo
                .get_by_name(&name)?
                .or(repo.get_by_os_username(&name)?)
                .context(format!("actor '{}' not found", name))?;

            let final_pin = match pin {
                Some(p) => p,
                None => {
                    print!("Enter new PIN for {}: ", actor.display_name);
                    use std::io::{self, Write};
                    io::stdout().flush()?;
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    input.trim().to_string()
                }
            };

            repo.update_as_admin(
                &admin()?.id,
                &actor.id,
                &ActorUpdate {
                    pin: Some(final_pin),
                    ..Default::default()
                },
            )?;

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "actor_id": actor.id.to_string(),
                        "display_name": actor.display_name,
                        "pin_set": true,
                    })
                );
            } else {
                println!(
                    "PIN successfully updated for actor '{}'",
                    actor.display_name
                );
            }
        }
        ActorCommands::Edit { name, new_name } => {
            let actor = repo.get_by_name(&name)?.context("User not found")?;
            let updated = repo.update_as_admin(
                &admin()?.id,
                &actor.id,
                &ActorUpdate {
                    display_name: Some(new_name),
                    ..Default::default()
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&updated)?);
            } else {
                println!("Updated user '{}'", updated.display_name);
            }
        }
        ActorCommands::Purge { name, confirm } => {
            let target = repo.get_by_name(&name)?.context("User not found")?;
            repo.permanently_delete_user(&admin()?.id, &target.id, &confirm)?;
            println!("{}", serde_json::json!({"permanently_deleted":target.id}));
        }
        ActorCommands::Remove { ref name } | ActorCommands::Restore { ref name } => {
            let restore = matches!(&cmd, ActorCommands::Restore { .. });
            let actor = repo.get_by_name(name)?.context("User not found")?;
            repo.update_as_admin(
                &admin()?.id,
                &actor.id,
                &ActorUpdate {
                    active: Some(restore),
                    ..Default::default()
                },
            )?;
            if json {
                println!("{}", serde_json::json!({"id":actor.id,"active":restore}));
            } else {
                println!(
                    "{} user '{}'",
                    if restore { "Reactivated" } else { "Removed" },
                    actor.display_name
                );
            }
        }
        ActorCommands::SetBadge { name, token, clear } => {
            let actor = repo
                .get_by_name(&name)?
                .or(repo.get_by_os_username(&name)?)
                .context(format!("actor '{}' not found", name))?;

            if clear {
                auth.set_badge_token(&actor.id, None)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "actor_id": actor.id.to_string(),
                            "badge_token": null,
                        })
                    );
                } else {
                    println!("Badge token cleared for actor '{}'", actor.display_name);
                }
            } else {
                let badge = token.context("Provide a badge token or pass --clear to remove")?;
                auth.set_badge_token(&actor.id, Some(&badge))?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "actor_id": actor.id.to_string(),
                            "badge_token": badge,
                        })
                    );
                } else {
                    println!(
                        "Assigned badge '{}' to actor '{}'",
                        badge, actor.display_name
                    );
                }
            }
        }
    }
    Ok(())
}
