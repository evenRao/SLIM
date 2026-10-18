//! Automation and hook engine CLI commands.

use anyhow::Context;
use rusqlite::Connection;
use slim_core::model::automation::*;
use slim_core::service::{AutomationRepository, OutboxRepository};
use slim_db::repo::automation::{AutomationEngine, AutomationOptions, SqliteAutomationRepo};
use slim_db::repo::outbox::SqliteOutboxRepo;
use std::fs;
use std::thread::sleep;
use std::time::Duration;

use crate::cli::*;
use crate::config::Config;

/// Dispatch automation subcommands.
pub fn handle_automation(
    conn: &Connection,
    config: &Config,
    args: AutomationArgs,
    json: bool,
) -> anyhow::Result<()> {
    match args.command {
        AutomationCommands::Worker(worker_args) => handle_worker(conn, config, worker_args, json),
        AutomationCommands::Rules(rules_args) => handle_rules(conn, config, rules_args, json),
        AutomationCommands::History(hist_args) => handle_history(conn, hist_args, json),
        AutomationCommands::Pending(pending_args) => handle_pending(conn, pending_args, json),
        AutomationCommands::Test(test_args) => handle_test(conn, config, test_args, json),
    }
}

/// Helper to aggregate rules from both the SQLite database and config files.
pub fn load_all_rules(conn: &Connection, config: &Config) -> Vec<AutomationRule> {
    let repo = SqliteAutomationRepo::new(conn);
    let mut rules = repo.list_rules().unwrap_or_default();

    for file_rule in config.load_file_rules() {
        let deleted = conn
            .query_row(
                "SELECT archived_at IS NOT NULL FROM automation_rules WHERE id=?1",
                [&file_rule.id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false);
        if deleted {
            continue;
        }
        if !rules.iter().any(|r| r.id == file_rule.id) {
            rules.push(file_rule);
        }
    }

    rules
}

fn handle_worker(
    conn: &Connection,
    config: &Config,
    args: WorkerArgs,
    json: bool,
) -> anyhow::Result<()> {
    let allow_commands = args.allow_commands || config.allow_commands.unwrap_or(false);
    let command_allowlist = config.command_allowlist.clone().unwrap_or_default();

    let options = AutomationOptions {
        allow_commands,
        command_allowlist,
        max_attempts: 5,
    };

    let engine = AutomationEngine::new(conn);
    let outbox_repo = SqliteOutboxRepo::new(conn);

    if args.once {
        let rules = load_all_rules(conn, config);
        let runs = engine.process_pending(&rules, &options, args.limit)?;

        if json {
            println!("{}", serde_json::to_string_pretty(&runs)?);
        } else {
            let pending_count = outbox_repo.pending_count()?;
            println!(
                "Processed {} event(s). {} action(s) executed. {} pending event(s) remaining.",
                runs.len(),
                runs.iter()
                    .filter(|r| r.status == AutomationRunStatus::Success)
                    .count(),
                pending_count
            );
            for run in &runs {
                println!(
                    "  [{}] rule '{}' ({}) -> {}",
                    run.status,
                    run.rule_id,
                    run.action_type,
                    run.output
                        .as_deref()
                        .or(run.error_message.as_deref())
                        .unwrap_or("")
                );
            }
        }
        return Ok(());
    }

    // Continuous polling mode
    let rules = load_all_rules(conn, config);
    println!(
        "SLIM Automation Worker running. Polling every {}s. Loaded {} rule(s).",
        args.poll_interval,
        rules.len()
    );
    if allow_commands {
        println!(
            "  Command execution: ENABLED (allowlist: {:?})",
            options.command_allowlist
        );
    } else {
        println!("  Command execution: DISABLED (use --allow-commands to enable)");
    }
    println!("Press Ctrl+C to stop.\n");

    loop {
        let current_rules = load_all_rules(conn, config);
        let runs = engine.process_pending(&current_rules, &options, args.limit)?;

        for run in runs {
            let time_str = run
                .executed_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| run.executed_at.to_string());

            println!(
                "{} [{}] Rule '{}' on event '{}' -> {}: {}",
                time_str,
                run.status,
                run.rule_id,
                run.event_type,
                run.action_type,
                run.output
                    .as_deref()
                    .or(run.error_message.as_deref())
                    .unwrap_or("")
            );
        }

        sleep(Duration::from_secs(args.poll_interval));
    }
}

