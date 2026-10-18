//! SQLite implementation of AutomationRepository and AutomationEngine.
//!
//! Provides rule persistence, run history logging, and the core automation
//! engine for executing rules against committed outbox events.

use rusqlite::{Connection, params};
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::automation::*;
use slim_core::service::{AutomationRepository, OutboxRepository};
use std::process::Command;
use time::OffsetDateTime;
use uuid::Uuid;

use super::actor::{OptionalExt, format_time, parse_time};
use super::outbox::SqliteOutboxRepo;

/// Configuration options controlling automation execution safety.
#[derive(Debug, Clone)]
pub struct AutomationOptions {
    /// Whether external command actions are allowed to execute.
    pub allow_commands: bool,
    /// Allowlist of permitted command binary names or paths.
    pub command_allowlist: Vec<String>,
    /// Maximum retry attempts for a failing outbox event before skipping.
    pub max_attempts: u32,
}

impl Default for AutomationOptions {
    fn default() -> Self {
        Self {
            allow_commands: false,
            command_allowlist: Vec::new(),
            max_attempts: 5,
        }
    }
}

/// SQLite-backed automation repository.
pub struct SqliteAutomationRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteAutomationRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationRule> {
    let id: String = row.get(0)?;
    let name: Option<String> = row.get(1)?;
    let enabled: bool = row.get::<_, i64>(2)? != 0;
    let event: String = row.get(3)?;
    let item_sku: Option<String> = row.get(4)?;
    let location_name: Option<String> = row.get(5)?;
    let condition_json: Option<String> = row.get(6)?;
    let _action_type: String = row.get(7)?;
    let action_config: String = row.get(8)?;

    let condition = condition_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<RuleCondition>(json).ok());

    let action = serde_json::from_str::<RuleAction>(&action_config).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(AutomationRule {
        id,
        name,
        enabled,
        event,
        item_sku,
        location_name,
        condition,
        action,
    })
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationRun> {
    let id_str: String = row.get(0)?;
    let rule_id: String = row.get(1)?;
    let outbox_str: Option<String> = row.get(2)?;
    let event_type: String = row.get(3)?;
    let action_type: String = row.get(4)?;
    let status_str: String = row.get(5)?;
    let output: Option<String> = row.get(6)?;
    let error_message: Option<String> = row.get(7)?;
    let executed_str: String = row.get(8)?;

    let id = AutomationRunId(Uuid::parse_str(&id_str).unwrap_or_default());
    let outbox_id = outbox_str
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(OutboxEventId);

    let status = match status_str.as_str() {
        "success" => AutomationRunStatus::Success,
        "failed" => AutomationRunStatus::Failed,
        _ => AutomationRunStatus::Skipped,
    };
    let executed_at = parse_time(&executed_str);

    Ok(AutomationRun {
        id,
        rule_id,
        outbox_id,
        event_type,
        action_type,
        status,
        output,
        error_message,
        executed_at,
    })
}

impl AutomationRepository for SqliteAutomationRepo<'_> {
    fn save_rule(&self, rule: &AutomationRule) -> CoreResult<()> {
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        let condition_json = rule
            .condition
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());

        let action_config = serde_json::to_string(&rule.action)
            .map_err(|e| CoreError::InvalidInput(format!("invalid rule action: {}", e)))?;

        self.conn
            .execute(
                "INSERT INTO automation_rules (
                    id, name, enabled, event_type, item_sku, location_name,
                    condition_json, action_type, action_config, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                    name = ?2,
                    enabled = ?3,
                    event_type = ?4,
                    item_sku = ?5,
                    location_name = ?6,
                    condition_json = ?7,
                    action_type = ?8,
                    action_config = ?9,
                    updated_at = ?10",
                params![
                    rule.id,
                    rule.name.as_deref(),
                    if rule.enabled { 1 } else { 0 },
                    rule.event,
                    rule.item_sku.as_deref(),
                    rule.location_name.as_deref(),
                    condition_json.as_deref(),
                    rule.action.action_type_str(),
                    action_config,
                    now_str,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(())
    }

    fn get_rule(&self, id: &str) -> CoreResult<Option<AutomationRule>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, enabled, event_type, item_sku, location_name,
                        condition_json, action_type, action_config
                 FROM automation_rules WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![id], row_to_rule)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn list_rules(&self) -> CoreResult<Vec<AutomationRule>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, enabled, event_type, item_sku, location_name,
                        condition_json, action_type, action_config
                 FROM automation_rules WHERE archived_at IS NULL
                 ORDER BY created_at ASC",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_rule)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut rules = Vec::new();
        for row in rows {
            rules.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(rules)
    }

    fn delete_rule(&self, id: &str) -> CoreResult<()> {
        let rows = self
            .conn
            .execute("DELETE FROM automation_rules WHERE id = ?1", params![id])
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "automation rule",
                id: id.to_string(),
            });
        }

        Ok(())
    }

    fn record_run(&self, run: &AutomationRun) -> CoreResult<()> {
        let executed_str = format_time(run.executed_at);
        self.conn
            .execute(
                "INSERT INTO automation_runs (
                    id, rule_id, outbox_id, event_type, action_type, status,
                    output, error_message, executed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    run.id.0.to_string(),
                    run.rule_id,
                    run.outbox_id.map(|id| id.0.to_string()),
                    run.event_type,
                    run.action_type,
                    run.status.to_string(),
                    run.output.as_deref(),
                    run.error_message.as_deref(),
                    executed_str,
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(())
    }

    fn list_runs(&self, limit: usize) -> CoreResult<Vec<AutomationRun>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, rule_id, outbox_id, event_type, action_type, status,
                        output, error_message, executed_at
                 FROM automation_runs
                 ORDER BY executed_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], row_to_run)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(runs)
    }
}

