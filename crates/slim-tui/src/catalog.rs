//! Catalog and first-run forms that keep ordinary inventory work inside Ratatui.
use crate::app::{App, Modal, NotificationLevel, Tab};
use slim_core::model::{
    actor::{ActorId, ActorUpdate, NewActor, Role},
    auth::NewAuditEvent,
    gridfinity::{NewGridBase, NewGridfinityBin, PlaceBinCmd},
    item::{ItemId, ItemUpdate, NewItem},
    location::{LocationId, LocationKind, NewLocation},
    stock::StockThreshold,
};
use slim_core::service::{
    ActorRepository, AuditRepository, AuthService, GridRepository, ItemRepository,
    LocationRepository, ThresholdRepository,
};
use slim_db::repo::{
    actor::SqliteActorRepo, audit::SqliteAuditRepo, auth::SqliteAuthService, grid::SqliteGridRepo,
    inventory::SqliteThresholdRepo, item::SqliteItemRepo, location::SqliteLocationRepo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogKind {
    Setup,
    Item(Option<ItemId>),
    Location(Option<LocationId>),
    Base,
    Bin,
    Placement,
    Person,
    EditPerson(ActorId),
    ResetPin(ActorId),
}

#[derive(Debug, Clone)]
pub struct CatalogField {
    pub label: &'static str,
    pub value: String,
    pub secret: bool,
}

#[derive(Debug, Clone)]
pub struct CatalogForm {
    pub kind: CatalogKind,
    pub title: &'static str,
    pub fields: Vec<CatalogField>,
    pub focused_field: usize,
    pub error: Option<String>,
    pub context: Option<String>,
}

impl CatalogForm {
    fn new(kind: CatalogKind, title: &'static str, labels: &[(&'static str, &str)]) -> Self {
        Self {
            kind,
            title,
            fields: labels
                .iter()
                .map(|(label, value)| CatalogField {
                    label,
                    value: (*value).into(),
                    secret: label.contains("PIN"),
                })
                .collect(),
            focused_field: 0,
            error: None,
            context: None,
        }
    }
    pub fn move_field(&mut self, next: bool) {
        self.focused_field =
            (self.focused_field + if next { 1 } else { self.fields.len() - 1 }) % self.fields.len();
    }
    fn text(&self, index: usize) -> &str {
        self.fields[index].value.trim()
    }
    fn number(&self, index: usize) -> anyhow::Result<u32> {
        self.text(index).parse().map_err(|_| {
            anyhow::anyhow!(
                "{} must be a whole number from 0 to 4,294,967,295",
                self.fields[index].label
            )
        })
    }
    fn optional(&self, index: usize) -> Option<String> {
        (!self.text(index).is_empty()).then(|| self.text(index).into())
    }
}

