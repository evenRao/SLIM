//! Offline, loopback-only application. Assets are embedded in the executable.
use crate::{cli::AppArgs, config::Config};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use slim_core::{
    model::{
        actor::*, auth::*, automation::*, device::*, gridfinity::*, identifier::*, item::*,
        location::*, operation::*, stock::*,
    },
    service::*,
};
use slim_db::{
    connection::open_connection,
    migrate,
    repo::{
        actor::SqliteActorRepo,
        audit::SqliteAuditRepo,
        auth::SqliteAuthService,
        automation::{AutomationEngine, AutomationOptions, SqliteAutomationRepo},
        device::SqliteDeviceRepo,
        grid::SqliteGridRepo,
        identifier::SqliteIdentifierRepo,
        inventory::{SqliteInventoryService, SqliteReconciliationService, SqliteThresholdRepo},
        item::SqliteItemRepo,
        led::SqliteLedRepo,
        location::SqliteLocationRepo,
        scan::SqliteScanResolver,
    },
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use uuid::Uuid;

pub fn run(path: &Path, config: &Config, args: &AppArgs) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let conn = open_connection(path)?;
    if !migrate::is_up_to_date(&conn)? {
        let has_tables: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='items')",
            [],
            |r| r.get(0),
        )?;
        if has_tables {
            let backup =
                path.with_extension(format!("before-v{}.sqlite3", migrate::latest_version()));
            ensure!(
                !backup.exists(),
                "Migration backup already exists at {}. Move it to a safe location before retrying.",
                backup.display()
            );
            conn.backup(rusqlite::DatabaseName::Main, &backup, None)?;
        }
        migrate::run_migrations(&conn)?;
    }
    let server =
        Server::http(("127.0.0.1", args.port)).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let address = server.server_addr().to_string();
    let origin = format!("http://{address}");
    let key = Uuid::new_v4().simple().to_string();
    let url = format!("{origin}/#{key}");
    println!("SLIM is ready: {url}");
    use std::io::Write;
    std::io::stdout().flush()?;
    if !args.no_open
        && let Err(error) = open_browser(&url)
    {
        eprintln!("Open {url} in your browser: {error}");
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let db = path.to_path_buf();
    let cfg = config.clone();
    let worker = std::thread::spawn(move || {
        while !worker_stop.load(Ordering::Relaxed) {
            if let Ok(c) = open_connection(&db) {
                let rules = crate::commands::automation::load_all_rules(&c, &cfg);
                // The application never enables arbitrary command actions implicitly.
                let options = AutomationOptions {
                    allow_commands: cfg.allow_commands.unwrap_or(false),
                    command_allowlist: cfg.command_allowlist.clone().unwrap_or_default(),
                    max_attempts: 5,
                };
                if let Err(error) = AutomationEngine::new(&c).process_pending(&rules, &options, 20)
                {
                    tracing::warn!(%error,"automation worker");
                }
            }
            for _ in 0..20 {
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    for mut request in server.incoming_requests() {
        let header = |name: &str| {
            request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
                .map(|h| h.value.as_str().to_owned())
        };
        let trusted = header("host").as_deref() == Some(&address)
            && header("origin").is_none_or(|o| o == origin);
        let api = request.url().starts_with("/api/");
        let authorized = !api || header("x-slim-key").as_deref() == Some(&key);
        let session = header("x-slim-session");
        let mut content_type = "application/json; charset=utf-8";
        let mut status = 200;
        let mut quit = false;
        let body = if !trusted || !authorized {
            status = 403;
            json!({"error":"This request is not authorized. Reopen SLIM from its launcher."})
                .to_string()
        } else if !api {
            match (request.method(), request.url()) {
                (&Method::Get, "/") => {
                    content_type = "text/html; charset=utf-8";
                    include_str!("desktop/index.html").to_owned()
                }
                (&Method::Get, "/app.css") => {
                    content_type = "text/css; charset=utf-8";
                    include_str!("desktop/app.css").to_owned()
                }
                (&Method::Get, "/app.js") => {
                    content_type = "text/javascript; charset=utf-8";
                    include_str!("desktop/app.js").to_owned()
                }
                _ => {
                    status = 404;
                    json!({"error":"Not found"}).to_string()
                }
            }
        } else {
            let route = request.url().to_owned();
            let method = request.method().as_str().to_owned();
            let result = (|| -> Result<Value> {
                ensure!(
                    method == "GET" || method == "POST",
                    "Unsupported request method"
                );
                let mut bytes = Vec::new();
                use std::io::Read;
                request
                    .as_reader()
                    .take(1_048_577)
                    .read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 1_048_576, "Request is too large");
                let data = if bytes.is_empty() {
                    json!({})
                } else {
                    serde_json::from_slice(&bytes).context("Invalid request JSON")?
                };
                if method == "GET" {
                    ensure!(route == "/api/state", "Unknown read operation");
                    return state(&conn, path, session.as_deref());
                }
                if route == "/api/quit" {
                    quit = true;
                    return Ok(json!({"ok":true}));
                }
                let result = dispatch(&conn, path, &route, &data, session.as_deref())?;
                if result.get("restarted").and_then(Value::as_bool) == Some(true) {
                    quit = true;
                }
                Ok(result)
            })();
            match result {
                Ok(value) => value.to_string(),
                Err(error) => {
                    status = 400;
                    json!({"error":format!("{error:#}")}).to_string()
                }
            }
        };
        let response=Response::from_string(body).with_status_code(StatusCode(status))
            .with_header(Header::from_bytes("Content-Type",content_type).unwrap())
            .with_header(Header::from_bytes("Cache-Control","no-store").unwrap())
            .with_header(Header::from_bytes("X-Content-Type-Options","nosniff").unwrap())
            .with_header(Header::from_bytes("Referrer-Policy","no-referrer").unwrap())
            .with_header(Header::from_bytes("Content-Security-Policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'").unwrap());
        let _ = request.respond(response);
        if quit {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = worker.join();
    Ok(())
}

fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(url).spawn();
    status.map(|_| ())
}

fn pick_inventory() -> Result<Option<std::path::PathBuf>> {
    #[cfg(target_os = "windows")]
    let output = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("powershell.exe").creation_flags(0x08000000)
            .args(["-NoProfile","-STA","-Command","Add-Type -AssemblyName System.Windows.Forms; $dialog=New-Object System.Windows.Forms.OpenFileDialog; $dialog.Title='Open SLIM inventory or backup'; $dialog.Filter='SQLite inventory (*.sqlite3;*.db)|*.sqlite3;*.db|All files (*.*)|*.*'; if ($dialog.ShowDialog() -eq 'OK') { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::WriteLine($dialog.FileName) }"]).output()?
    };
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "POSIX path of (choose file with prompt \"Open SLIM inventory or backup\")",
        ])
        .output()?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let output = std::process::Command::new("zenity")
        .args(["--file-selection", "--title=Open SLIM inventory or backup"])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout)?
        .trim()
        .trim_start_matches('\u{feff}')
        .to_owned();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.into()))
    }
}