/// Automation engine that evaluates outbox events and executes rule actions.
pub struct AutomationEngine<'a> {
    conn: &'a Connection,
    repo: SqliteAutomationRepo<'a>,
    outbox: SqliteOutboxRepo<'a>,
}

impl<'a> AutomationEngine<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self {
            conn,
            repo: SqliteAutomationRepo::new(conn),
            outbox: SqliteOutboxRepo::new(conn),
        }
    }

    /// Helper to resolve item SKU from item UUID string.
    pub fn resolve_item_sku(&self, item_id_str: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT sku FROM items WHERE id = ?1",
                params![item_id_str],
                |row| row.get(0),
            )
            .ok()
    }

    /// Helper to resolve location name from location UUID string.
    pub fn resolve_location_name(&self, location_id_str: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT name FROM locations WHERE id = ?1",
                params![location_id_str],
                |row| row.get(0),
            )
            .ok()
    }

    /// Check if a (rule_id, outbox_id) pair was already executed successfully (idempotency guard).
    fn is_already_executed(&self, rule_id: &str, outbox_id: &OutboxEventId) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs
                 WHERE rule_id = ?1 AND outbox_id = ?2 AND status = 'success'",
                params![rule_id, outbox_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap_or(0);

        count > 0
    }

    /// Execute a single rule action against an event.
    pub fn execute_action(
        &self,
        rule: &AutomationRule,
        event: &OutboxEvent,
        options: &AutomationOptions,
    ) -> AutomationRun {
        let run_id = AutomationRunId::new();
        let now = OffsetDateTime::now_utc();

        match &rule.action {
            RuleAction::Notify { message } => AutomationRun {
                id: run_id,
                rule_id: rule.id.clone(),
                outbox_id: Some(event.id),
                event_type: event.event_type.clone(),
                action_type: "notify".to_string(),
                status: AutomationRunStatus::Success,
                output: Some(message.clone()),
                error_message: None,
                executed_at: now,
            },

            RuleAction::Command { command, args } => {
                if !options.allow_commands {
                    return AutomationRun {
                        id: run_id,
                        rule_id: rule.id.clone(),
                        outbox_id: Some(event.id),
                        event_type: event.event_type.clone(),
                        action_type: "command".to_string(),
                        status: AutomationRunStatus::Failed,
                        output: None,
                        error_message: Some(
                            "Command execution is disabled by default. Enable with --allow-commands or in config."
                                .to_string(),
                        ),
                        executed_at: now,
                    };
                }

                // Check allowlist
                let cmd_file_name = std::path::Path::new(command)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(command.as_str());

                let is_allowed = options.command_allowlist.iter().any(|allowed| {
                    allowed == "*"
                        || allowed.eq_ignore_ascii_case(command)
                        || allowed.eq_ignore_ascii_case(cmd_file_name)
                });

                if !is_allowed {
                    return AutomationRun {
                        id: run_id,
                        rule_id: rule.id.clone(),
                        outbox_id: Some(event.id),
                        event_type: event.event_type.clone(),
                        action_type: "command".to_string(),
                        status: AutomationRunStatus::Failed,
                        output: None,
                        error_message: Some(format!(
                            "Command '{}' is not in the approved command allowlist",
                            command
                        )),
                        executed_at: now,
                    };
                }

                // Execute the command
                match Command::new(command).args(args).output() {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

                        if output.status.success() {
                            let out_text = if !stdout.is_empty() {
                                stdout
                            } else if !stderr.is_empty() {
                                stderr
                            } else {
                                "Command completed successfully with code 0".to_string()
                            };

                            AutomationRun {
                                id: run_id,
                                rule_id: rule.id.clone(),
                                outbox_id: Some(event.id),
                                event_type: event.event_type.clone(),
                                action_type: "command".to_string(),
                                status: AutomationRunStatus::Success,
                                output: Some(out_text),
                                error_message: None,
                                executed_at: now,
                            }
                        } else {
                            AutomationRun {
                                id: run_id,
                                rule_id: rule.id.clone(),
                                outbox_id: Some(event.id),
                                event_type: event.event_type.clone(),
                                action_type: "command".to_string(),
                                status: AutomationRunStatus::Failed,
                                output: Some(stdout),
                                error_message: Some(format!(
                                    "Command failed with exit code {:?}: {}",
                                    output.status.code(),
                                    stderr
                                )),
                                executed_at: now,
                            }
                        }
                    }
                    Err(e) => AutomationRun {
                        id: run_id,
                        rule_id: rule.id.clone(),
                        outbox_id: Some(event.id),
                        event_type: event.event_type.clone(),
                        action_type: "command".to_string(),
                        status: AutomationRunStatus::Failed,
                        output: None,
                        error_message: Some(format!("Failed to execute command: {}", e)),
                        executed_at: now,
                    },
                }
            }

            RuleAction::Webhook { url, payload } => {
                let body = payload.clone().unwrap_or_else(|| event.payload.clone());
                let result = if url.starts_with("https://") || url.starts_with("http://") {
                    ureq::AgentBuilder::new()
                        .timeout(std::time::Duration::from_secs(10))
                        .redirects(0)
                        .build()
                        .post(url)
                        .set("Content-Type", "application/json")
                        .set("Idempotency-Key", &format!("{}:{}", event.id, rule.id))
                        .send_string(&body)
                        .map(|response| response.status().to_string())
                        .map_err(|error| error.to_string())
                } else {
                    Err("Webhook URL must use HTTP or HTTPS".into())
                };
                AutomationRun {
                    id: run_id,
                    rule_id: rule.id.clone(),
                    outbox_id: Some(event.id),
                    event_type: event.event_type.clone(),
                    action_type: "webhook".to_string(),
                    status: if result.is_ok() {
                        AutomationRunStatus::Success
                    } else {
                        AutomationRunStatus::Failed
                    },
                    output: result
                        .as_ref()
                        .ok()
                        .map(|status| format!("POST delivered: HTTP {status}")),
                    error_message: result.err(),
                    executed_at: now,
                }
            }
        }
    }

    /// Process a batch of pending outbox events against the given rules.
    pub fn process_pending(
        &self,
        rules: &[AutomationRule],
        options: &AutomationOptions,
        limit: usize,
    ) -> CoreResult<Vec<AutomationRun>> {
        let events = self.outbox.fetch_pending(limit)?;
        let mut executed_runs = Vec::new();

        for event in events {
            // Reserve the event before external effects. Independent app/worker
            // processes must not dispatch the same event concurrently.
            let now = OffsetDateTime::now_utc();
            let lease = format_time(now + time::Duration::minutes(5));
            let claimed = self
                .conn
                .execute(
                    "UPDATE event_outbox SET claimed_until=?1 WHERE id=?2 AND delivered_at IS NULL
                 AND (claimed_until IS NULL OR claimed_until<=?3)",
                    params![lease, event.id.to_string(), format_time(now)],
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;
            if claimed == 0 {
                continue;
            }
            if event.attempt_count >= options.max_attempts {
                // Exceeded max attempts: record skipped run and mark delivered to unblock outbox
                let run = AutomationRun {
                    id: AutomationRunId::new(),
                    rule_id: "system-max-attempts".to_string(),
                    outbox_id: Some(event.id),
                    event_type: event.event_type.clone(),
                    action_type: "system".to_string(),
                    status: AutomationRunStatus::Skipped,
                    output: None,
                    error_message: Some(format!(
                        "Exceeded max attempt limit of {} retries; skipping event",
                        options.max_attempts
                    )),
                    executed_at: OffsetDateTime::now_utc(),
                };
                self.repo.record_run(&run)?;
                self.outbox.mark_delivered(&event.id)?;
                executed_runs.push(run);
                continue;
            }

            let payload_json: serde_json::Value =
                serde_json::from_str(&event.payload).unwrap_or(serde_json::Value::Null);

            // Resolve item SKU from aggregate_id or payload item_id
            let item_id_str = payload_json
                .get("item_id")
                .and_then(|v| v.as_str())
                .unwrap_or(&event.aggregate_id);
            let item_sku = self.resolve_item_sku(item_id_str);

            // Resolve location name if present in payload
            let location_id_str = payload_json.get("location_id").and_then(|v| v.as_str());
            let location_name = location_id_str.and_then(|id| self.resolve_location_name(id));

            let mut any_action_failed = false;
            let mut matched_any = false;

            for rule in rules {
                if !rule.matches_event(
                    &event.event_type,
                    item_sku.as_deref(),
                    location_name.as_deref(),
                    &payload_json,
                ) {
                    continue;
                }

                matched_any = true;

                // Idempotency check: don't re-run successful action for this outbox event
                if self.is_already_executed(&rule.id, &event.id) {
                    continue;
                }

                let run = self.execute_action(rule, &event, options);
                if run.status == AutomationRunStatus::Failed {
                    any_action_failed = true;
                    if let Some(ref err) = run.error_message {
                        let _ = self.outbox.record_attempt_error(&event.id, err);
                    }
                }

                self.repo.record_run(&run)?;
                executed_runs.push(run);
            }

            // If no action failed (or no rule matched), mark the event delivered
            if !any_action_failed {
                self.outbox.mark_delivered(&event.id)?;
            }
            self.conn
                .execute(
                    "UPDATE event_outbox SET claimed_until=NULL WHERE id=?1",
                    [event.id.to_string()],
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            // Suppress unused warning if no rule matched
            let _ = matched_any;
        }

        Ok(executed_runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;

    #[test]
    fn test_rule_crud_persistence() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        let repo = SqliteAutomationRepo::new(&conn);

        let rule = AutomationRule {
            id: "low-m3".to_string(),
            name: Some("Low M3 Alert".to_string()),
            enabled: true,
            event: "stock.below_threshold".to_string(),
            item_sku: Some("M3X10".to_string()),
            location_name: Some("GF-001".to_string()),
            condition: Some(RuleCondition {
                quantity_below: Some(50),
                ..Default::default()
            }),
            action: RuleAction::Notify {
                message: "M3 screws running low".to_string(),
            },
        };

        repo.save_rule(&rule).unwrap();

        let fetched = repo.get_rule("low-m3").unwrap().unwrap();
        assert_eq!(fetched.id, "low-m3");
        assert_eq!(fetched.name.as_deref(), Some("Low M3 Alert"));
        assert!(fetched.enabled);
        assert_eq!(fetched.event, "stock.below_threshold");
        assert_eq!(fetched.item_sku.as_deref(), Some("M3X10"));
        assert_eq!(fetched.location_name.as_deref(), Some("GF-001"));
        assert_eq!(fetched.condition.unwrap().quantity_below, Some(50));
        assert_eq!(
            fetched.action,
            RuleAction::Notify {
                message: "M3 screws running low".to_string()
            }
        );

        let list = repo.list_rules().unwrap();
        assert_eq!(list.len(), 1);

        repo.delete_rule("low-m3").unwrap();
        assert!(repo.get_rule("low-m3").unwrap().is_none());
        assert_eq!(repo.list_rules().unwrap().len(), 0);
    }

    #[test]
    fn test_record_and_list_runs() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        let repo = SqliteAutomationRepo::new(&conn);

        let run = AutomationRun {
            id: AutomationRunId::new(),
            rule_id: "low-m3".to_string(),
            outbox_id: None,
            event_type: "StockBelowThreshold".to_string(),
            action_type: "notify".to_string(),
            status: AutomationRunStatus::Success,
            output: Some("Notification sent".to_string()),
            error_message: None,
            executed_at: OffsetDateTime::now_utc(),
        };

        repo.record_run(&run).unwrap();

        let runs = repo.list_runs(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].rule_id, "low-m3");
        assert_eq!(runs[0].status, AutomationRunStatus::Success);
    }

    #[test]
    fn test_engine_processes_matching_events() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Insert item M3X10
        let now_str = format_time(OffsetDateTime::now_utc());
        let item_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO items (id, sku, name, unit, created_at, updated_at)
             VALUES (?1, 'M3X10', 'M3 Screw', 'each', ?2, ?2)",
            params![item_id, now_str],
        )
        .unwrap();

        // Insert outbox event for StockBelowThreshold
        let event_id = OutboxEventId::new();
        let payload = serde_json::json!({
            "schema_version": 1,
            "item_id": item_id,
            "on_hand": 20,
            "min_qty": 50,
        });

        conn.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockBelowThreshold', ?2, ?3, ?4)",
            params![
                event_id.0.to_string(),
                item_id,
                payload.to_string(),
                now_str
            ],
        )
        .unwrap();

        let rules = vec![AutomationRule {
            id: "low-stock-rule".to_string(),
            name: Some("Low Stock Alert".to_string()),
            enabled: true,
            event: "stock.below_threshold".to_string(),
            item_sku: Some("M3X10".to_string()),
            location_name: None,
            condition: Some(RuleCondition {
                quantity_below: Some(50),
                ..Default::default()
            }),
            action: RuleAction::Notify {
                message: "M3 stock is low".to_string(),
            },
        }];

        let engine = AutomationEngine::new(&conn);
        let options = AutomationOptions::default();

        let runs = engine.process_pending(&rules, &options, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, AutomationRunStatus::Success);
        assert_eq!(runs[0].output.as_deref(), Some("M3 stock is low"));

        // Event should now be delivered
        let outbox = SqliteOutboxRepo::new(&conn);
        assert_eq!(outbox.pending_count().unwrap(), 0);
    }

    #[test]
    fn test_engine_command_safety_guard() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();

        let event_id = OutboxEventId::new();
        let now_str = format_time(OffsetDateTime::now_utc());
        conn.execute(
            "INSERT INTO event_outbox (id, event_type, aggregate_id, payload, created_at)
             VALUES (?1, 'StockReceived', 'item-1', '{}', ?2)",
            params![event_id.0.to_string(), now_str],
        )
        .unwrap();

        let rules = vec![AutomationRule {
            id: "cmd-rule".to_string(),
            name: None,
            enabled: true,
            event: "stock.received".to_string(),
            item_sku: None,
            location_name: None,
            condition: None,
            action: RuleAction::Command {
                command: "whoami".to_string(),
                args: vec![],
            },
        }];

        let engine = AutomationEngine::new(&conn);

        // 1. By default, command is disabled
        let default_options = AutomationOptions::default();
        let runs = engine
            .process_pending(&rules, &default_options, 10)
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, AutomationRunStatus::Failed);
        assert!(
            runs[0]
                .error_message
                .as_ref()
                .unwrap()
                .contains("disabled by default")
        );

        // Outbox event should NOT be marked delivered because an action failed
        let outbox = SqliteOutboxRepo::new(&conn);
        assert_eq!(outbox.pending_count().unwrap(), 1);

        // 2. Enable commands but command not in allowlist
        let disallowed_options = AutomationOptions {
            allow_commands: true,
            command_allowlist: vec!["other_command".to_string()],
            max_attempts: 5,
        };
        let runs2 = engine
            .process_pending(&rules, &disallowed_options, 10)
            .unwrap();
        assert_eq!(runs2.len(), 1);
        assert_eq!(runs2[0].status, AutomationRunStatus::Failed);
        assert!(
            runs2[0]
                .error_message
                .as_ref()
                .unwrap()
                .contains("not in the approved command allowlist")
        );

        // 3. Enable commands with allowlist containing whoami
        let allowed_options = AutomationOptions {
            allow_commands: true,
            command_allowlist: vec!["whoami".to_string()],
            max_attempts: 5,
        };
        let runs3 = engine
            .process_pending(&rules, &allowed_options, 10)
            .unwrap();
        assert_eq!(runs3.len(), 1);
        assert_eq!(runs3[0].status, AutomationRunStatus::Success);

        // Now event should be delivered
        assert_eq!(outbox.pending_count().unwrap(), 0);
    }
}
