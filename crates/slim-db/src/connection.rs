//! SQLite connection management with proper PRAGMA configuration.

use rusqlite::Connection;
use std::path::Path;

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<StorageError> for slim_core::error::CoreError {
    fn from(e: StorageError) -> Self {
        slim_core::error::CoreError::Storage(e.to_string())
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Open a SQLite connection with the required PRAGMA configuration.
pub fn open_connection(path: &Path) -> StorageResult<Connection> {
    let conn = Connection::open(path)?;
    configure_connection(&conn)?;
    Ok(conn)
}

/// Open an in-memory SQLite connection (for testing).
pub fn open_memory() -> StorageResult<Connection> {
    let conn = Connection::open_in_memory()?;
    configure_connection(&conn)?;
    Ok(conn)
}

/// Apply standard PRAGMA settings to a connection.
fn configure_connection(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA synchronous = NORMAL;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_connection_has_correct_pragmas() {
        let conn = open_memory().unwrap();

        let fk: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(fk, "foreign_keys should be ON");

        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        // In-memory databases use "memory" journal mode, which is fine.
        // WAL is applied but may fall back to "memory" for in-memory DBs.
        assert!(
            journal == "wal" || journal == "memory",
            "journal_mode should be wal or memory, got: {journal}"
        );

        let timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);
    }

    #[test]
    fn file_connection_opens_and_configures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let conn = open_connection(&path).unwrap();

        let fk: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(fk);

        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
    }
}
