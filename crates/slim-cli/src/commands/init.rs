//! Init command handler.

use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn handle_init(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    let conn = slim_db::connection::open_connection(db_path)?;
    let applied = slim_db::migrate::run_migrations(&conn)?;
    let version = slim_db::migrate::current_version(&conn)?;

    println!("Initialized SLIM database at: {}", db_path.display());
    if applied > 0 {
        println!(
            "Applied {} migration(s). Current schema version: {}",
            applied, version
        );
    } else {
        println!("Schema is up to date (version {}).", version);
    }

    Ok(())
}
