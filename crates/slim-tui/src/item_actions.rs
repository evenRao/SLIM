//! Admin-only, recoverable removal with confirmation and audit history.
use crate::app::{App, Modal, NotificationLevel};
use slim_core::{
    model::{actor::Role, item::ItemId},
    service::ActorRepository,
};
use slim_db::repo::{actor::SqliteActorRepo, item::SqliteItemRepo};

#[derive(Debug, Clone)]
pub struct DeleteForm {
    pub id: ItemId,
    pub name: String,
    pub restore: bool,
    pub permanent: bool,
    pub confirmation: String,
    pub error: Option<String>,
}

impl App<'_> {
    fn current_is_admin(&self) -> bool {
        self.authenticated
            && SqliteActorRepo::new(self.conn)
                .get_by_id(&self.actor.id)
                .ok()
                .flatten()
                .is_some_and(|a| a.active && a.role == Role::Admin)
    }
    pub fn toggle_deleted(&mut self) {
        if !self.current_is_admin() {
            self.set_notification(
                NotificationLevel::Warning,
                "Only Admin can view deleted items",
            );
            return;
        }
        self.show_deleted = !self.show_deleted;
        self.items_table_state.select(None);
        if let Err(error) = self.refresh_data() {
            self.set_notification(NotificationLevel::Error, error.to_string());
        }
    }
    pub fn open_delete_modal(&mut self, restore: bool) {
        self.open_item_removal(restore, false);
    }
    pub fn open_permanent_delete_modal(&mut self) {
        self.open_item_removal(false, true);
    }
    fn open_item_removal(&mut self, restore: bool, permanent: bool) {
        if !self.current_is_admin() {
            self.set_notification(
                NotificationLevel::Warning,
                "Only Admin can delete or restore inventory items",
            );
            return;
        }
        let Some(item) = self
            .items_table_state
            .selected()
            .and_then(|i| self.items.get(i))
        else {
            self.set_notification(NotificationLevel::Info, "Select an inventory item first");
            return;
        };
        if permanent && !self.show_deleted {
            self.set_notification(
                NotificationLevel::Warning,
                "Press v to open Deleted items first",
            );
            return;
        }
        if item.item.archived_at.is_some() != (restore || permanent) {
            self.set_notification(
                NotificationLevel::Warning,
                if restore {
                    "Press v to view deleted items first"
                } else {
                    "This item is already deleted"
                },
            );
            return;
        }
        if !restore && item.total_on_hand > 0 {
            self.set_notification(
                NotificationLevel::Warning,
                "Issue remaining stock or adjust its count to zero before deleting",
            );
            return;
        }
        self.edit_cursor = None;
        self.active_modal = Some(Modal::Delete(DeleteForm {
            id: item.item.id,
            name: item.item.name.clone(),
            restore,
            permanent,
            confirmation: String::new(),
            error: None,
        }));
    }
    pub fn submit_delete(&mut self, mut form: DeleteForm) {
        let expected = if form.permanent {
            "DELETE PERMANENTLY"
        } else if form.restore {
            "RESTORE"
        } else {
            "DELETE"
        };
        if form.confirmation.trim() != expected {
            form.error = Some(format!("Type {expected} to confirm, or Esc to cancel"));
            self.active_modal = Some(Modal::Delete(form));
            return;
        }
        let repo = SqliteItemRepo::new(self.conn);
        let result = if form.permanent {
            repo.permanently_delete_as_admin(&form.id, &self.actor.id, &form.confirmation)
        } else if form.restore {
            repo.restore_as_admin(&form.id, &self.actor.id)
        } else {
            repo.delete_as_admin(&form.id, &self.actor.id)
        };
        match result {
            Ok(()) => {
                self.close_modal();
                let message = if form.permanent {
                    format!("Permanently deleted {} and its history.", form.name)
                } else if form.restore {
                    format!("Restored {}. Press v for active inventory.", form.name)
                } else {
                    format!(
                        "Deleted {}. Press v to view or restore deleted items.",
                        form.name
                    )
                };
                self.set_notification(NotificationLevel::Success, message);
                if let Err(error) = self.refresh_data() {
                    self.set_notification(
                        NotificationLevel::Error,
                        format!("Saved; refresh failed: {error}"),
                    );
                }
            }
            Err(error) => {
                form.error = Some(error.to_string());
                self.active_modal = Some(Modal::Delete(form));
            }
        }
    }
}
