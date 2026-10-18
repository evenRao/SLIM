//! Entity resolution helpers for CLI commands (items, locations, actors).

use anyhow::{Result, bail};
use rusqlite::Connection;
use slim_core::model::actor::Actor;
use slim_core::model::item::Item;
use slim_core::model::location::Location;
use slim_core::service::{ActorRepository, ItemRepository, LocationRepository};
use slim_db::repo::actor::SqliteActorRepo;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::location::SqliteLocationRepo;

/// Resolve an actor using the priority order defined in ADR-03:
/// 1. Explicit CLI --actor flag
/// 2. Config file default_actor
/// 3. OS username mapping
/// 4. Single-actor DB convenience
pub fn resolve_actor(
    conn: &Connection,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
) -> Result<Actor> {
    let repo = SqliteActorRepo::new(conn);
    use slim_core::service::AuthService;
    let session = crate::config::Config::load_session_token().and_then(|token| {
        slim_db::repo::auth::SqliteAuthService::new(conn)
            .validate_session(&token)
            .ok()
    });
    let validate = |actor: Actor| -> Result<Actor> {
        anyhow::ensure!(actor.active, "Account is inactive");
        if repo.get_pin_hash(&actor.id)?.is_some() {
            anyhow::ensure!(
                session.as_ref().is_some_and(|s| s.actor.id == actor.id),
                "'{}' requires login. Use 'slim session login' or sign in inside the app.",
                actor.display_name
            );
        }
        Ok(actor)
    };
    if let Some(name) = cli_actor {
        return validate(
            repo.get_by_name(name)?
                .ok_or_else(|| anyhow::anyhow!("Actor '{}' not found", name))?,
        );
    }
    if let Some(auth) = session.as_ref() {
        return Ok(auth.actor.clone());
    }
    if let Some(name) = config_actor {
        return validate(
            repo.get_by_name(name)?
                .ok_or_else(|| anyhow::anyhow!("Configured actor '{}' not found", name))?,
        );
    }
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .ok();
    if let Some(user) = user
        && let Some(actor) = repo.get_by_os_username(&user)?
    {
        return validate(actor);
    }
    let actors = repo.list()?;
    if actors.len() == 1 {
        return validate(actors[0].clone());
    }
    bail!("Choose a user in the app, or sign in with 'slim session login'.")
}

