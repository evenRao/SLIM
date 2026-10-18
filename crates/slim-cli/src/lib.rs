#![deny(clippy::all)]

//! SLIM — System for Local Inventory Management
//! Command-line interface entry point.

mod authorization;
mod cli;
mod commands;
mod config;
mod desktop;

use clap::Parser;
use cli::{Cli, Commands};
use config::Config;
use std::process::ExitCode;

pub fn entry() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("Error: {:#}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

pub fn run() -> anyhow::Result<()> {
    run_with_default(false)
}

/// The graphical executable explicitly selects the browser application.
pub fn run_app() -> anyhow::Result<()> {
    run_with_default(true)
}

fn run_with_default(browser: bool) -> anyhow::Result<()> {
    let args = Cli::parse();

    // Initialize tracing subscriber for diagnostic logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cfg = Config::load(args.config.as_deref())?;
    let db_path = cfg.resolve_db_path(args.db.as_deref());

    let command = args.command.unwrap_or_else(|| {
        if browser {
            Commands::App(cli::AppArgs::default())
        } else {
            Commands::Tui
        }
    });
    if let Commands::App(ref app_args) = command {
        return desktop::run(&db_path, &cfg, app_args);
    }

    // Init does not require an existing database file
    if let Commands::Init(init_args) = command {
        let path = init_args.db.unwrap_or(db_path);
        return commands::init::handle_init(&path);
    }

    // Check if database exists for all other commands
    if !db_path.exists() && !matches!(command, Commands::Tui) {
        anyhow::bail!(
            "Database file not found at '{}'. Run 'slim init' to create it.",
            db_path.display()
        );
    }

    let existed = db_path.exists();
    if !existed && let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let conn = slim_db::connection::open_connection(&db_path)?;
    if !slim_db::migrate::is_up_to_date(&conn)? {
        let backup_path = db_path.with_extension(format!(
            "before-v{}.sqlite3",
            slim_db::migrate::latest_version()
        ));
        if existed && !backup_path.exists() {
            conn.backup(rusqlite::DatabaseName::Main, &backup_path, None)?;
        }
        slim_db::migrate::run_migrations(&conn)?;
    }

    let cli_actor = args.actor.as_deref();
    let config_actor = cfg.default_actor.as_deref();
    let json = args.json;

    authorization::check(&conn, &command, cli_actor, config_actor)?;
    match command {
        Commands::App(_) => unreachable!(),
        Commands::Init(_) => unreachable!(),
        Commands::Tui => {
            let actor = commands::resolve::resolve_actor(&conn, cli_actor, config_actor).ok();
            slim_tui::run_with_login(&conn, actor)?;
        }
        Commands::Db(db_args) => commands::db::handle_db(
            &conn,
            &db_path,
            db_args.command,
            cli_actor,
            config_actor,
            json,
        )?,
        Commands::Actor(actor_args) => {
            commands::actor::handle_actor(&conn, actor_args.command, json, cli_actor, config_actor)?
        }
        Commands::Item(item_args) => {
            commands::item::handle_item(&conn, item_args.command, json, cli_actor, config_actor)?
        }
        Commands::Location(loc_args) => {
            commands::location::handle_location(&conn, loc_args.command, json)?
        }
        Commands::Catalog(catalog) => {
            use slim_db::repo::catalog::{CatalogKind, SqliteCatalogService};
            let actor = commands::resolve::resolve_actor(&conn, cli_actor, config_actor)?;
            let kind: CatalogKind = serde_json::from_value(serde_json::json!(catalog.kind))?;
            let service = SqliteCatalogService::new(&conn);
            let result = match catalog.action.as_str() {
                "list" => serde_json::json!(service.list(&actor.id, kind, catalog.deleted)?),
                "add" | "edit" => {
                    let mut data = serde_json::json!({});
                    for field in catalog.fields {
                        let (key, value) = field
                            .split_once('=')
                            .ok_or_else(|| anyhow::anyhow!("Use --field key=value"))?;
                        data[key] = serde_json::json!(value);
                    }
                    let id = if catalog.action == "edit" {
                        Some(
                            catalog
                                .id
                                .as_deref()
                                .ok_or_else(|| anyhow::anyhow!("--id is required"))?,
                        )
                    } else {
                        None
                    };
                    serde_json::json!(service.save(&actor.id, kind, id, &data)?)
                }
                action => {
                    service.lifecycle(
                        &actor.id,
                        kind,
                        catalog
                            .id
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("--id is required"))?,
                        action,
                        &catalog.confirm,
                    )?;
                    serde_json::json!({"ok":true})
                }
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Identifier(id_args) => {
            commands::identifier::handle_identifier(&conn, id_args.command, json)?
        }
        Commands::Stock(stock_args) => {
            commands::stock::handle_stock(&conn, stock_args.command, cli_actor, config_actor, json)?
        }
        Commands::History(hist_args) => commands::history::handle_history(&conn, hist_args, json)?,
        Commands::Find(find_args) => commands::find::handle_find(&conn, find_args, json)?,
        Commands::Grid(grid_args) => commands::grid::handle_grid(&conn, grid_args.command, json)?,
        Commands::Bin(bin_args) => commands::bin::handle_bin(&conn, bin_args.command, json)?,
        Commands::Label(label_args) => {
            commands::label::handle_label(&conn, label_args.command, json)?
        }
        Commands::Scan(scan_args) => {
            commands::scan::handle_scan(&conn, scan_args.command, cli_actor, config_actor, json)?
        }
        Commands::Session(session_args) => {
            commands::session::handle_session(&conn, session_args.command, json)?
        }
        Commands::Audit(audit_args) => {
            commands::audit::handle_audit(&conn, audit_args.command, json)?
        }
        Commands::Automation(auto_args) | Commands::Hook(auto_args) => {
            if let cli::AutomationCommands::Rules(ref rules) = auto_args.command
                && let cli::RulesCommands::Remove(ref remove) = rules.command
            {
                let actor = commands::resolve::resolve_actor(&conn, cli_actor, config_actor)?;
                slim_db::repo::catalog::SqliteCatalogService::new(&conn).lifecycle(
                    &actor.id,
                    slim_db::repo::catalog::CatalogKind::Rule,
                    &remove.id,
                    "delete",
                    "DELETE",
                )?;
                println!("Rule removed from active use; moved to Deleted");
            } else {
                commands::automation::handle_automation(&conn, &cfg, auto_args, json)?
            }
        }
        Commands::Device(dev_args) => {
            if let cli::DeviceCommands::Remove(ref remove) = dev_args.command {
                let actor = commands::resolve::resolve_actor(&conn, cli_actor, config_actor)?;
                slim_db::repo::catalog::SqliteCatalogService::new(&conn).lifecycle(
                    &actor.id,
                    slim_db::repo::catalog::CatalogKind::Device,
                    &remove.id,
                    "delete",
                    "DELETE",
                )?;
                println!("Device unregistered successfully; moved to Deleted");
            } else {
                commands::device::handle_device(&conn, dev_args.command, json)?
            }
        }
        Commands::Observation(obs_args) => commands::device::handle_observation(
            &conn,
            obs_args.command,
            cli_actor,
            config_actor,
            json,
        )?,
        Commands::Led(led_args) => commands::led::handle_led(&conn, led_args.command, json)?,
    }

    Ok(())
}
