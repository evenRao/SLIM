//! Schema migration runner for SLIM.
//!
//! Migrations are embedded SQL strings applied in order. A `_schema_versions`
//! table tracks which migrations have been applied.

use rusqlite::Connection;

use crate::connection::StorageResult;

/// A single migration with its version number and SQL.
struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

/// All migrations in order. Add new migrations at the end.
fn all_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            name: "initial_schema",
            sql: include_str!("../migrations/001_initial.sql"),
        },
        Migration {
            version: 2,
            name: "gridfinity_schema",
            sql: include_str!("../migrations/002_gridfinity.sql"),
        },
        Migration {
            version: 3,
            name: "auth_and_sessions",
            sql: include_str!("../migrations/003_auth_and_sessions.sql"),
        },
        Migration {
            version: 4,
            name: "automation",
            sql: include_str!("../migrations/004_automation.sql"),
        },
        Migration {
            version: 5,
            name: "devices",
            sql: include_str!("../migrations/005_devices.sql"),
        },
        Migration {
            version: 6,
            name: "led_and_docks",
            sql: include_str!("../migrations/006_led_and_docks.sql"),
        },
        Migration {
            version: 7,
            name: "reliability",
            sql: include_str!("../migrations/007_reliability.sql"),
        },
        Migration {
            version: 8,
            name: "catalog_management",
            sql: include_str!("../migrations/008_catalog_management.sql"),
        },
    ]
}

/// Ensure the schema versions table exists.
fn ensure_versions_table(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _schema_versions (
            version    INTEGER PRIMARY KEY NOT NULL,
            name       TEXT NOT NULL,
            applied_at TEXT NOT NULL
         );",
    )?;
    Ok(())
}

/// Get the current schema version (0 if no migrations applied).
pub fn current_version(conn: &Connection) -> StorageResult<u32> {
    ensure_versions_table(conn)?;
    let version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _schema_versions",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Apply all pending migrations. Returns the number of migrations applied.
pub fn run_migrations(conn: &Connection) -> StorageResult<u32> {
    ensure_versions_table(conn)?;

    let current = current_version(conn)?;
    let migrations = all_migrations();
    let mut applied = 0u32;

    for migration in &migrations {
        if migration.version <= current {
            continue;
        }

        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applying migration"
        );

        let tx = conn.unchecked_transaction()?;

        // Execute the migration SQL
        tx.execute_batch(migration.sql)?;

        // Record the migration
        let now = time::OffsetDateTime::now_utc();
        let now_str = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".to_string());

        tx.execute(
            "INSERT INTO _schema_versions (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, migration.name, now_str],
        )?;

        tx.commit()?;
        applied += 1;

        tracing::info!(
            version = migration.version,
            name = migration.name,
            "migration applied successfully"
        );
    }

    Ok(applied)
}

/// Check if the database is up to date.
pub fn is_up_to_date(conn: &Connection) -> StorageResult<bool> {
    let current = current_version(conn)?;
    let latest = all_migrations().last().map_or(0, |m| m.version);
    Ok(current >= latest)
}

/// Get the latest available migration version.
pub fn latest_version() -> u32 {
    all_migrations().last().map_or(0, |m| m.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;

    #[test]
    fn fresh_db_has_version_zero() {
        let conn = open_memory().unwrap();
        let ver = current_version(&conn).unwrap();
        assert_eq!(ver, 0);
    }

    #[test]
    fn migrations_apply_to_fresh_db() {
        let conn = open_memory().unwrap();
        let applied = run_migrations(&conn).unwrap();
        assert!(applied > 0, "at least one migration should be applied");
        assert!(is_up_to_date(&conn).unwrap());
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = open_memory().unwrap();
        let first = run_migrations(&conn).unwrap();
        let second = run_migrations(&conn).unwrap();
        assert!(first > 0);
        assert_eq!(second, 0, "re-running migrations should apply zero");
    }

    #[test]
    fn all_tables_created() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();

        let expected_tables = [
            "actors",
            "items",
            "locations",
            "identifiers",
            "stock_thresholds",
            "operations",
            "stock_movements",
            "stock_balances",
            "event_outbox",
            "sessions",
            "audit_events",
            "automation_rules",
            "automation_runs",
            "devices",
            "observations",
            "dock_led_states",
            "dock_presence_states",
        ];

        for table in &expected_tables {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "table {table} should exist");
        }
    }

    #[test]
    fn foreign_keys_enforced() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Try to insert a stock_movement referencing a nonexistent operation
        let result = conn.execute(
            "INSERT INTO stock_movements (id, operation_id, item_id, location_id, delta_qty, created_at)
             VALUES ('test-mv', 'nonexistent-op', 'nonexistent-item', 'nonexistent-loc', 1, '2026-01-01T00:00:00Z')",
            [],
        );
        assert!(result.is_err(), "FK violation should be rejected");
    }

    #[test]
    fn duplicate_sku_rejected() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();

        let now = "2026-01-01T00:00:00Z";
        conn.execute(
            "INSERT INTO items (id, sku, name, unit, created_at, updated_at) VALUES ('id1', 'M3X10', 'Screw', 'each', ?1, ?1)",
            [now],
        ).unwrap();

        let result = conn.execute(
            "INSERT INTO items (id, sku, name, unit, created_at, updated_at) VALUES ('id2', 'M3X10', 'Duplicate', 'each', ?1, ?1)",
            [now],
        );
        assert!(result.is_err(), "duplicate SKU should be rejected");
    }
}
