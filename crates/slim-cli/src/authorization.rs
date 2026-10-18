//! Authorization at the executable boundary. Identity overrides never grant permissions.
use crate::cli::*;
use crate::commands::resolve::resolve_actor;
use rusqlite::Connection;
use slim_core::{model::actor::Role, service::ActorRepository};
use slim_db::repo::actor::SqliteActorRepo;

pub fn check(
    conn: &Connection,
    command: &Commands,
    actor: Option<&str>,
    configured: Option<&str>,
) -> anyhow::Result<()> {
    let required = match command {
        Commands::Actor(args) => match &args.command {
            ActorCommands::List => None,
            ActorCommands::Add { .. } if SqliteActorRepo::new(conn).list()?.is_empty() => None,
            _ => Some(Role::Admin),
        },
        Commands::Item(a) => match a.command {
            ItemCommands::Add { .. } => Some(Role::Manager),
            ItemCommands::Archive { .. }
            | ItemCommands::Restore { .. }
            | ItemCommands::Purge { .. } => Some(Role::Admin),
            _ => None,
        },
        Commands::Location(a) => match a.command {
            LocationCommands::Add { .. } => Some(Role::Manager),
            _ => None,
        },
        Commands::Identifier(a) => match a.command {
            IdentifierCommands::Add { .. } => Some(Role::Manager),
            _ => None,
        },
        Commands::Grid(a) => match a.command {
            GridCommands::Create { .. } => Some(Role::Manager),
            _ => None,
        },
        Commands::Bin(a) => match a.command {
            BinCommands::List | BinCommands::Show { .. } => None,
            _ => Some(Role::Manager),
        },
        Commands::Stock(a) => match a.command {
            StockCommands::Threshold { .. } => Some(Role::Manager),
            _ => None,
        },
        Commands::Db(a) => match &a.command {
            DbCommands::Reconcile(r) if r.repair => Some(Role::Admin),
            _ => None,
        },
        Commands::Device(a) => match a.command {
            DeviceCommands::List => None,
            DeviceCommands::Remove(_) => Some(Role::Admin),
            _ => Some(Role::Manager),
        },
        Commands::Observation(a) => match a.command {
            ObservationCommands::List(_) => None,
            _ => Some(Role::Operator),
        },
        Commands::Automation(a) | Commands::Hook(a) => match &a.command {
            AutomationCommands::History(_) | AutomationCommands::Pending(_) => None,
            AutomationCommands::Rules(r) if matches!(r.command, RulesCommands::List) => None,
            _ => Some(Role::Admin),
        },
        _ => None,
    };
    if let Some(role) = required {
        let current = resolve_actor(conn, actor, configured)?;
        anyhow::ensure!(
            current.role >= role,
            "Role '{}' requires '{}' permission for this action",
            current.role,
            role
        );
    }
    Ok(())
}