fn handle_rules(
    conn: &Connection,
    config: &Config,
    args: RulesArgs,
    json: bool,
) -> anyhow::Result<()> {
    let repo = SqliteAutomationRepo::new(conn);

    match args.command {
        RulesCommands::List => {
            let db_rules = repo.list_rules()?;
            let file_rules = config.load_file_rules();

            #[derive(serde::Serialize)]
            struct RuleListing {
                #[serde(flatten)]
                rule: AutomationRule,
                source: &'static str,
            }

            let mut all: Vec<RuleListing> = Vec::new();
            for r in db_rules {
                all.push(RuleListing {
                    rule: r,
                    source: "database",
                });
            }
            for r in file_rules {
                if !all.iter().any(|item| item.rule.id == r.id) {
                    all.push(RuleListing {
                        rule: r,
                        source: "config_file",
                    });
                }
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
                return Ok(());
            }

            if all.is_empty() {
                println!("No automation rules found.");
                return Ok(());
            }

            println!(
                "{:<16} {:<22} {:<12} {:<10} {:<10} {:<10}",
                "ID", "EVENT", "SKU", "ACTION", "ENABLED", "SOURCE"
            );
            println!("{}", "-".repeat(84));
            for item in &all {
                let sku = item.rule.item_sku.as_deref().unwrap_or("-");
                println!(
                    "{:<16} {:<22} {:<12} {:<10} {:<10} {:<10}",
                    item.rule.id,
                    item.rule.event,
                    sku,
                    item.rule.action.action_type_str(),
                    if item.rule.enabled { "yes" } else { "no" },
                    item.source
                );
            }
        }

        RulesCommands::Add(add_args) => {
            let action = match add_args.action_type.to_ascii_lowercase().as_str() {
                "notify" => RuleAction::Notify {
                    message: add_args.message.context(
                        "A notification message (--message) is required for 'notify' action",
                    )?,
                },
                "command" => RuleAction::Command {
                    command: add_args.command.context(
                        "A command executable (--command) is required for 'command' action",
                    )?,
                    args: add_args.args.unwrap_or_default(),
                },
                "webhook" => RuleAction::Webhook {
                    url: add_args
                        .url
                        .context("A URL (--url) is required for 'webhook' action")?,
                    payload: add_args.message,
                },
                other => anyhow::bail!(
                    "Unsupported action type '{}'. Use notify, command, or webhook.",
                    other
                ),
            };

            let condition = if add_args.quantity_below.is_some() {
                Some(RuleCondition {
                    quantity_below: add_args.quantity_below,
                    ..Default::default()
                })
            } else {
                None
            };

            let rule = AutomationRule {
                id: add_args.id,
                name: add_args.name,
                enabled: true,
                event: add_args.event,
                item_sku: add_args.sku,
                location_name: add_args.location,
                condition,
                action,
            };

            repo.save_rule(&rule)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&rule)?);
            } else {
                println!("Automation rule '{}' saved successfully.", rule.id);
            }
        }

        RulesCommands::Remove(rm_args) => {
            repo.delete_rule(&rm_args.id)?;
            if json {
                println!(r#"{{"deleted": "{}"}}"#, rm_args.id);
            } else {
                println!("Automation rule '{}' removed.", rm_args.id);
            }
        }

        RulesCommands::Import(import_args) => {
            let content = fs::read_to_string(&import_args.file)
                .with_context(|| format!("Failed to read file '{}'", import_args.file.display()))?;

            #[derive(serde::Deserialize)]
            struct RulesWrapper {
                #[serde(default)]
                rules: Vec<AutomationRule>,
            }

            let parsed: RulesWrapper = toml::from_str(&content).with_context(|| {
                format!("Failed to parse TOML in '{}'", import_args.file.display())
            })?;

            let mut count = 0;
            for rule in &parsed.rules {
                repo.save_rule(rule)?;
                count += 1;
            }

            if json {
                println!(r#"{{"imported_count": {}}}"#, count);
            } else {
                println!(
                    "Successfully imported {} rule(s) from '{}'.",
                    count,
                    import_args.file.display()
                );
            }
        }
    }

    Ok(())
}

