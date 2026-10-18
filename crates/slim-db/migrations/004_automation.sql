-- ═══════════════════════════════════════════════════════════
-- SLIM SCHEMA MIGRATION 004: AUTOMATION AND HOOKS
-- ═══════════════════════════════════════════════════════════

-- Automation rules stored in database (in addition to file-based rules)
CREATE TABLE IF NOT EXISTS automation_rules (
    id             TEXT PRIMARY KEY NOT NULL,
    name           TEXT,
    enabled        INTEGER NOT NULL DEFAULT 1,
    event_type     TEXT NOT NULL,
    item_sku       TEXT,
    location_name  TEXT,
    condition_json TEXT,
    action_type    TEXT NOT NULL,
    action_config  TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

-- Automation execution runs history
CREATE TABLE IF NOT EXISTS automation_runs (
    id            TEXT PRIMARY KEY NOT NULL,
    rule_id       TEXT NOT NULL,
    outbox_id     TEXT REFERENCES event_outbox(id),
    event_type    TEXT NOT NULL,
    action_type   TEXT NOT NULL,
    status        TEXT NOT NULL,
    output        TEXT,
    error_message TEXT,
    executed_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_automation_runs_rule ON automation_runs(rule_id);
CREATE INDEX IF NOT EXISTS idx_automation_runs_executed ON automation_runs(executed_at);
CREATE INDEX IF NOT EXISTS idx_automation_runs_outbox ON automation_runs(outbox_id);