impl App<'_> {
    pub fn open_new_catalog(&mut self) {
        let managed = match self.current_tab {
            Tab::Locations => Some(slim_db::repo::catalog::CatalogKind::Storage),
            Tab::Automation => Some(slim_db::repo::catalog::CatalogKind::Rule),
            Tab::Devices => Some(slim_db::repo::catalog::CatalogKind::Device),
            _ => None,
        };
        if let Some(kind) = managed {
            self.open_record_editor(kind, None);
            return;
        }
        let kind = match self.current_tab {
            Tab::Locations => CatalogKind::Location(None),
            Tab::Gridfinity => CatalogKind::Base,
            _ => CatalogKind::Item(None),
        };
        self.open_catalog(kind);
    }

    pub fn open_edit_catalog(&mut self) {
        use slim_db::repo::catalog::CatalogKind as Managed;
        if self.current_tab == Tab::Locations {
            if let Some(location) = self
                .locations_list_state
                .selected()
                .and_then(|i| self.locations.get(i))
            {
                let id = location.location.id;
                if let Ok(Some(bin)) = SqliteGridRepo::new(self.conn).get_bin_by_location_id(&id) {
                    self.open_record_editor(Managed::Bin, Some(bin.id.to_string()));
                } else {
                    self.open_record_editor(Managed::Storage, Some(id.to_string()));
                }
            }
            return;
        }
        if self.current_tab == Tab::Gridfinity {
            if let Some(base) = self
                .grid_base_list_state
                .selected()
                .and_then(|i| self.grid_bases.get(i))
            {
                self.open_record_editor(Managed::Base, Some(base.id.to_string()));
            }
            return;
        }
        if matches!(self.current_tab, Tab::Devices | Tab::Automation) {
            self.open_management();
            return;
        }
        if self.current_tab == Tab::Items && self.show_deleted {
            self.set_notification(
                NotificationLevel::Warning,
                "Restore the item before editing it",
            );
            return;
        }
        let kind = match self.current_tab {
            Tab::Items => self
                .items_table_state
                .selected()
                .and_then(|i| self.items.get(i))
                .map(|p| CatalogKind::Item(Some(p.item.id))),
            Tab::Locations => self
                .locations_list_state
                .selected()
                .and_then(|i| self.locations.get(i))
                .map(|p| CatalogKind::Location(Some(p.location.id))),
            _ => None,
        };
        if let Some(kind) = kind {
            self.open_catalog(kind);
        }
    }

    pub fn open_catalog(&mut self, kind: CatalogKind) {
        if kind != CatalogKind::Setup && (!self.authenticated || !self.actor.role.can_adjust()) {
            self.set_notification(NotificationLevel::Warning, "Manager or Admin role required");
            return;
        }
        if matches!(
            kind,
            CatalogKind::Person | CatalogKind::EditPerson(_) | CatalogKind::ResetPin(_)
        ) && !self.actor.role.can_admin()
        {
            self.set_notification(
                NotificationLevel::Warning,
                "Admin role required to add people",
            );
            return;
        }
        let mut form = match kind {
            CatalogKind::EditPerson(id) | CatalogKind::ResetPin(id) => {
                let user = match SqliteActorRepo::new(self.conn).get_by_id(&id) {
                    Ok(Some(user)) => user,
                    _ => {
                        self.set_notification(NotificationLevel::Error, "User is unavailable");
                        return;
                    }
                };
                let mut f = if matches!(kind, CatalogKind::ResetPin(_)) {
                    CatalogForm::new(
                        kind,
                        "Reset user PIN · Admin",
                        &[("New PIN (at least 4 characters)", ""), ("Confirm PIN", "")],
                    )
                } else {
                    CatalogForm::new(
                        kind,
                        "Edit user · Admin",
                        &[
                            ("Name", &user.display_name),
                            (
                                "Role: viewer/operator/manager/admin",
                                &user.role.to_string(),
                            ),
                        ],
                    )
                };
                f.context = Some(format!("Account: {}", user.display_name));
                f
            }
            CatalogKind::Setup => CatalogForm::new(
                kind,
                "Welcome to SLIM - create your owner account",
                &[
                    ("Your name", ""),
                    ("PIN (at least 4 characters)", ""),
                    ("Confirm PIN", ""),
                ],
            ),
            CatalogKind::Item(id) => {
                let mut f = CatalogForm::new(
                    kind,
                    if id.is_some() {
                        "Edit item"
                    } else {
                        "Add item"
                    },
                    &[
                        ("Name", ""),
                        ("SKU (optional)", ""),
                        ("Unit", "each"),
                        ("Description (optional)", ""),
                        ("Default minimum quantity", "0"),
                    ],
                );
                if let Some(item) = id.and_then(|id| self.items.iter().find(|p| p.item.id == id)) {
                    f.fields[0].value.clone_from(&item.item.name);
                    f.fields[1].value = item.item.sku.clone().unwrap_or_default();
                    f.fields[2].value.clone_from(&item.item.unit);
                    f.fields[3].value = item.item.description.clone().unwrap_or_default();
                    match SqliteThresholdRepo::new(self.conn).get_for_item(&item.item.id) {
                        Ok(thresholds) => {
                            f.fields[4].value = thresholds
                                .iter()
                                .find(|t| t.stock_location_id.is_none())
                                .map(|t| t.min_qty)
                                .unwrap_or(0)
                                .to_string()
                        }
                        Err(error) => {
                            self.set_notification(NotificationLevel::Error, error.to_string());
                            return;
                        }
                    }
                }
                f
            }
            CatalogKind::Location(Some(id)) => {
                let Some(location) = self.locations.iter().find(|p| p.location.id == id) else {
                    return;
                };
                CatalogForm::new(
                    kind,
                    "Rename location",
                    &[("Name", &location.location.name)],
                )
            }
            CatalogKind::Location(None) => CatalogForm::new(
                kind,
                "Add storage location",
                &[
                    ("Name", ""),
                    (
                        "Kind: room/cabinet/drawer/shelf/storage/compartment",
                        "storage",
                    ),
                    ("Parent name, full path, or UUID (optional)", ""),
                ],
            ),
            CatalogKind::Base => CatalogForm::new(
                kind,
                "Add Gridfinity base",
                &[
                    ("Base code", ""),
                    ("Location name, full path, or UUID", ""),
                    ("Columns (1-100)", "6"),
                    ("Rows (1-100)", "4"),
                ],
            ),
            CatalogKind::Bin => CatalogForm::new(
                kind,
                "Add Gridfinity bin",
                &[
                    ("Bin code", ""),
                    ("Parent location (optional)", ""),
                    ("Width units", "1"),
                    ("Depth units", "1"),
                    ("Height units", "3"),
                    ("Compartments", "1"),
                ],
            ),
            CatalogKind::Placement => CatalogForm::new(
                kind,
                "Place or move bin",
                &[
                    ("Bin code", ""),
                    ("Base code", ""),
                    ("Column X (starts at 0)", "0"),
                    ("Row Y (starts at 0)", "0"),
                    ("Rotation: 0 or 90", "0"),
                ],
            ),
            CatalogKind::Person => CatalogForm::new(
                kind,
                "Add person",
                &[
                    ("Name", ""),
                    ("Role: viewer/operator/manager/admin", "operator"),
                    ("PIN (at least 4 characters)", ""),
                    ("Confirm PIN", ""),
                ],
            ),
        };
        if kind == CatalogKind::Placement
            && let Some(base) = self
                .grid_base_list_state
                .selected()
                .and_then(|i| self.grid_bases.get(i))
        {
            form.fields[1].value.clone_from(&base.code);
        }
        self.edit_cursor = None;
        self.active_modal = Some(Modal::Catalog(form));
    }

    pub fn submit_catalog(&mut self, mut form: CatalogForm) {
        match self.save_catalog(&form) {
            Ok(()) => {
                self.close_modal();
                if let Err(error) = self.refresh_data() {
                    self.set_notification(
                        NotificationLevel::Error,
                        format!("Saved, but refresh failed: {error}"),
                    );
                } else {
                    self.set_notification(
                        NotificationLevel::Success,
                        format!("{} saved", form.title),
                    );
                }
                if matches!(
                    form.kind,
                    CatalogKind::Person | CatalogKind::EditPerson(_) | CatalogKind::ResetPin(_)
                ) {
                    let _ = self.revalidate_actor();
                    if self.authenticated {
                        self.open_people_modal();
                    }
                }
            }
            Err(error) => {
                form.error = Some(error.to_string());
                self.active_modal = Some(Modal::Catalog(form));
            }
        }
    }

    fn save_catalog(&mut self, form: &CatalogForm) -> anyhow::Result<()> {
        let actors = SqliteActorRepo::new(self.conn);
        if form.kind != CatalogKind::Setup {
            let current = actors
                .get_by_id(&self.actor.id)?
                .filter(|a| a.active)
                .ok_or_else(|| anyhow::anyhow!("Sign in again"))?;
            anyhow::ensure!(
                self.authenticated && current.role.can_adjust(),
                "Manager or Admin role required"
            );
            anyhow::ensure!(
                !matches!(
                    form.kind,
                    CatalogKind::Person | CatalogKind::EditPerson(_) | CatalogKind::ResetPin(_)
                ) || current.role.can_admin(),
                "Admin role required"
            );
        }
        // Account services own their authorization/audit transactions.
        match form.kind {
            CatalogKind::Person => {
                anyhow::ensure!(form.text(2) == form.text(3), "PINs do not match");
                actors.create_as_admin(
                    &self.actor.id,
                    &NewActor::new(form.text(0)).with_role(form.text(1).parse()?),
                    Some(form.text(2)),
                )?;
                return Ok(());
            }
            CatalogKind::EditPerson(id) => {
                actors.update_as_admin(
                    &self.actor.id,
                    &id,
                    &ActorUpdate {
                        display_name: Some(form.text(0).into()),
                        role: Some(form.text(1).parse()?),
                        ..Default::default()
                    },
                )?;
                return Ok(());
            }
            CatalogKind::ResetPin(id) => {
                anyhow::ensure!(form.text(0) == form.text(1), "PINs do not match");
                actors.update_as_admin(
                    &self.actor.id,
                    &id,
                    &ActorUpdate {
                        pin: Some(form.text(0).into()),
                        ..Default::default()
                    },
                )?;
                return Ok(());
            }
            _ => {}
        }
        // Placement validates and opens its own immediate transaction.
        if form.kind == CatalogKind::Placement {
            let grid = SqliteGridRepo::new(self.conn);
            let bin = grid
                .get_bin_by_code(form.text(0))?
                .ok_or_else(|| anyhow::anyhow!("Bin code not found"))?;
            let base = grid
                .get_base_by_code(form.text(1))?
                .ok_or_else(|| anyhow::anyhow!("Base code not found"))?;
            grid.place_bin(&PlaceBinCmd {
                bin_id: bin.id,
                base_id: base.id,
                x: form.number(2)?,
                y: form.number(3)?,
                rotation: form.text(4).parse()?,
            })?;
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut owner = None;
        match form.kind {
            CatalogKind::Setup => {
                anyhow::ensure!(
                    actors.list()?.is_empty(),
                    "Workspace already has an owner; sign in instead"
                );
                anyhow::ensure!(form.text(1) == form.text(2), "PINs do not match");
                let actor = actors.create(&NewActor::new(form.text(0)).with_role(Role::Admin))?;
                SqliteAuthService::new(self.conn).set_pin(&actor.id, form.text(1))?;
                if SqliteLocationRepo::new(self.conn)
                    .list_children(None)?
                    .is_empty()
                {
                    SqliteLocationRepo::new(self.conn).create(&NewLocation {
                        parent_id: None,
                        name: "Main storage".into(),
                        kind: LocationKind::Storage,
                    })?;
                }
                owner = Some(actor);
            }
            CatalogKind::Item(id) => {
                let minimum = form.number(4)?;
                let repo = SqliteItemRepo::new(self.conn);
                let item = match id {
                    Some(id) => repo.update(
                        &id,
                        &ItemUpdate {
                            name: Some(form.text(0).into()),
                            sku: Some(form.optional(1)),
                            unit: Some(form.text(2).into()),
                            description: Some(form.optional(3)),
                            ..Default::default()
                        },
                    )?,
                    None => repo.create(&NewItem {
                        name: form.text(0).into(),
                        sku: form.optional(1),
                        unit: form.text(2).into(),
                        description: form.optional(3),
                        metadata: None,
                    })?,
                };
                SqliteThresholdRepo::new(self.conn).set(&StockThreshold {
                    item_id: item.id,
                    stock_location_id: None,
                    min_qty: minimum,
                })?;
            }
            CatalogKind::Location(Some(id)) => {
                anyhow::ensure!(!form.text(0).is_empty(), "Location name is required");
                anyhow::ensure!(
                    SqliteGridRepo::new(self.conn)
                        .get_bin_by_location_id(&id)?
                        .is_none(),
                    "Bin locations keep their bin code as their name"
                );
                self.conn.execute(
                    "UPDATE locations SET name=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![
                        form.text(0),
                        time::OffsetDateTime::now_utc()
                            .format(&time::format_description::well_known::Rfc3339)?,
                        id.to_string()
                    ],
                )?;
            }
            CatalogKind::Location(None) => {
                let kind: LocationKind = form.text(1).parse()?;
                anyhow::ensure!(
                    kind != LocationKind::Bin,
                    "Use Add bin on the Gridfinity tab for bin locations"
                );
                SqliteLocationRepo::new(self.conn).create(&NewLocation {
                    name: form.text(0).into(),
                    kind,
                    parent_id: self.catalog_parent(form.text(2))?,
                })?;
            }
            CatalogKind::Base => {
                SqliteGridRepo::new(self.conn).create_base(&NewGridBase {
                    code: form.text(0).into(),
                    location_id: self
                        .resolve_location(form.text(1))
                        .map_err(anyhow::Error::msg)?
                        .0,
                    cols: form.number(2)?,
                    rows: form.number(3)?,
                    pitch_mm: None,
                })?;
            }
            CatalogKind::Bin => {
                let parent = self.catalog_parent(form.text(1))?;
                let bin = NewGridfinityBin {
                    bin_code: form.text(0).into(),
                    parent_location_id: parent,
                    width_units: form.number(2)?,
                    depth_units: form.number(3)?,
                    height_units: form.number(4)?,
                    compartment_count: form.number(5)?,
                };
                bin.validate()?;
                let location = SqliteLocationRepo::new(self.conn).create(&NewLocation {
                    parent_id: parent,
                    name: bin.bin_code.clone(),
                    kind: LocationKind::Bin,
                })?;
                SqliteGridRepo::new(self.conn).create_bin(&bin, &location.id)?;
            }
            CatalogKind::Person | CatalogKind::EditPerson(_) | CatalogKind::ResetPin(_) => {
                unreachable!("Account service returned above")
            }
            CatalogKind::Placement => unreachable!(),
        }
        let audit_actor = owner.as_ref().unwrap_or(&self.actor);
        SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new("tui.catalog.save")
                .with_actor(audit_actor.id)
                .with_details(form.title),
        )?;
        tx.commit()?;
        if let Some(actor) = owner {
            self.actor = actor;
            self.authenticated = true;
            self.capture_authenticated_pin()?;
        }
        Ok(())
    }

    fn catalog_parent(&self, text: &str) -> anyhow::Result<Option<LocationId>> {
        if text.is_empty() {
            Ok(None)
        } else {
            self.resolve_location(text)
                .map(|(id, _)| Some(id))
                .map_err(anyhow::Error::msg)
        }
    }

    pub fn create_backup(&mut self) {
        if !self.authenticated {
            return;
        }
        let result = (|| -> anyhow::Result<std::path::PathBuf> {
            let path: String = self.conn.query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                !path.is_empty(),
                "An in-memory database has no backup folder"
            );
            let directory = std::path::Path::new(&path)
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Database folder is unavailable"))?
                .join("backups");
            std::fs::create_dir_all(&directory)?;
            let destination = directory.join(format!(
                "slim-{}.sqlite3",
                time::OffsetDateTime::now_utc().unix_timestamp_nanos()
            ));
            self.conn
                .backup(rusqlite::DatabaseName::Main, &destination, None)?;
            Ok(destination)
        })();
        match result {
            Ok(path) => self.set_notification(
                NotificationLevel::Success,
                format!("Backup: {}", path.display()),
            ),
            Err(error) => self.set_notification(NotificationLevel::Error, error.to_string()),
        }
    }
}
