//! Shared record directory, forms, and confirmed lifecycle actions.
use crate::{
    App,
    app::{Modal, NotificationLevel, Tab},
};
use serde_json::{Value, json};
use slim_db::repo::catalog::{CatalogKind as Kind, CatalogRecord, KINDS, SqliteCatalogService};

#[derive(Debug, Clone)]
pub struct Directory {
    pub kind: Kind,
    pub deleted: bool,
    pub records: Vec<CatalogRecord>,
    pub selected: usize,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct RecordEditor {
    pub kind: Kind,
    pub id: Option<String>,
    pub fields: Vec<(&'static str, &'static str, String)>,
    pub focused: usize,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct RecordAction {
    pub kind: Kind,
    pub id: String,
    pub name: String,
    pub action: &'static str,
    pub confirmation: String,
    pub error: Option<String>,
}
impl App<'_> {
    pub fn context_record_action(&mut self, deleted: bool) {
        let mut kind = match self.current_tab {
            Tab::Gridfinity => Kind::Base,
            Tab::Devices => Kind::Device,
            Tab::Automation => Kind::Rule,
            _ => Kind::Storage,
        };
        let mut selected = match self.current_tab {
            Tab::Locations => self
                .locations_list_state
                .selected()
                .and_then(|i| self.locations.get(i))
                .map(|l| l.location.id.to_string()),
            Tab::Gridfinity => self
                .grid_base_list_state
                .selected()
                .and_then(|i| self.grid_bases.get(i))
                .map(|b| b.id.to_string()),
            _ => None,
        };
        if self.current_tab == Tab::Locations
            && let Some(id) = selected.as_ref()
            && let Ok(parsed) = id.parse()
        {
            use slim_core::service::GridRepository;
            if let Ok(Some(bin)) =
                slim_db::repo::grid::SqliteGridRepo::new(self.conn).get_bin_by_location_id(&parsed)
            {
                kind = Kind::Bin;
                selected = Some(bin.id.to_string());
            }
        }
        self.open_directory(kind, deleted);
        if !deleted
            && let Some(id) = selected
            && let Some(Modal::Directory(f)) = self.active_modal.as_mut()
            && let Some(at) = f.records.iter().position(|r| r.id == id)
        {
            f.selected = at;
            self.directory_action(false);
        }
    }
    pub fn open_management(&mut self) {
        let kind = match self.current_tab {
            Tab::Gridfinity => Kind::Base,
            Tab::Automation => Kind::Rule,
            Tab::Devices => Kind::Device,
            _ => Kind::Storage,
        };
        self.open_directory(kind, false);
    }
    pub fn open_directory(&mut self, kind: Kind, deleted: bool) {
        match SqliteCatalogService::new(self.conn).list(&self.actor.id, kind, deleted) {
            Ok(records) if self.authenticated => {
                self.active_modal = Some(Modal::Directory(Directory {
                    kind,
                    deleted,
                    records,
                    selected: 0,
                    error: None,
                }));
                self.edit_cursor = None;
            }
            Ok(_) => self.set_notification(NotificationLevel::Warning, "Sign in first"),
            Err(e) => self.set_notification(NotificationLevel::Error, e.to_string()),
        }
    }
    pub fn directory_tab(&mut self, next: bool) {
        if let Some(Modal::Directory(f)) = &self.active_modal {
            let at = KINDS.iter().position(|k| *k == f.kind).unwrap_or(0);
            self.open_directory(
                KINDS[(at + if next { 1 } else { KINDS.len() - 1 }) % KINDS.len()],
                false,
            );
        }
    }
    pub fn directory_edit(&mut self, new: bool) {
        if let Some(Modal::Directory(f)) = &self.active_modal {
            if f.deleted {
                self.set_notification(
                    NotificationLevel::Warning,
                    "Restore the record before editing",
                );
                return;
            }
            let kind = f.kind;
            let id = if new {
                None
            } else {
                f.records.get(f.selected).map(|r| r.id.clone())
            };
            if !new && id.is_none() {
                return;
            }
            self.open_record_editor(kind, id);
        }
    }
    pub fn open_record_editor(&mut self, kind: Kind, id: Option<String>) {
        if !self.authenticated
            || !self.actor.role.can_adjust()
            || (kind == Kind::Rule && !self.actor.role.can_admin())
        {
            self.set_notification(
                NotificationLevel::Warning,
                "Insufficient permission to edit this record",
            );
            return;
        }
        let data = match id
            .as_ref()
            .map(|id| SqliteCatalogService::new(self.conn).get(kind, id))
            .transpose()
        {
            Ok(record) => record.map(|r| r.data).unwrap_or(json!({})),
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e.to_string());
                return;
            }
        };
        let fields = kind
            .fields()
            .iter()
            .map(|(key, label, default)| {
                let value = &data[*key];
                (
                    *key,
                    *label,
                    if value.is_null() {
                        (*default).into()
                    } else if let Some(s) = value.as_str() {
                        s.into()
                    } else {
                        value.to_string()
                    },
                )
            })
            .collect();
        self.active_modal = Some(Modal::RecordEditor(RecordEditor {
            kind,
            id,
            fields,
            focused: 0,
            error: None,
        }));
        self.edit_cursor = None;
    }
    pub fn directory_action(&mut self, restore: bool) {
        if let Some(Modal::Directory(f)) = &self.active_modal
            && let Some(r) = f.records.get(f.selected)
        {
            self.active_modal = Some(Modal::RecordAction(RecordAction {
                kind: f.kind,
                id: r.id.clone(),
                name: r.name.clone(),
                action: if restore {
                    "restore"
                } else if f.deleted {
                    "purge"
                } else {
                    "delete"
                },
                confirmation: String::new(),
                error: None,
            }));
            self.edit_cursor = None;
        }
    }
    pub fn submit_record_editor(&mut self, mut form: RecordEditor) {
        if !self.authenticated {
            self.set_notification(NotificationLevel::Warning, "Sign in first");
            return;
        }
        let mut data = json!({});
        for (key, _, value) in &form.fields {
            data[*key] = Value::String(value.clone());
        }
        match SqliteCatalogService::new(self.conn).save(
            &self.actor.id,
            form.kind,
            form.id.as_deref(),
            &data,
        ) {
            Ok(_) => {
                let _ = self.refresh_data();
                self.open_directory(form.kind, false);
                self.set_notification(NotificationLevel::Success, "Record saved");
            }
            Err(e) => {
                form.error = Some(e.to_string());
                self.active_modal = Some(Modal::RecordEditor(form));
            }
        }
    }
    pub fn submit_record_action(&mut self, mut form: RecordAction) {
        if !self.authenticated {
            self.set_notification(NotificationLevel::Warning, "Sign in first");
            return;
        }
        match SqliteCatalogService::new(self.conn).lifecycle(
            &self.actor.id,
            form.kind,
            &form.id,
            form.action,
            &form.confirmation,
        ) {
            Ok(()) => {
                let _ = self.refresh_data();
                self.open_directory(form.kind, form.action != "restore");
                self.set_notification(
                    NotificationLevel::Success,
                    match form.action {
                        "purge" => "Record permanently deleted",
                        "restore" => "Record restored",
                        _ => "Record moved to Deleted",
                    },
                );
            }
            Err(e) => {
                form.error = Some(e.to_string());
                self.active_modal = Some(Modal::RecordAction(form));
            }
        }
    }
}