fn handle_history(
    conn: &Connection,
    args: AutomationHistoryArgs,
    json: bool,
) -> anyhow::Result<()> {
    let repo = SqliteAutomationRepo::new(conn);
    let runs = repo.list_runs(args.limit)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&runs)?);
        return Ok(());
    }

    if runs.is_empty() {
        println!("No automation execution history found.");
        return Ok(());
    }

    println!(
        "{:<25} {:<16} {:<20} {:<10} {:<10} OUTPUT / ERROR",
        "EXECUTED AT", "RULE ID", "EVENT", "ACTION", "STATUS"
    );
    println!("{}", "-".repeat(95));
    for run in &runs {
        let time_str = run
            .executed_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| run.executed_at.to_string());

        let detail = run
            .output
            .as_deref()
            .or(run.error_message.as_deref())
            .unwrap_or("-");

        println!(
            "{:<25} {:<16} {:<20} {:<10} {:<10} {}",
            time_str, run.rule_id, run.event_type, run.action_type, run.status, detail
        );
    }

    Ok(())
}

fn handle_pending(conn: &Connection, args: PendingArgs, json: bool) -> anyhow::Result<()> {
    let outbox = SqliteOutboxRepo::new(conn);
    let pending = outbox.fetch_pending(args.limit)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&pending)?);
        return Ok(());
    }

    if pending.is_empty() {
        println!("No pending events in outbox. All events delivered.");
        return Ok(());
    }

    println!(
        "{:<38} {:<22} {:<10} {:<25} PAYLOAD",
        "EVENT ID", "EVENT TYPE", "ATTEMPTS", "CREATED AT"
    );

    println!("{}", "-".repeat(110));
    for event in &pending {
        let time_str = event
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| event.created_at.to_string());

        println!(
            "{:<38} {:<22} {:<10} {:<25} {}",
            event.id.to_string(),
            event.event_type,
            event.attempt_count,
            time_str,
            event.payload
        );
    }

    Ok(())
}

fn handle_test(
    conn: &Connection,
    config: &Config,
    args: TestRuleArgs,
    json: bool,
) -> anyhow::Result<()> {
    let rules = load_all_rules(conn, config);
    let rule = rules
        .into_iter()
        .find(|r| r.id == args.rule_id)
        .with_context(|| {
            format!(
                "Automation rule '{}' not found in database or config",
                args.rule_id
            )
        })?;

    let event_type = args.event.unwrap_or_else(|| rule.event.clone());

    let payload = serde_json::json!({
        "item_sku": args.sku,
        "location_name": args.location,
        "on_hand": args.qty,
        "resulting_qty": args.qty,
    });

    let matches = rule.matches_event(
        &event_type,
        args.sku.as_deref(),
        args.location.as_deref(),
        &payload,
    );

    if !matches {
        if json {
            println!(
                r#"{{"rule_id": "{}", "matches": false, "reason": "Rule conditions did not match simulated event"}}"#,
                rule.id
            );
        } else {
            println!(
                "Rule '{}' does NOT match simulated event (event: '{}', sku: {:?}, location: {:?}, qty: {:?}).",
                rule.id, event_type, args.sku, args.location, args.qty
            );
        }
        return Ok(());
    }

    println!("Rule '{}' MATCHES the simulated event.", rule.id);

    let dummy_event = OutboxEvent {
        id: OutboxEventId::new(),
        event_type,
        aggregate_id: "test-aggregate".to_string(),
        payload: payload.to_string(),
        created_at: time::OffsetDateTime::now_utc(),
        delivered_at: None,
        attempt_count: 0,
        last_error: None,
    };

    let options = AutomationOptions {
        allow_commands: args.allow_commands || config.allow_commands.unwrap_or(false),
        command_allowlist: config.command_allowlist.clone().unwrap_or_default(),
        max_attempts: 5,
    };

    let engine = AutomationEngine::new(conn);
    let run = engine.execute_action(&rule, &dummy_event, &options);

    if json {
        println!("{}", serde_json::to_string_pretty(&run)?);
    } else {
        println!("Simulated Execution Result: [{}]", run.status);
        if let Some(ref out) = run.output {
            println!("  Output: {}", out);
        }
        if let Some(ref err) = run.error_message {
            println!("  Error: {}", err);
        }
    }

    Ok(())
}
