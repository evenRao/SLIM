//! Admin-only account management and live account validation for open terminals.
use crate::{
    app::{App, Modal, NotificationLevel},
    catalog::CatalogKind,
};
use slim_core::{
    error::CoreResult,
    model::actor::{Actor, ActorId, ActorUpdate, Role},
    service::ActorRepository,
};
use slim_db::repo::actor::SqliteActorRepo;

#[derive(Debug, Clone)]
pub struct PeopleForm {
    pub users: Vec<Actor>,
    pub selected: usize,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct RemoveUserForm {
    pub id: ActorId,
    pub name: String,
    pub permanent: bool,
    pub confirmation: String,
    pub error: Option<String>,
}

impl App<'_> {
    pub fn revalidate_actor(&mut self) -> CoreResult<()> {
        if !self.authenticated {
            return Ok(());
        }
        let repo = SqliteActorRepo::new(self.conn);
        let account = repo.get_by_id(&self.actor.id)?;
        let current_pin = repo.get_pin_hash(&self.actor.id)?;
        if account.as_ref().is_none_or(|a| !a.active) || current_pin != self.authenticated_pin_hash
        {
            self.authenticated = false;
            self.show_deleted = false;
            self.edit_cursor = None;
            self.open_user_modal();
            if let Some(Modal::User(form)) = self.active_modal.as_mut() {
                form.error = Some("Your account changed. Sign in again.".into());
            }
        } else if let Some(actor) = account {
            self.actor = actor;
        }
        Ok(())
    }
    pub fn capture_authenticated_pin(&mut self) -> CoreResult<()> {
        self.authenticated_pin_hash =
            SqliteActorRepo::new(self.conn).get_pin_hash(&self.actor.id)?;
        Ok(())
    }
    fn people_admin(&self) -> bool {
        self.authenticated
            && SqliteActorRepo::new(self.conn)
                .get_by_id(&self.actor.id)
                .ok()
                .flatten()
                .is_some_and(|a| a.active && a.role == Role::Admin)
    }
    pub fn open_people_modal(&mut self) {
        if !self.people_admin() {
            self.set_notification(
                NotificationLevel::Warning,
                "Only Admin can manage users or change PINs",
            );
            return;
        }
        match SqliteActorRepo::new(self.conn).list() {
            Ok(users) => {
                self.active_modal = Some(Modal::People(PeopleForm {
                    users,
                    selected: 0,
                    error: None,
                }))
            }
            Err(error) => self.set_notification(NotificationLevel::Error, error.to_string()),
        }
    }
    pub fn open_selected_person(&mut self, pin: bool) {
        if let Some(Modal::People(form)) = self.active_modal.as_ref()
            && let Some(user) = form.users.get(form.selected)
        {
            self.open_catalog(if pin {
                CatalogKind::ResetPin(user.id)
            } else {
                CatalogKind::EditPerson(user.id)
            });
        }
    }
    pub fn open_remove_user(&mut self) {
        if !self.people_admin() {
            return;
        }
        if let Some(Modal::People(form)) = self.active_modal.as_ref()
            && let Some(user) = form.users.get(form.selected)
        {
            self.active_modal = Some(Modal::RemoveUser(RemoveUserForm {
                id: user.id,
                name: user.display_name.clone(),
                permanent: !user.active,
                confirmation: String::new(),
                error: None,
            }));
            self.edit_cursor = None;
        }
    }
    pub fn submit_remove_user(&mut self, mut form: RemoveUserForm) {
        let expected = if form.permanent {
            "DELETE PERMANENTLY"
        } else {
            "REMOVE"
        };
        if form.confirmation.trim() != expected {
            form.error = Some(format!("Type {expected} to confirm, or Esc to cancel"));
            self.active_modal = Some(Modal::RemoveUser(form));
            return;
        }
        let repo = SqliteActorRepo::new(self.conn);
        let result = if form.permanent {
            repo.permanently_delete_user(&self.actor.id, &form.id, &form.confirmation)
        } else {
            repo.update_as_admin(
                &self.actor.id,
                &form.id,
                &ActorUpdate {
                    active: Some(false),
                    ..Default::default()
                },
            )
            .map(|_| ())
        };
        match result {
            Ok(_) => {
                self.close_modal();
                let _ = self.revalidate_actor();
                if self.authenticated {
                    self.open_people_modal();
                }
                self.set_notification(
                    NotificationLevel::Success,
                    if form.permanent {
                        format!(
                            "Permanently deleted {}. Past activity anonymized.",
                            form.name
                        )
                    } else {
                        format!("Removed {}. History is preserved.", form.name)
                    },
                );
            }
            Err(error) => {
                form.error = Some(error.to_string());
                self.active_modal = Some(Modal::RemoveUser(form));
            }
        }
    }
    pub fn restore_selected_user(&mut self) {
        let target = if let Some(Modal::People(form)) = self.active_modal.as_ref() {
            form.users
                .get(form.selected)
                .filter(|a| !a.active)
                .map(|a| a.id)
        } else {
            None
        };
        if let Some(id) = target {
            let result = SqliteActorRepo::new(self.conn).update_as_admin(
                &self.actor.id,
                &id,
                &ActorUpdate {
                    active: Some(true),
                    ..Default::default()
                },
            );
            match result {
                Ok(_) => self.open_people_modal(),
                Err(error) => {
                    if let Some(Modal::People(form)) = self.active_modal.as_mut() {
                        form.error = Some(error.to_string());
                    }
                }
            }
        }
    }
}