fn text<'a>(data: &'a Value, key: &str) -> Result<&'a str> {
    let value = data.get(key).and_then(Value::as_str).unwrap_or("").trim();
    ensure!(!value.is_empty(), "{} is required", key.replace('_', " "));
    Ok(value)
}
fn optional(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn number(data: &Value, key: &str) -> Result<u32> {
    data.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("{} must be a whole number from 0 to {}", key, u32::MAX))
}
fn actor(conn: &Connection, session: Option<&str>, role: Role) -> Result<Actor> {
    let auth = SqliteAuthService::new(conn)
        .validate_session(session.context("Please sign in to continue")?)?;
    ensure!(
        auth.actor.role >= role,
        "This action requires {} permission",
        role
    );
    Ok(auth.actor)
}
fn audit(conn: &Connection, who: ActorId, action: &str, data: &Value) -> Result<()> {
    SqliteAuditRepo::new(conn).record(
        &NewAuditEvent::new(action)
            .with_actor(who)
            .with_details(data.to_string()),
    )?;
    Ok(())
}

fn state(conn: &Connection, path: &Path, session: Option<&str>) -> Result<Value> {
    let actors = SqliteActorRepo::new(conn).list()?;
    let auth = session.and_then(|s| SqliteAuthService::new(conn).validate_session(s).ok());
    let can_manage = auth.as_ref().is_some_and(|s| s.actor.role == Role::Admin);
    let names=actors.iter().filter(|a|a.active||can_manage).map(|a|Ok(json!({"id":a.id,"name":a.display_name,"role":a.role,"active":a.active,"has_pin":SqliteActorRepo::new(conn).get_pin_hash(&a.id)?.is_some()}))).collect::<Result<Vec<_>>>()?;
    if auth.is_none() {
        return Ok(json!({"setup":actors.is_empty(),"actors":names,"actor":null}));
    }
    let auth = auth.unwrap();
    let inventory = SqliteInventoryService::new(conn);
    let items=SqliteItemRepo::new(conn).list(&ItemFilter{include_archived:true,search:None})?.into_iter().map(|item| {
        let balances=inventory.get_item_balances(&item.id)?;
        let total:u64=balances.iter().map(|b|u64::from(b.qty)).sum();
        Ok(json!({"item":item,"balances":balances,"total":total,"thresholds":SqliteThresholdRepo::new(conn).get_for_item(&item.id)?}))
    }).collect::<Result<Vec<_>>>()?;
    let mut stmt =
        conn.prepare("SELECT id FROM locations WHERE archived_at IS NULL ORDER BY name")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let repo = SqliteLocationRepo::new(conn);
    let locations = ids
        .into_iter()
        .map(|id| {
            let id: LocationId = id.parse()?;
            let location = repo.get_by_id(&id)?.context("Location missing")?;
            let path = repo
                .get_path(&id)?
                .into_iter()
                .map(|l| l.name)
                .collect::<Vec<_>>()
                .join(" > ");
            let storage = slim_db::repo::catalog::SqliteCatalogService::new(conn)
                .get(
                    slim_db::repo::catalog::CatalogKind::Storage,
                    &id.to_string(),
                )?
                .data;
            Ok(json!({"location":location,"path":path,"storage":storage}))
        })
        .collect::<Result<Vec<_>>>()?;
    let grid = SqliteGridRepo::new(conn);
    let bases = grid
        .list_bases(None)?
        .iter()
        .map(|base| grid.render_grid_map(&base.id))
        .collect::<slim_core::error::CoreResult<Vec<_>>>()?;
    let mut stmt=conn.prepare("SELECT o.id,o.kind,a.display_name,o.reason,o.created_at,i.name,l.name,m.delta_qty FROM operations o JOIN actors a ON a.id=o.actor_id JOIN stock_movements m ON m.operation_id=o.id JOIN items i ON i.id=m.item_id JOIN locations l ON l.id=m.location_id ORDER BY o.created_at DESC LIMIT 250")?;
    let history=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"actor":r.get::<_,String>(2)?,"reason":r.get::<_,Option<String>>(3)?,"date":r.get::<_,String>(4)?,"item":r.get::<_,String>(5)?,"location":r.get::<_,String>(6)?,"delta":r.get::<_,i64>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"setup":false,"actor":auth.actor,"actors":names,"items":items,"locations":locations,
        "low_stock":inventory.get_low_stock()?,"bases":bases,"bins":grid.list_bins()?,"history":history,
        "rules":SqliteAutomationRepo::new(conn).list_rules()?,"runs":SqliteAutomationRepo::new(conn).list_runs(50)?,
        "devices":SqliteDeviceRepo::new(conn).list_devices()?,"observations":SqliteDeviceRepo::new(conn).list_observations(false,None,100)?,
        "database":path.display().to_string(),"version":env!("CARGO_PKG_VERSION")}),
    )
}

