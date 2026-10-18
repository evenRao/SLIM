//! Shared, authorized lifecycle and editors for mutable catalog records.
use super::{
    actor::SqliteActorRepo, audit::SqliteAuditRepo, automation::SqliteAutomationRepo,
    device::SqliteDeviceRepo, grid::SqliteGridRepo, location::SqliteLocationRepo,
};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slim_core::{
    model::{
        actor::{ActorId, Role},
        auth::NewAuditEvent,
        automation::*,
        device::*,
        gridfinity::{NewGridBase, NewGridfinityBin},
        identifier::*,
        location::{LocationKind, NewLocation},
    },
    service::*,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Storage,
    Base,
    Bin,
    Rule,
    Device,
    Barcode,
}
pub const KINDS: [CatalogKind; 6] = [
    CatalogKind::Storage,
    CatalogKind::Base,
    CatalogKind::Bin,
    CatalogKind::Rule,
    CatalogKind::Device,
    CatalogKind::Barcode,
];
impl CatalogKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::Base => "base",
            Self::Bin => "bin",
            Self::Rule => "rule",
            Self::Device => "device",
            Self::Barcode => "barcode",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Storage => "Storage",
            Self::Base => "Gridfinity bases",
            Self::Bin => "Gridfinity bins",
            Self::Rule => "Automation rules",
            Self::Device => "Devices",
            Self::Barcode => "Barcode links",
        }
    }
    fn table(self) -> &'static str {
        match self {
            Self::Storage => "locations",
            Self::Base => "grid_bases",
            Self::Bin => "gridfinity_bins",
            Self::Rule => "automation_rules",
            Self::Device => "devices",
            Self::Barcode => "identifiers",
        }
    }
    pub fn fields(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            Self::Storage => &[
                ("name", "Name", ""),
                ("kind", "Kind (custom/shelf/drawer/room/cabinet)", "custom"),
                ("parent", "Parent name, full path, or UUID (optional)", ""),
                ("type_label", "Custom type, e.g. toolbox (optional)", ""),
                ("notes", "Notes (optional)", ""),
                ("width_mm", "Width mm (optional)", ""),
                ("depth_mm", "Depth mm (optional)", ""),
                ("height_mm", "Height mm (optional)", ""),
            ],
            Self::Base => &[
                ("name", "Base code", ""),
                ("location", "Storage name, full path, or UUID", ""),
                ("cols", "Columns (1-100)", "6"),
                ("rows", "Rows (1-100)", "4"),
            ],
            Self::Bin => &[
                ("name", "Bin code", ""),
                ("parent", "Parent storage (optional)", ""),
                ("width", "Width units", "1"),
                ("depth", "Depth units", "1"),
                ("height", "Height units", "3"),
                ("compartments", "Compartments", "1"),
            ],
            Self::Rule => &[
                ("name", "Name", ""),
                (
                    "event",
                    "Event, e.g. StockBelowThreshold",
                    "StockBelowThreshold",
                ),
                ("sku", "Item SKU (optional)", ""),
                ("action", "Action: notify or webhook", "notify"),
                ("message", "Notification message (optional)", ""),
                ("url", "Webhook URL (optional)", ""),
            ],
            Self::Device => &[
                ("name", "Name", ""),
                ("kind", "Kind", "mock_sensor"),
                ("port", "Port or address (optional)", ""),
                ("location", "Storage (optional)", ""),
            ],
            Self::Barcode => &[
                ("name", "Barcode or tag value", ""),
                ("kind", "Kind: barcode/qr/nfc/rfid/manual", "barcode"),
                ("item", "Item UUID or SKU (leave blank for storage)", ""),
                ("location", "Storage (leave blank for item)", ""),
            ],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRecord {
    pub id: String,
    pub name: String,
    pub deleted: bool,
    pub data: Value,
}
pub struct SqliteCatalogService<'a> {
    conn: &'a Connection,
}
impl<'a> SqliteCatalogService<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
    fn authorize(&self, actor: &ActorId, role: Role) -> Result<()> {
        let a = SqliteActorRepo::new(self.conn)
            .get_by_id(actor)?
            .context("Sign in again")?;
        ensure!(a.active && a.role >= role, "{} permission required", role);
        Ok(())
    }
    pub fn list(
        &self,
        actor: &ActorId,
        kind: CatalogKind,
        deleted: bool,
    ) -> Result<Vec<CatalogRecord>> {
        self.authorize(actor, if deleted { Role::Admin } else { Role::Manager })?;
        let mut statement = self.conn.prepare(&format!(
            "SELECT id FROM {} WHERE archived_at IS {}NULL {} ORDER BY rowid",
            kind.table(),
            if deleted { "NOT " } else { "" },
            if kind == CatalogKind::Storage {
                "AND id NOT IN (SELECT location_id FROM gridfinity_bins)"
            } else {
                ""
            }
        ))?;
        let ids = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter().map(|id| self.get(kind, id)).collect()
    }
    pub fn get(&self, kind: CatalogKind, id: &str) -> Result<CatalogRecord> {
        let deleted = self
            .conn
            .query_row(
                &format!(
                    "SELECT archived_at IS NOT NULL FROM {} WHERE id=?1",
                    kind.table()
                ),
                [id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .context("Record not found")?;
        let data=match kind {
   CatalogKind::Storage=>{let l=SqliteLocationRepo::new(self.conn).get_by_id(&id.parse()?)?.context("Storage not found")?;
    let mut data=json!({"name":l.name,"kind":l.kind.to_string(),"parent":l.parent_id.map(|p|p.to_string()).unwrap_or_default()});
    let profile=self.conn.query_row("SELECT type_label,notes,width_mm,depth_mm,height_mm FROM storage_profiles WHERE location_id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<f64>>(2)?,r.get::<_,Option<f64>>(3)?,r.get::<_,Option<f64>>(4)?))).optional()?;
    if let Some((label,notes,w,d,h))=profile {data["type_label"]=json!(label);data["notes"]=json!(notes);data["width_mm"]=json!(w);data["depth_mm"]=json!(d);data["height_mm"]=json!(h);}data
   },
   CatalogKind::Base=>{let b=SqliteGridRepo::new(self.conn).get_base_by_id(&id.parse()?)?.context("Base not found")?;json!({"name":b.code,"location":b.location_id,"cols":b.cols,"rows":b.rows})},
   CatalogKind::Bin=>{let b=SqliteGridRepo::new(self.conn).get_bin_by_id(&id.parse()?)?.context("Bin not found")?;let l=SqliteLocationRepo::new(self.conn).get_by_id(&b.location_id)?.context("Storage not found")?;json!({"name":b.bin_code,"parent":l.parent_id.map(|p|p.to_string()).unwrap_or_default(),"width":b.width_units,"depth":b.depth_units,"height":b.height_units,"compartments":b.compartment_count})},
   CatalogKind::Rule=>{let r=SqliteAutomationRepo::new(self.conn).get_rule(id)?.context("Rule not found")?;let (action,message,url)=match r.action {RuleAction::Notify{message}=>("notify",message,String::new()),RuleAction::Webhook{url,..}=>("webhook",String::new(),url),_=>("command",String::new(),String::new())};json!({"name":r.name.unwrap_or_else(||id.into()),"event":r.event,"sku":r.item_sku.unwrap_or_default(),"action":action,"message":message,"url":url})},
   CatalogKind::Device=>{let d=SqliteDeviceRepo::new(self.conn).get_device(id)?.context("Device not found")?;json!({"name":d.name,"kind":d.kind.to_string(),"port":d.port_or_address.unwrap_or_default(),"location":d.location_id.map(|l|l.to_string()).unwrap_or_default()})},
   CatalogKind::Barcode=>self.conn.query_row("SELECT value,kind,item_id,location_id FROM identifiers WHERE id=?1",[id],|r|Ok(json!({"name":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"item":r.get::<_,Option<String>>(2)?.unwrap_or_default(),"location":r.get::<_,Option<String>>(3)?.unwrap_or_default()})))?,
  };
        Ok(CatalogRecord {
            id: id.into(),
            name: text(&data, "name").into(),
            deleted,
            data,
        })
    }
    fn location(&self, value: &str) -> Result<slim_core::model::location::LocationId> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM locations WHERE archived_at IS NULL")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let repo = SqliteLocationRepo::new(self.conn);
        let mut matches = Vec::new();
        for id in ids {
            let parsed = id.parse()?;
            let path = repo.get_path(&parsed)?;
            if id == value
                || path
                    .last()
                    .is_some_and(|l| l.name.eq_ignore_ascii_case(value))
                || path
                    .iter()
                    .map(|l| l.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" > ")
                    .eq_ignore_ascii_case(value)
            {
                matches.push(parsed);
            }
        }
        ensure!(
            matches.len() == 1,
            "Choose a unique active storage name, full path, or UUID"
        );
        Ok(matches[0])
    }
    pub fn save(
        &self,
        actor: &ActorId,
        kind: CatalogKind,
        id: Option<&str>,
        data: &Value,
    ) -> Result<CatalogRecord> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.authorize(
            actor,
            if kind == CatalogKind::Rule {
                Role::Admin
            } else {
                Role::Manager
            },
        )?;
        if let Some(id) = id {
            ensure!(
                !self.get(kind, id)?.deleted,
                "Restore this record before editing"
            );
        }
        let mut merged = if let Some(id) = id {
            self.get(kind, id)?.data
        } else {
            json!({})
        };
        ensure!(data.is_object(), "Record fields must be an object");
        for (key, value) in data.as_object().unwrap() {
            merged[key] = value.clone();
        }
        let data = &merged;
        let name = text(data, "name");
        ensure!(!name.is_empty(), "Name is required");
        let now = super::actor::format_time(time::OffsetDateTime::now_utc());
        let record_id = id
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        match kind {
            CatalogKind::Storage => {
                let raw_kind = text(data, "kind");
                let storage_kind = raw_kind
                    .parse::<LocationKind>()
                    .unwrap_or(LocationKind::Custom);
                let parent = if text(data, "parent").is_empty() {
                    None
                } else {
                    Some(self.location(text(data, "parent"))?)
                };
                if let Some(parent) = parent {
                    let path = SqliteLocationRepo::new(self.conn).get_path(&parent)?;
                    ensure!(
                        !path.iter().any(|l| Some(l.id.to_string().as_str()) == id),
                        "Storage cannot be moved inside itself or its descendants"
                    );
                }
                if let Some(id) = id {
                    ensure!(
                        !self.exists(
                            "SELECT EXISTS(SELECT 1 FROM gridfinity_bins WHERE location_id=?1)",
                            id
                        )?,
                        "Edit Gridfinity bins through the Bins editor"
                    );
                    if !storage_kind.can_hold_stock() {
                        ensure!(!self.exists("SELECT EXISTS(SELECT 1 FROM stock_balances WHERE location_id=?1 AND qty<>0)",id)?,"Move stock before changing to structural storage");
                    }
                    self.conn.execute("UPDATE locations SET name=?1,kind=?2,can_hold_stock=?3,parent_id=?4,updated_at=?5 WHERE id=?6",params![name,storage_kind.to_string(),storage_kind.can_hold_stock(),parent.map(|p|p.to_string()),now,id])?;
                } else {
                    self.conn.execute("INSERT INTO locations(id,name,kind,can_hold_stock,parent_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",params![record_id,name,storage_kind.to_string(),storage_kind.can_hold_stock(),parent.map(|p|p.to_string()),now])?;
                }
                let label = if text(data, "type_label").is_empty()
                    && storage_kind == LocationKind::Custom
                    && raw_kind != "custom"
                {
                    raw_kind
                } else {
                    text(data, "type_label")
                };
                self.conn.execute("INSERT INTO storage_profiles(location_id,type_label,notes,width_mm,depth_mm,height_mm) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(location_id) DO UPDATE SET type_label=excluded.type_label,notes=excluded.notes,width_mm=excluded.width_mm,depth_mm=excluded.depth_mm,height_mm=excluded.height_mm",params![record_id,label,text(data,"notes"),dimension(data,"width_mm")?,dimension(data,"depth_mm")?,dimension(data,"height_mm")?])?;
            }
            CatalogKind::Base => {
                let location = self.location(text(data, "location"))?;
                let base = NewGridBase {
                    location_id: location,
                    code: name.into(),
                    cols: number(data, "cols")?,
                    rows: number(data, "rows")?,
                    pitch_mm: None,
                };
                base.validate()?;
                if let Some(id) = id {
                    let old = SqliteGridRepo::new(self.conn)
                        .get_base_by_id(&id.parse()?)?
                        .context("Base not found")?;
                    if old.cols != base.cols || old.rows != base.rows || old.location_id != location
                    {
                        ensure!(!self.exists("SELECT EXISTS(SELECT 1 FROM bin_placements WHERE base_id=?1 AND removed_at IS NULL)",id)?,"Unplace bins before changing a base's dimensions or location");
                    }
                    self.conn.execute(
                        "UPDATE grid_bases SET code=?1,location_id=?2,cols=?3,rows=?4 WHERE id=?5",
                        params![name, location.to_string(), base.cols, base.rows, id],
                    )?;
                } else {
                    self.conn.execute("INSERT INTO grid_bases(id,location_id,code,cols,rows,pitch_mm,created_at) VALUES(?1,?2,?3,?4,?5,42,?6)",params![record_id,location.to_string(),name,base.cols,base.rows,now])?;
                }
            }
            CatalogKind::Bin => {
                let parent = if text(data, "parent").is_empty() {
                    None
                } else {
                    Some(self.location(text(data, "parent"))?)
                };
                let bin = NewGridfinityBin {
                    parent_location_id: parent,
                    bin_code: name.into(),
                    width_units: number(data, "width")?,
                    depth_units: number(data, "depth")?,
                    height_units: number(data, "height")?,
                    compartment_count: number(data, "compartments")?,
                };
                bin.validate()?;
                if let Some(id) = id {
                    let old = SqliteGridRepo::new(self.conn)
                        .get_bin_by_id(&id.parse()?)?
                        .context("Bin not found")?;
                    let old_parent = SqliteLocationRepo::new(self.conn)
                        .get_by_id(&old.location_id)?
                        .context("Bin storage not found")?
                        .parent_id;
                    if old.width_units != bin.width_units
                        || old.depth_units != bin.depth_units
                        || old.height_units != bin.height_units
                        || old.compartment_count != bin.compartment_count
                        || old_parent != parent
                    {
                        ensure!(!self.exists("SELECT EXISTS(SELECT 1 FROM bin_placements WHERE bin_id=?1 AND removed_at IS NULL)",id)?,"Unplace the bin before editing its dimensions or parent");
                    }
                    if let Some(parent) = parent {
                        ensure!(
                            !SqliteLocationRepo::new(self.conn)
                                .get_path(&parent)?
                                .iter()
                                .any(|l| l.id == old.location_id),
                            "A bin cannot contain itself"
                        );
                    }
                    self.conn.execute("UPDATE gridfinity_bins SET bin_code=?1,width_units=?2,depth_units=?3,height_units=?4,compartment_count=?5 WHERE id=?6",params![name,bin.width_units,bin.depth_units,bin.height_units,bin.compartment_count,id])?;
                    self.conn.execute(
                        "UPDATE locations SET name=?1,parent_id=?2,updated_at=?3 WHERE id=?4",
                        params![
                            name,
                            parent.map(|p| p.to_string()),
                            now,
                            old.location_id.to_string()
                        ],
                    )?;
                } else {
                    let location = SqliteLocationRepo::new(self.conn).create(&NewLocation {
                        name: name.into(),
                        kind: LocationKind::Bin,
                        parent_id: parent,
                    })?;
                    self.conn.execute("INSERT INTO gridfinity_bins(id,location_id,bin_code,width_units,depth_units,height_units,compartment_count,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![record_id,location.id.to_string(),name,bin.width_units,bin.depth_units,bin.height_units,bin.compartment_count,now])?;
                }
            }
            CatalogKind::Rule => {
                let action = match text(data, "action") {
                    "notify" => RuleAction::Notify {
                        message: text(data, "message").into(),
                    },
                    "webhook" => {
                        let url = text(data, "url");
                        ensure!(
                            url.starts_with("http://") || url.starts_with("https://"),
                            "Enter an http or https webhook URL"
                        );
                        RuleAction::Webhook {
                            url: url.into(),
                            payload: None,
                        }
                    }
                    _ => bail!("Choose notify or webhook"),
                };
                ensure!(!text(data, "event").is_empty(), "Event is required");
                let old = id
                    .map(|id| SqliteAutomationRepo::new(self.conn).get_rule(id))
                    .transpose()?
                    .flatten();
                SqliteAutomationRepo::new(self.conn).save_rule(&AutomationRule {
                    id: record_id.clone(),
                    name: Some(name.into()),
                    enabled: old.as_ref().map(|r| r.enabled).unwrap_or(true),
                    event: text(data, "event").into(),
                    item_sku: optional(data, "sku"),
                    location_name: old.as_ref().and_then(|r| r.location_name.clone()),
                    condition: old.and_then(|r| r.condition),
                    action,
                })?;
            }
            CatalogKind::Device => {
                let location = if text(data, "location").is_empty() {
                    None
                } else {
                    Some(self.location(text(data, "location"))?)
                };
                let kind: DeviceKind = text(data, "kind").parse()?;
                if let Some(id) = id {
                    self.conn.execute("UPDATE devices SET name=?1,kind=?2,port_or_address=?3,location_id=?4,updated_at=?5 WHERE id=?6",params![name,kind.to_string(),optional(data,"port"),location.map(|l|l.to_string()),now,id])?;
                } else {
                    SqliteDeviceRepo::new(self.conn).register_device(&NewDevice {
                        id: record_id.clone(),
                        name: name.into(),
                        kind,
                        port_or_address: optional(data, "port"),
                        location_id: location,
                        settings_json: None,
                    })?;
                }
            }
            CatalogKind::Barcode => {
                let item = text(data, "item");
                let location = text(data, "location");
                ensure!(
                    item.is_empty() != location.is_empty(),
                    "Choose either an item or storage target"
                );
                let item = if item.is_empty() {
                    None
                } else {
                    let repo = super::item::SqliteItemRepo::new(self.conn);
                    let found = if let Ok(id) = item.parse() {
                        repo.get_by_id(&id)?
                    } else {
                        repo.get_by_sku(item)?
                    };
                    Some(
                        found
                            .filter(|i| i.archived_at.is_none())
                            .context("Active item not found")?
                            .id,
                    )
                };
                let location = if location.is_empty() {
                    None
                } else {
                    Some(self.location(location)?)
                };
                let kind: IdentifierKind = text(data, "kind").parse()?;
                if let Some(id) = id {
                    self.conn.execute("UPDATE identifiers SET value=?1,kind=?2,item_id=?3,location_id=?4 WHERE id=?5",params![name,kind.to_string(),item.map(|i|i.to_string()),location.map(|l|l.to_string()),id])?;
                } else {
                    self.conn.execute("INSERT INTO identifiers(id,value,kind,item_id,location_id,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![record_id,name,kind.to_string(),item.map(|i|i.to_string()),location.map(|l|l.to_string()),now])?;
                }
            }
        }
        self.audit(actor, "catalog.saved", kind, &record_id)?;
        let result = self.get(kind, &record_id)?;
        tx.commit()?;
        Ok(result)
    }
    fn exists(&self, sql: &str, id: &str) -> Result<bool> {
        Ok(self.conn.query_row(sql, [id], |r| r.get(0))?)
    }
    fn audit(&self, actor: &ActorId, action: &str, kind: CatalogKind, id: &str) -> Result<()> {
        SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new(action)
                .with_actor(*actor)
                .with_details(format!("{} {}", kind.key(), id)),
        )?;
        Ok(())
    }
    fn empty_storage(&self, id: &str, permanent: bool) -> Result<()> {
        ensure!(!self.exists("SELECT EXISTS(SELECT 1 FROM stock_balances WHERE location_id=?1 AND qty<>0) OR EXISTS(SELECT 1 FROM stock_movements WHERE location_id=?1 GROUP BY item_id HAVING sum(delta_qty)<>0)",id)?,"Move all stock out before deleting storage");
        let dependents = if permanent {
            "SELECT EXISTS(SELECT 1 FROM locations WHERE parent_id=?1) OR EXISTS(SELECT 1 FROM grid_bases WHERE location_id=?1)"
        } else {
            "SELECT EXISTS(SELECT 1 FROM locations WHERE parent_id=?1 AND archived_at IS NULL) OR EXISTS(SELECT 1 FROM grid_bases WHERE location_id=?1 AND archived_at IS NULL)"
        };
        ensure!(
            !self.exists(dependents, id)?,
            "Move or delete child storage and bases first (permanent deletion requires removing their database records)"
        );
        if permanent {
            ensure!(
                !self.exists(
                    "SELECT EXISTS(SELECT 1 FROM stock_movements WHERE location_id=?1)",
                    id
                )?,
                "This storage has stock history. Permanently delete the linked deleted items first"
            );
        }
        Ok(())
    }
    pub fn lifecycle(
        &self,
        actor: &ActorId,
        kind: CatalogKind,
        id: &str,
        action: &str,
        confirmation: &str,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.authorize(actor, Role::Admin)?;
        let old = self.get(kind, id)?;
        if kind == CatalogKind::Storage {
            ensure!(
                !self.exists(
                    "SELECT EXISTS(SELECT 1 FROM gridfinity_bins WHERE location_id=?1)",
                    id
                )?,
                "Manage this storage through Gridfinity bins"
            );
        }
        let purge = action == "purge";
        ensure!(
            matches!(action, "delete" | "restore" | "purge"),
            "Unknown action"
        );
        ensure!(
            confirmation.trim()
                == if purge {
                    "DELETE PERMANENTLY"
                } else if action == "restore" {
                    "RESTORE"
                } else {
                    "DELETE"
                },
            "Type {} to confirm",
            if purge {
                "DELETE PERMANENTLY"
            } else if action == "restore" {
                "RESTORE"
            } else {
                "DELETE"
            }
        );
        ensure!(
            old.deleted == (purge || action == "restore"),
            "{}",
            if old.deleted {
                "Record is already deleted"
            } else {
                "Open Deleted records first"
            }
        );
        let location = match kind {
            CatalogKind::Storage => Some(id.to_string()),
            CatalogKind::Bin => Some(
                SqliteGridRepo::new(self.conn)
                    .get_bin_by_id(&id.parse()?)?
                    .context("Bin not found")?
                    .location_id
                    .to_string(),
            ),
            _ => None,
        };
        if action != "restore" {
            if let Some(ref location) = location {
                self.empty_storage(location, purge)?;
                if kind == CatalogKind::Storage {
                    ensure!(
                        !self.exists(
                            "SELECT EXISTS(SELECT 1 FROM gridfinity_bins WHERE location_id=?1)",
                            location
                        )?,
                        "Manage this storage through Gridfinity bins"
                    );
                }
            }
            if kind == CatalogKind::Base {
                ensure!(!self.exists("SELECT EXISTS(SELECT 1 FROM bin_placements WHERE base_id=?1 AND removed_at IS NULL)",id)?,"Unplace bins before deleting the base");
            }
            if kind == CatalogKind::Bin {
                ensure!(!self.exists("SELECT EXISTS(SELECT 1 FROM bin_placements WHERE bin_id=?1 AND removed_at IS NULL)",id)?,"Unplace the bin before deleting it");
            }
        } else {
            if let Some(ref location) = location {
                let parent: Option<String> = self.conn.query_row(
                    "SELECT parent_id FROM locations WHERE id=?1",
                    [location],
                    |r| r.get(0),
                )?;
                if let Some(parent) = parent {
                    self.location(&parent)?;
                }
            }
            if kind == CatalogKind::Base {
                self.location(text(&old.data, "location"))?;
            }
            if kind == CatalogKind::Device && !text(&old.data, "location").is_empty() {
                self.location(text(&old.data, "location"))?;
            }
        }
        if purge {
            match kind {
                CatalogKind::Storage | CatalogKind::Bin => {
                    let location = location.as_ref().unwrap();
                    self.conn.execute(
                        "DELETE FROM stock_balances WHERE location_id=?1",
                        [location],
                    )?;
                    self.conn.execute(
                        "DELETE FROM stock_thresholds WHERE stock_location_id=?1",
                        [location],
                    )?;
                    self.conn
                        .execute("DELETE FROM identifiers WHERE location_id=?1", [location])?;
                    self.conn.execute(
                        "UPDATE devices SET location_id=NULL WHERE location_id=?1",
                        [location],
                    )?;
                    if kind == CatalogKind::Bin {
                        self.conn
                            .execute("DELETE FROM bin_placements WHERE bin_id=?1", [id])?;
                        self.conn
                            .execute("DELETE FROM gridfinity_bins WHERE id=?1", [id])?;
                    }
                    self.conn
                        .execute("DELETE FROM locations WHERE id=?1", [location])?;
                }
                CatalogKind::Base => {
                    self.conn
                        .execute("DELETE FROM bin_placements WHERE base_id=?1", [id])?;
                    self.conn
                        .execute("DELETE FROM grid_bases WHERE id=?1", [id])?;
                }
                CatalogKind::Rule => {
                    self.conn
                        .execute("DELETE FROM automation_runs WHERE rule_id=?1", [id])?;
                    self.conn
                        .execute("DELETE FROM automation_rules WHERE id=?1", [id])?;
                }
                CatalogKind::Device => {
                    self.conn.execute("DELETE FROM devices WHERE id=?1", [id])?;
                }
                CatalogKind::Barcode => {
                    self.conn
                        .execute("DELETE FROM identifiers WHERE id=?1", [id])?;
                }
            }
        } else {
            let archived = if action == "restore" {
                None
            } else {
                Some(super::actor::format_time(time::OffsetDateTime::now_utc()))
            };
            self.conn.execute(
                &format!("UPDATE {} SET archived_at=?1 WHERE id=?2", kind.table()),
                params![archived, id],
            )?;
            if kind == CatalogKind::Bin {
                self.conn.execute(
                    "UPDATE locations SET archived_at=?1 WHERE id=?2",
                    params![archived, location],
                )?;
            }
            if action == "delete" && kind == CatalogKind::Rule {
                self.conn
                    .execute("UPDATE automation_rules SET enabled=0 WHERE id=?1", [id])?;
            }
            if kind == CatalogKind::Device {
                self.conn.execute(
                    "UPDATE devices SET active=?1 WHERE id=?2",
                    params![action == "restore", id],
                )?;
            }
        }
        self.audit(
            actor,
            if purge {
                "catalog.permanently_deleted"
            } else if action == "restore" {
                "catalog.restored"
            } else {
                "catalog.deleted"
            },
            kind,
            id,
        )?;
        tx.commit()?;
        Ok(())
    }
}
pub fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or("").trim()
}
fn optional(data: &Value, key: &str) -> Option<String> {
    (!text(data, key).is_empty()).then(|| text(data, key).into())
}
fn number(data: &Value, key: &str) -> Result<u32> {
    data[key]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .or_else(|| text(data, key).parse().ok())
        .with_context(|| format!("{key} must be a positive whole number"))
}
fn dimension(data: &Value, key: &str) -> Result<Option<f64>> {
    if data[key].is_null() || data[key].as_str() == Some("") {
        return Ok(None);
    }
    let value = data[key]
        .as_f64()
        .or_else(|| text(data, key).parse().ok())
        .context("Dimensions must be positive millimeters")?;
    ensure!(
        value.is_finite() && value > 0.,
        "Dimensions must be positive millimeters"
    );
    Ok(Some(value))
}