/// Resolve an item by SKU or name.
pub fn resolve_item(conn: &Connection, identifier: &str) -> Result<Item> {
    let repo = SqliteItemRepo::new(conn);
    if let Ok(id) = identifier.parse()
        && let Some(item) = repo.get_by_id(&id)?
    {
        return Ok(item);
    }

    // Try SKU first
    if let Some(item) = repo.get_by_sku(identifier)? {
        return Ok(item);
    }

    // Try exact name match
    let mut stmt = conn.prepare(
        "SELECT id, sku, name, unit, description, metadata, archived_at, created_at, updated_at
         FROM items WHERE name = ?1 AND archived_at IS NULL",
    )?;

    let mut matches: Vec<Item> = Vec::new();
    let rows = stmt.query_map([identifier], |row| {
        let id_str: String = row.get(0)?;
        let sku: Option<String> = row.get(1)?;
        let name: String = row.get(2)?;
        let unit: String = row.get(3)?;
        let description: Option<String> = row.get(4)?;
        let metadata_str: Option<String> = row.get(5)?;
        let archived_str: Option<String> = row.get(6)?;
        let created_str: String = row.get(7)?;
        let updated_str: String = row.get(8)?;

        let id = slim_core::model::item::ItemId(uuid::Uuid::parse_str(&id_str).unwrap_or_default());
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());
        let archived_at = archived_str.map(|s| {
            time::OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        });

        Ok(Item {
            id,
            sku,
            name,
            unit,
            description,
            metadata,
            archived_at,
            created_at: time::OffsetDateTime::parse(
                &created_str,
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            updated_at: time::OffsetDateTime::parse(
                &updated_str,
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
        })
    })?;

    for row in rows {
        matches.push(row?);
    }

    if matches.len() == 1 {
        return Ok(matches.into_iter().next().unwrap());
    }

    if matches.len() > 1 {
        let skus: Vec<String> = matches
            .iter()
            .map(|m| m.sku.clone().unwrap_or_else(|| m.id.to_string()))
            .collect();
        bail!(
            "Multiple items found matching '{}'. Specify by SKU: {}",
            identifier,
            skus.join(", ")
        );
    }

    // Try search
    let searched = repo.list(&slim_core::model::item::ItemFilter {
        search: Some(identifier.to_string()),
        include_archived: false,
    })?;

    if searched.len() == 1 {
        return Ok(searched.into_iter().next().unwrap());
    }

    if searched.len() > 1 {
        let names: Vec<String> = searched
            .iter()
            .map(|m| format!("{} (SKU: {})", m.name, m.sku.as_deref().unwrap_or("none")))
            .collect();
        bail!(
            "Ambiguous item query '{}'. Matches:\n  - {}",
            identifier,
            names.join("\n  - ")
        );
    }

    bail!(
        "Item '{}' not found (tried SKU and name search)",
        identifier
    );
}

/// Resolve a location by name.
pub fn resolve_location(conn: &Connection, name: &str) -> Result<Location> {
    let repo = SqliteLocationRepo::new(conn);
    if let Ok(id) = name.parse()
        && let Some(location) = repo.get_by_id(&id)?
    {
        return Ok(location);
    }
    if name.contains(" > ") {
        let mut parent = None;
        let mut result = None;
        for part in name.split(" > ") {
            let location = repo
                .find_by_name(part.trim(), parent.as_ref())?
                .ok_or_else(|| anyhow::anyhow!("Location '{}' not found", name))?;
            parent = Some(location.id);
            result = Some(location);
        }
        return result.ok_or_else(|| anyhow::anyhow!("Location '{}' not found", name));
    }

    let mut stmt = conn.prepare(
        "SELECT id, parent_id, name, kind, can_hold_stock, created_at, updated_at
         FROM locations WHERE name = ?1",
    )?;

    let mut matches: Vec<Location> = Vec::new();
    let rows = stmt.query_map([name], |row| {
        let id_str: String = row.get(0)?;
        let parent_str: Option<String> = row.get(1)?;
        let name: String = row.get(2)?;
        let kind_str: String = row.get(3)?;
        let can_hold_stock: bool = row.get(4)?;
        let created_str: String = row.get(5)?;
        let updated_str: String = row.get(6)?;

        let id = slim_core::model::location::LocationId(
            uuid::Uuid::parse_str(&id_str).unwrap_or_default(),
        );
        let parent_id = parent_str.map(|s| {
            slim_core::model::location::LocationId(uuid::Uuid::parse_str(&s).unwrap_or_default())
        });
        let kind: slim_core::model::location::LocationKind = kind_str
            .parse()
            .unwrap_or(slim_core::model::location::LocationKind::Storage);

        Ok(Location {
            id,
            parent_id,
            name,
            kind,
            can_hold_stock,
            created_at: time::OffsetDateTime::parse(
                &created_str,
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            updated_at: time::OffsetDateTime::parse(
                &updated_str,
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
        })
    })?;

    for row in rows {
        matches.push(row?);
    }

    if matches.len() == 1 {
        return Ok(matches.into_iter().next().unwrap());
    }

    if matches.len() > 1 {
        let mut paths = Vec::new();
        for m in &matches {
            let path = repo.get_path(&m.id)?;
            let path_str: Vec<String> = path.iter().map(|p| p.name.clone()).collect();
            paths.push(format!("{} (ID: {})", path_str.join(" > "), m.id));
        }
        bail!(
            "Multiple locations named '{}' found:\n  - {}",
            name,
            paths.join("\n  - ")
        );
    }

    bail!("Location '{}' not found", name);
}