fn dispatch(
    conn: &Connection,
    path: &Path,
    route: &str,
    data: &Value,
    session: Option<&str>,
) -> Result<Value> {
    let actors = SqliteActorRepo::new(conn);
    if route == "/api/open" {
        // Selection is an explicit native user gesture. Never accept an arbitrary
        // unauthenticated filesystem path supplied by an HTTP client.
        let Some(selected) = pick_inventory()? else {
            return Ok(json!({"cancelled":true}));
        };
        let inspection =
            Connection::open_with_flags(&selected, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let valid: bool = inspection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='stock_movements')",
            [],
            |r| r.get(0),
        )?;
        ensure!(valid, "This file is not a SLIM inventory database");
        let integrity: String = inspection.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        ensure!(
            integrity == "ok",
            "The selected database failed its integrity check"
        );
        drop(inspection);
        if let Some(dirs) = directories::ProjectDirs::from("", "", "slim") {
            std::fs::create_dir_all(dirs.data_dir())?;
            std::fs::write(
                dirs.data_dir().join("preferred-inventory.txt"),
                selected.to_string_lossy().as_bytes(),
            )?;
        }
        std::process::Command::new(std::env::current_exe()?)
            .arg("--db")
            .arg(&selected)
            .arg("app")
            .spawn()?;
        return Ok(json!({"restarted":true}));
    }
    if route == "/api/setup" {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        ensure!(actors.list()?.is_empty(), "Setup is already complete");
        let name = text(data, "name")?;
        let pin = text(data, "pin")?;
        let a = actors.create(&NewActor::new(name).with_role(Role::Admin))?;
        SqliteAuthService::new(conn).set_pin(&a.id, pin)?;
        SqliteLocationRepo::new(conn).create(&NewLocation {
            parent_id: None,
            name: "Main storage".into(),
            kind: LocationKind::Storage,
        })?;
        let login = SqliteAuthService::new(conn).login_with_pin(name, pin)?;
        tx.commit()?;
        return Ok(json!({"session":login.token}));
    }
    if route == "/api/login" {
        let name = text(data, "name")?;
        let a = actors.get_by_name(name)?.context("User not found")?;
        ensure!(a.active, "Account is inactive");
        let auth = SqliteAuthService::new(conn);
        if actors.get_pin_hash(&a.id)?.is_some() {
            return Ok(json!({"session":auth.login_with_pin(name,text(data,"pin")?)?.token}));
        }
        // Legacy accounts without PINs remain usable locally; setting a PIN requires authentication afterwards.
        let token = generate_session_token();
        let now = time::OffsetDateTime::now_utc();
        let format = time::format_description::well_known::Rfc3339;
        conn.execute(
            "INSERT INTO sessions(id,actor_id,token,created_at,expires_at) VALUES(?1,?2,?3,?4,?5)",
            params![
                Uuid::new_v4().to_string(),
                a.id.to_string(),
                token,
                now.format(&format)?,
                (now + time::Duration::hours(12)).format(&format)?
            ],
        )?;
        return Ok(json!({"session":token}));
    }
    if route == "/api/logout" {
        if let Some(token) = session {
            SqliteAuthService::new(conn).terminate_session(token)?;
        }
        return Ok(json!({"ok":true}));
    }
    let required = if route.starts_with("/api/users")
        || route.starts_with("/api/rules")
        || route == "/api/repair"
        || matches!(
            route,
            "/api/items/delete" | "/api/items/archive" | "/api/items/restore" | "/api/items/purge"
        ) {
        Role::Admin
    } else if route == "/api/stock"
        || route == "/api/scan"
        || route.starts_with("/api/observations")
        || route == "/api/locate"
    {
        Role::Operator
    } else if route == "/api/check" || route == "/api/export" {
        Role::Viewer
    } else {
        Role::Manager
    };
    let who = actor(conn, session, required)?;
    if route.starts_with("/api/catalog/") {
        use slim_db::repo::catalog::{CatalogKind, SqliteCatalogService};
        let kind: CatalogKind = serde_json::from_value(data["kind"].clone())?;
        let service = SqliteCatalogService::new(conn);
        return match route {
            "/api/catalog/list" => Ok(
                json!({"records":service.list(&who.id,kind,data["deleted"].as_bool().unwrap_or(false))?,"fields":kind.fields()}),
            ),
            "/api/catalog/save" => Ok(json!(service.save(
                &who.id,
                kind,
                optional(data, "id").as_deref(),
                &data["data"]
            )?)),
            "/api/catalog/delete" | "/api/catalog/restore" | "/api/catalog/purge" => {
                service.lifecycle(
                    &who.id,
                    kind,
                    text(data, "id")?,
                    route.rsplit('/').next().unwrap(),
                    text(data, "confirmation")?,
                )?;
                Ok(json!({"ok":true}))
            }
            _ => bail!("Unknown catalog action"),
        };
    }
    if matches!(route, "/api/devices/remove" | "/api/rules/remove") {
        use slim_db::repo::catalog::{CatalogKind, SqliteCatalogService};
        SqliteCatalogService::new(conn).lifecycle(
            &who.id,
            if route.contains("devices") {
                CatalogKind::Device
            } else {
                CatalogKind::Rule
            },
            text(data, "id")?,
            "delete",
            "DELETE",
        )?;
        return Ok(json!({"ok":true}));
    }
    // Stock services own their transactions. All other compound changes commit as one action.
    let tx = if route.starts_with("/api/users")
        || matches!(
            route,
            "/api/stock"
                | "/api/repair"
                | "/api/items/archive"
                | "/api/items/delete"
                | "/api/items/restore"
                | "/api/items/purge"
                | "/api/bins/place"
                | "/api/backup"
                | "/api/export"
                | "/api/check"
                | "/api/label"
                | "/api/scan"
        ) {
        None
    } else {
        Some(rusqlite::Transaction::new_unchecked(
            conn,
            rusqlite::TransactionBehavior::Immediate,
        )?)
    };
    let item_repo = SqliteItemRepo::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);
    let grid = SqliteGridRepo::new(conn);
    let result = match route {
        "/api/items" => {
            let item = item_repo.create(&NewItem {
                sku: optional(data, "sku"),
                name: text(data, "name")?.into(),
                unit: optional(data, "unit").unwrap_or_else(|| "each".into()),
                description: optional(data, "description"),
                metadata: None,
            })?;
            if data.get("minimum").is_some() {
                SqliteThresholdRepo::new(conn).set(&StockThreshold {
                    item_id: item.id,
                    stock_location_id: None,
                    min_qty: number(data, "minimum")?,
                })?;
            }
            json!({"id":item.id})
        }
        "/api/items/edit" => {
            let id = text(data, "id")?.parse()?;
            let item = item_repo.update(
                &id,
                &ItemUpdate {
                    sku: data.get("sku").map(|_| optional(data, "sku")),
                    name: Some(text(data, "name")?.into()),
                    unit: Some(text(data, "unit")?.into()),
                    description: Some(optional(data, "description")),
                    metadata: None,
                },
            )?;
            json!({"id":item.id})
        }
        "/api/items/archive" | "/api/items/delete" => {
            item_repo.delete_as_admin(&text(data, "id")?.parse()?, &who.id)?;
            json!({"ok":true})
        }
        "/api/items/restore" => {
            item_repo.restore_as_admin(&text(data, "id")?.parse()?, &who.id)?;
            json!({"ok":true})
        }
        "/api/items/purge" => {
            item_repo.permanently_delete_as_admin(
                &text(data, "id")?.parse()?,
                &who.id,
                text(data, "confirmation")?,
            )?;
            json!({"ok":true})
        }
        "/api/locations" => {
            let loc = loc_repo.create(&NewLocation {
                parent_id: optional(data, "parent").map(|s| s.parse()).transpose()?,
                name: text(data, "name")?.into(),
                kind: text(data, "kind")?.parse()?,
            })?;
            json!({"id":loc.id})
        }
        "/api/locations/edit" => {
            let id: LocationId = text(data, "id")?.parse()?;
            let existing = loc_repo.get_by_id(&id)?.context("Location not found")?;
            ensure!(
                existing.kind != LocationKind::Bin,
                "Manage bin placement from the Gridfinity page"
            );
            let parent: Option<LocationId> =
                optional(data, "parent").map(|v| v.parse()).transpose()?;
            if let Some(parent) = parent {
                ensure!(
                    !loc_repo.get_path(&parent)?.iter().any(|l| l.id == id),
                    "A location cannot be placed inside itself or its descendants"
                );
            }
            let kind: LocationKind = text(data, "kind")?.parse()?;
            ensure!(
                existing.kind != LocationKind::Bin || kind == LocationKind::Bin,
                "Change bin placement from Gridfinity"
            );
            if !kind.can_hold_stock() {
                ensure!(
                    SqliteInventoryService::new(conn)
                        .get_location_balances(&id)?
                        .is_empty(),
                    "Move the remaining stock before making this a structural location"
                );
            }
            conn.execute(
                "UPDATE locations SET name=?1,parent_id=?2,kind=?3,can_hold_stock=?4 WHERE id=?5",
                params![
                    text(data, "name")?,
                    parent.map(|p| p.to_string()),
                    kind.to_string(),
                    kind.can_hold_stock(),
                    id.to_string()
                ],
            )?;
            json!({"ok":true})
        }
        "/api/threshold" => {
            SqliteThresholdRepo::new(conn).set(&StockThreshold {
                item_id: text(data, "item")?.parse()?,
                stock_location_id: optional(data, "location").map(|s| s.parse()).transpose()?,
                min_qty: number(data, "minimum")?,
            })?;
            json!({"ok":true})
        }
        "/api/stock" => {
            let inv = SqliteInventoryService::new(conn);
            let item_id = text(data, "item")?.parse()?;
            let quantity = number(data, "quantity")?;
            let reason = optional(data, "reason");
            let key = Some(text(data, "request_id")?.to_owned());
            let receipt = match text(data, "kind")? {
                "receive" => inv.receive(ReceiveStock {
                    actor_id: who.id,
                    item_id,
                    destination_id: text(data, "to")?.parse()?,
                    quantity,
                    source: OperationSource::Tui,
                    reason,
                    idempotency_key: key,
                })?,
                "issue" => inv.issue(IssueStock {
                    actor_id: who.id,
                    item_id,
                    source_id: text(data, "from")?.parse()?,
                    quantity,
                    source: OperationSource::Tui,
                    reason,
                    idempotency_key: key,
                })?,
                "transfer" => inv.transfer(TransferStock {
                    actor_id: who.id,
                    item_id,
                    from_id: text(data, "from")?.parse()?,
                    to_id: text(data, "to")?.parse()?,
                    quantity,
                    source: OperationSource::Tui,
                    reason,
                    idempotency_key: key,
                })?,
                "adjust" => inv.adjust(AdjustStock {
                    actor_id: who.id,
                    item_id,
                    location_id: text(data, "from")?.parse()?,
                    new_quantity: quantity,
                    source: OperationSource::Tui,
                    reason: reason.context("A reason is required for an adjustment")?,
                    idempotency_key: key,
                })?,
                _ => bail!("Unknown stock operation"),
            };
            return Ok(serde_json::to_value(receipt)?);
        }
        "/api/bases" => {
            json!({"id":grid.create_base(&NewGridBase{location_id:text(data,"location")?.parse()?,code:text(data,"code")?.into(),cols:number(data,"cols")?,rows:number(data,"rows")?,pitch_mm:None})?.id})
        }
        "/api/bins" => {
            let code = text(data, "code")?;
            let parent = optional(data, "parent").map(|s| s.parse()).transpose()?;
            let loc = loc_repo.create(&NewLocation {
                parent_id: parent,
                name: code.into(),
                kind: LocationKind::Bin,
            })?;
            let bin = grid.create_bin(
                &NewGridfinityBin {
                    bin_code: code.into(),
                    width_units: number(data, "width")?,
                    depth_units: number(data, "depth")?,
                    height_units: number(data, "height")?,
                    compartment_count: number(data, "compartments")?,
                    parent_location_id: parent,
                },
                &loc.id,
            )?;
            json!({"id":bin.id})
        }
        "/api/bins/place" => {
            let placed = grid.place_bin(&PlaceBinCmd {
                bin_id: text(data, "bin")?.parse()?,
                base_id: text(data, "base")?.parse()?,
                x: number(data, "x")?,
                y: number(data, "y")?,
                rotation: text(data, "rotation")?.parse()?,
            })?;
            json!(placed)
        }
        "/api/bins/remove" => {
            grid.remove_bin(&text(data, "bin")?.parse()?)?;
            json!({"ok":true})
        }
        "/api/scan" => json!(SqliteScanResolver::new(conn).resolve_scan(text(data, "code")?)?),
        "/api/identifier" => {
            let target = if let Some(id) = optional(data, "item") {
                IdentifierTarget::Item(id.parse()?)
            } else {
                IdentifierTarget::Location(text(data, "location")?.parse()?)
            };
            json!(SqliteIdentifierRepo::new(conn).create(&NewIdentifier {
                kind: text(data, "kind")?.parse()?,
                value: text(data, "value")?.into(),
                target
            })?)
        }
        "/api/label" => {
            let id: ItemId = text(data, "item")?.parse()?;
            let item = item_repo.get_by_id(&id)?.context("Item not found")?;
            json!({"svg":slim_core::label::generate_svg_qr(&slim_core::model::scan::item_uri(&item.id.to_string()))?})
        }
        "/api/locate" => json!(SqliteLedRepo::new(conn).locate_target(
            text(data, "target")?,
            Some(30),
            time::OffsetDateTime::now_utc()
        )?),
        "/api/users" => {
            let user = actors.create_as_admin(
                &who.id,
                &NewActor::new(text(data, "name")?).with_role(text(data, "role")?.parse()?),
                Some(text(data, "pin")?),
            )?;
            json!({"id":user.id})
        }
        "/api/users/edit" => {
            let id = text(data, "id")?.parse()?;
            let changes = slim_core::model::actor::ActorUpdate {
                display_name: Some(text(data, "name")?.into()),
                role: Some(text(data, "role")?.parse()?),
                pin: optional(data, "pin"),
                ..Default::default()
            };
            json!({"user":actors.update_as_admin(&who.id,&id,&changes)?})
        }
        "/api/users/remove" | "/api/users/restore" => {
            actors.update_as_admin(
                &who.id,
                &text(data, "id")?.parse()?,
                &slim_core::model::actor::ActorUpdate {
                    active: Some(route.ends_with("restore")),
                    ..Default::default()
                },
            )?;
            json!({"ok":true})
        }
        "/api/users/role" => {
            let id: ActorId = text(data, "id")?.parse()?;
            let role: Role = text(data, "role")?.parse()?;
            actors.update_as_admin(
                &who.id,
                &id,
                &slim_core::model::actor::ActorUpdate {
                    role: Some(role),
                    ..Default::default()
                },
            )?;
            json!({"ok":true})
        }
        "/api/users/pin" => {
            actors.update_as_admin(
                &who.id,
                &text(data, "id")?.parse()?,
                &slim_core::model::actor::ActorUpdate {
                    pin: Some(text(data, "pin")?.into()),
                    ..Default::default()
                },
            )?;
            json!({"ok":true})
        }
        "/api/users/purge" => {
            actors.permanently_delete_user(
                &who.id,
                &text(data, "id")?.parse()?,
                text(data, "confirmation")?,
            )?;
            json!({"ok":true})
        }
        "/api/rules" => {
            let rule = AutomationRule {
                id: optional(data, "id").unwrap_or_else(|| Uuid::new_v4().to_string()),
                name: Some(text(data, "name")?.into()),
                enabled: true,
                event: text(data, "event")?.into(),
                item_sku: optional(data, "sku"),
                location_name: None,
                condition: None,
                action: match text(data, "action")? {
                    "webhook" => RuleAction::Webhook {
                        url: text(data, "url")?.into(),
                        payload: None,
                    },
                    "notify" => RuleAction::Notify {
                        message: text(data, "message")?.into(),
                    },
                    _ => bail!("Select notification or webhook"),
                },
            };
            SqliteAutomationRepo::new(conn).save_rule(&rule)?;
            json!({"ok":true})
        }
        "/api/rules/remove" => {
            SqliteAutomationRepo::new(conn).delete_rule(text(data, "id")?)?;
            json!({"ok":true})
        }
        "/api/rules/toggle" => {
            ensure!(
                conn.execute(
                    "UPDATE automation_rules SET enabled=NOT enabled WHERE id=?1",
                    [text(data, "id")?]
                )? == 1,
                "Rule not found"
            );
            json!({"ok":true})
        }
        "/api/devices" => json!(SqliteDeviceRepo::new(conn).register_device(&NewDevice {
            id: text(data, "id")?.into(),
            name: text(data, "name")?.into(),
            kind: text(data, "kind")?.parse()?,
            port_or_address: optional(data, "port"),
            location_id: optional(data, "location").map(|s| s.parse()).transpose()?,
            settings_json: None
        })?),
        "/api/devices/remove" => {
            SqliteDeviceRepo::new(conn).delete_device(text(data, "id")?)?;
            json!({"ok":true})
        }
        "/api/observations/confirm" => {
            SqliteDeviceRepo::new(conn).confirm_observation(
                &ObservationId(text(data, "id")?.parse()?),
                &who.id,
                None,
            )?;
            json!({"ok":true})
        }
        "/api/observations/reject" => {
            SqliteDeviceRepo::new(conn)
                .reject_observation(&ObservationId(text(data, "id")?.parse()?), &who.id)?;
            json!({"ok":true})
        }
        "/api/check" => json!(SqliteReconciliationService::new(conn).check()?),
        "/api/repair" => {
            let result =
                SqliteReconciliationService::new(conn).repair(&who.id, text(data, "reason")?)?;
            return Ok(json!(result));
        }
        "/api/backup" => {
            let folder = path.parent().unwrap_or(Path::new(".")).join("backups");
            std::fs::create_dir_all(&folder)?;
            let backup = folder.join(format!("slim-{}.sqlite3", Uuid::new_v4()));
            conn.backup(rusqlite::DatabaseName::Main, &backup, None)?;
            json!({"path":backup.display().to_string()})
        }
        "/api/export" => return state(conn, path, session),
        _ => bail!("Unknown action"),
    };
    if tx.is_some() {
        audit(conn, who.id, route, &json!({"result":result}))?;
    }
    if let Some(tx) = tx {
        tx.commit()?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Connection, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_connection(&dir.path().join("inventory.sqlite3")).unwrap();
        migrate::run_migrations(&conn).unwrap();
        let setup = dispatch(
            &conn,
            &dir.path().join("inventory.sqlite3"),
            "/api/setup",
            &json!({"name":"Owner","pin":"secure-test-pin"}),
            None,
        )
        .unwrap();
        (conn, dir, setup["session"].as_str().unwrap().into())
    }
    #[test]
    fn custom_storage_and_catalog_lifecycle_routes_enforce_admin_deletion() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        let r=dispatch(&c,&path,"/api/catalog/save",&json!({"kind":"storage","data":{"name":"Travel box","kind":"custom","type_label":"Plastic case","width_mm":"300"}}),Some(&s)).unwrap();
        let location = r["id"].as_str().unwrap();
        let state = state(&c, &path, Some(&s)).unwrap();
        assert!(
            state["locations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["storage"]["type_label"] == "Plastic case")
        );
        let user = dispatch(
            &c,
            &path,
            "/api/users",
            &json!({"name":"Staff","role":"manager","pin":"test-pin"}),
            Some(&s),
        )
        .unwrap();
        let login = dispatch(
            &c,
            &path,
            "/api/login",
            &json!({"name":"Staff","pin":"test-pin"}),
            None,
        )
        .unwrap();
        let input = json!({"kind":"storage","id":location,"confirmation":"DELETE"});
        assert!(
            dispatch(
                &c,
                &path,
                "/api/catalog/delete",
                &input,
                login["session"].as_str()
            )
            .is_err()
        );
        dispatch(&c, &path, "/api/catalog/delete", &input, Some(&s)).unwrap();
        dispatch(
            &c,
            &path,
            "/api/catalog/purge",
            &json!({"kind":"storage","id":location,"confirmation":"DELETE PERMANENTLY"}),
            Some(&s),
        )
        .unwrap();
        dispatch(&c, &path, "/api/users/remove", &user, Some(&s)).unwrap();
        dispatch(
            &c,
            &path,
            "/api/users/purge",
            &json!({"id":user["id"],"confirmation":"DELETE PERMANENTLY"}),
            Some(&s),
        )
        .unwrap();
        assert!(
            SqliteActorRepo::new(&c)
                .get_by_name("Staff")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn permanent_delete_requires_admin_deleted_item_and_confirmation() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        let item = dispatch(
            &c,
            &path,
            "/api/items",
            &json!({"name":"Purge test"}),
            Some(&s),
        )
        .unwrap();
        let input = json!({"id":item["id"],"confirmation":"DELETE PERMANENTLY"});
        assert!(dispatch(&c, &path, "/api/items/purge", &input, Some(&s)).is_err());
        dispatch(&c, &path, "/api/items/delete", &item, Some(&s)).unwrap();
        let user = dispatch(
            &c,
            &path,
            "/api/users",
            &json!({"name":"Manager","role":"manager","pin":"4321"}),
            Some(&s),
        )
        .unwrap();
        assert!(!user["id"].is_null());
        let login = dispatch(
            &c,
            &path,
            "/api/login",
            &json!({"name":"Manager","pin":"4321"}),
            None,
        )
        .unwrap();
        assert!(
            dispatch(
                &c,
                &path,
                "/api/items/purge",
                &input,
                login["session"].as_str()
            )
            .is_err()
        );
        assert!(dispatch(&c, &path, "/api/items/purge", &item, Some(&s)).is_err());
        dispatch(&c, &path, "/api/items/purge", &input, Some(&s)).unwrap();
        let id = item["id"].as_str().unwrap().parse().unwrap();
        assert!(SqliteItemRepo::new(&c).get_by_id(&id).unwrap().is_none());
        assert!(dispatch(&c, &path, "/api/items/restore", &item, Some(&s)).is_err());
    }
    #[test]
    fn user_management_is_admin_only_and_revokes_old_sessions() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        let user = dispatch(
            &c,
            &path,
            "/api/users",
            &json!({"name":"Staff","role":"manager","pin":"4321"}),
            Some(&s),
        )
        .unwrap();
        let login = dispatch(
            &c,
            &path,
            "/api/login",
            &json!({"name":"Staff","pin":"4321"}),
            None,
        )
        .unwrap();
        let token = login["session"].as_str().unwrap();
        for route in [
            "/api/users/edit",
            "/api/users/remove",
            "/api/users/restore",
            "/api/users/pin",
            "/api/users/role",
        ] {
            assert!(
                dispatch(
                    &c,
                    &path,
                    route,
                    &json!({"id":user["id"],"name":"Changed","role":"admin","pin":"5678"}),
                    Some(token)
                )
                .is_err()
            );
        }
        dispatch(
            &c,
            &path,
            "/api/users/edit",
            &json!({"id":user["id"],"name":"Renamed","role":"operator","pin":"5678"}),
            Some(&s),
        )
        .unwrap();
        assert!(state(&c, &path, Some(token)).unwrap()["actor"].is_null());
        assert!(
            dispatch(
                &c,
                &path,
                "/api/login",
                &json!({"name":"Renamed","pin":"4321"}),
                None
            )
            .is_err()
        );
        dispatch(&c, &path, "/api/users/remove", &user, Some(&s)).unwrap();
        assert!(
            dispatch(
                &c,
                &path,
                "/api/login",
                &json!({"name":"Renamed","pin":"5678"}),
                None
            )
            .is_err()
        );
        dispatch(&c, &path, "/api/users/restore", &user, Some(&s)).unwrap();
        assert!(
            dispatch(
                &c,
                &path,
                "/api/login",
                &json!({"name":"Renamed","pin":"5678"}),
                None
            )
            .is_ok()
        );
        let owner = SqliteActorRepo::new(&c)
            .get_by_name("Owner")
            .unwrap()
            .unwrap();
        assert!(
            dispatch(
                &c,
                &path,
                "/api/users/remove",
                &json!({"id":owner.id}),
                Some(&s)
            )
            .is_err()
        );
        dispatch(
            &c,
            &path,
            "/api/users/pin",
            &json!({"id":owner.id,"pin":"new-owner-pin"}),
            Some(&s),
        )
        .unwrap();
        assert!(state(&c, &path, Some(&s)).unwrap()["actor"].is_null());
    }
    #[test]
    fn delete_archive_and_restore_require_admin() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        let item = dispatch(
            &c,
            &path,
            "/api/items",
            &json!({"name":"Test item"}),
            Some(&s),
        )
        .unwrap();
        for role in [Role::Viewer, Role::Operator, Role::Manager] {
            let user = SqliteActorRepo::new(&c)
                .create(&NewActor::new(role.to_string()).with_role(role))
                .unwrap();
            SqliteAuthService::new(&c)
                .set_pin(&user.id, "test-pin")
                .unwrap();
            let token = dispatch(
                &c,
                &path,
                "/api/login",
                &json!({"name":role.to_string(),"pin":"test-pin"}),
                None,
            )
            .unwrap();
            let token = token["session"].as_str().unwrap();
            for route in [
                "/api/items/archive",
                "/api/items/delete",
                "/api/items/restore",
            ] {
                assert!(dispatch(&c, &path, route, &item, Some(token)).is_err());
            }
        }
        dispatch(&c, &path, "/api/items/delete", &item, Some(&s)).unwrap();
        let id = item["id"].as_str().unwrap().parse().unwrap();
        assert!(
            SqliteItemRepo::new(&c)
                .get_by_id(&id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_some()
        );
        dispatch(&c, &path, "/api/items/restore", &item, Some(&s)).unwrap();
        assert!(
            SqliteItemRepo::new(&c)
                .get_by_id(&id)
                .unwrap()
                .unwrap()
                .archived_at
                .is_none()
        );
    }
    #[test]
    fn onboarding_and_inventory_flow() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        let initial = state(&c, &path, Some(&s)).unwrap();
        assert_eq!(initial["actor"]["role"], "admin");
        let location = initial["locations"][0]["location"]["id"].as_str().unwrap();
        let item = dispatch(
            &c,
            &path,
            "/api/items",
            &json!({"name":"Fasteners","sku":"M3","minimum":5}),
            Some(&s),
        )
        .unwrap();
        let request = json!({"kind":"receive","item":item["id"],"to":location,"quantity":12,"request_id":"receive-1"});
        dispatch(&c, &path, "/api/stock", &request, Some(&s)).unwrap();
        dispatch(&c, &path, "/api/stock", &request, Some(&s)).unwrap();
        let after = state(&c, &path, Some(&s)).unwrap();
        assert_eq!(after["items"][0]["total"], 12);
        assert!(
            dispatch(
                &c,
                &path,
                "/api/items/archive",
                &json!({"id":item["id"]}),
                Some(&s)
            )
            .is_err()
        );
        dispatch(
            &c,
            &path,
            "/api/items/edit",
            &json!({"id":item["id"],"name":"M3 screws","unit":"each"}),
            Some(&s),
        )
        .unwrap();
        let backup = dispatch(&c, &path, "/api/backup", &json!({}), Some(&s)).unwrap();
        assert!(Path::new(backup["path"].as_str().unwrap()).exists());
        assert!(
            SqliteReconciliationService::new(&c)
                .check()
                .unwrap()
                .mismatches
                .is_empty()
        );
    }
    #[test]
    fn anonymous_and_viewer_mutations_are_denied() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        assert!(dispatch(&c, &path, "/api/items", &json!({"name":"Forbidden"}), None).is_err());
        dispatch(
            &c,
            &path,
            "/api/users",
            &json!({"name":"Read only","role":"viewer","pin":"view-pin"}),
            Some(&s),
        )
        .unwrap();
        let login = dispatch(
            &c,
            &path,
            "/api/login",
            &json!({"name":"Read only","pin":"view-pin"}),
            None,
        )
        .unwrap();
        let token = login["session"].as_str().unwrap();
        assert!(
            dispatch(
                &c,
                &path,
                "/api/items",
                &json!({"name":"Forbidden"}),
                Some(token)
            )
            .is_err()
        );
        assert!(
            dispatch(
                &c,
                &path,
                "/api/users/role",
                &json!({"id":state(&c,&path,Some(token)).unwrap()["actor"]["id"],"role":"admin"}),
                Some(token)
            )
            .is_err()
        );
        assert!(state(&c, &path, None).unwrap().get("items").is_none());
    }
    #[test]
    fn invalid_compound_forms_roll_back_and_setup_is_once() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        assert!(
            dispatch(
                &c,
                &path,
                "/api/setup",
                &json!({"name":"Intruder","pin":"1234"}),
                None
            )
            .is_err()
        );
        assert!(
            dispatch(
                &c,
                &path,
                "/api/bins",
                &json!({"code":"BAD","width":0,"depth":1,"height":3,"compartments":1}),
                Some(&s)
            )
            .is_err()
        );
        assert!(
            SqliteLocationRepo::new(&c)
                .find_by_name("BAD", None)
                .unwrap()
                .is_none()
        );
        assert!(
            dispatch(
                &c,
                &path,
                "/api/users",
                &json!({"name":"Bad PIN","role":"admin","pin":"1"}),
                Some(&s)
            )
            .is_err()
        );
        assert!(
            SqliteActorRepo::new(&c)
                .get_by_name("Bad PIN")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn wrong_pin_and_logged_out_session_are_rejected() {
        let (c, d, s) = fixture();
        let path = d.path().join("inventory.sqlite3");
        assert!(
            dispatch(
                &c,
                &path,
                "/api/login",
                &json!({"name":"Owner","pin":"wrong"}),
                None
            )
            .is_err()
        );
        dispatch(&c, &path, "/api/logout", &json!({}), Some(&s)).unwrap();
        assert!(state(&c, &path, Some(&s)).unwrap()["actor"].is_null());
        assert!(
            dispatch(
                &c,
                &path,
                "/api/items",
                &json!({"name":"Forbidden"}),
                Some(&s)
            )
            .is_err()
        );
    }
}
