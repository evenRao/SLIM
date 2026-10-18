//! Account mutations share authorization, lockout protection, and audit transactions.
use super::{actor::SqliteActorRepo, audit::SqliteAuditRepo};
use slim_core::{
    error::{CoreError, CoreResult},
    model::{
        actor::*,
        auth::{NewAuditEvent, hash_pin},
    },
    service::{ActorRepository, AuditRepository},
};

impl SqliteActorRepo<'_> {
    pub fn permanently_delete_user(
        &self,
        admin: &ActorId,
        target: &ActorId,
        confirmation: &str,
    ) -> CoreResult<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(storage)?;
        self.admin(admin)?;
        if confirmation.trim() != "DELETE PERMANENTLY" {
            return Err(CoreError::InvalidInput(
                "Type DELETE PERMANENTLY to erase this account".into(),
            ));
        }
        let account = self.get_by_id(target)?.ok_or_else(|| CoreError::NotFound {
            entity: "actor",
            id: target.to_string(),
        })?;
        if account.active || target == admin {
            return Err(CoreError::InvalidInput(
                "Remove the account before permanently deleting it".into(),
            ));
        }
        let system: bool = self
            .conn
            .query_row(
                "SELECT system_actor FROM actors WHERE id=?1",
                [target.to_string()],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if system {
            return Err(CoreError::InvalidInput(
                "System attribution cannot be deleted".into(),
            ));
        }
        let tombstone = "00000000-0000-0000-0000-000000000001";
        self.conn.execute("INSERT OR IGNORE INTO actors(id,display_name,active,role,created_at,system_actor) VALUES(?1,'Deleted user [system]',0,'viewer',?2,1)",rusqlite::params![tombstone,super::actor::format_time(time::OffsetDateTime::now_utc())]).map_err(storage)?;
        let valid: bool = self
            .conn
            .query_row(
                "SELECT system_actor=1 AND active=0 FROM actors WHERE id=?1",
                [tombstone],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if !valid {
            return Err(CoreError::InvalidInput(
                "System attribution ID is already in use".into(),
            ));
        }
        self.conn
            .execute(
                "UPDATE operations SET actor_id=?1 WHERE actor_id=?2",
                rusqlite::params![tombstone, target.to_string()],
            )
            .map_err(storage)?;
        self.conn
            .execute(
                "UPDATE observations SET confirmed_by=NULL WHERE confirmed_by=?1",
                [target.to_string()],
            )
            .map_err(storage)?;
        self.conn
            .execute(
                "UPDATE audit_events SET details=NULL WHERE actor_id=?1",
                [target.to_string()],
            )
            .map_err(storage)?;
        self.conn.execute("DELETE FROM audit_events WHERE action IN ('actor.created','actor.updated') AND instr(COALESCE(details,''),?1)>0",[target.to_string()]).map_err(storage)?;
        self.conn.execute("UPDATE event_outbox SET aggregate_id=?1,payload=json_set(payload,'$.actor_id',?1) WHERE json_valid(payload) AND json_extract(payload,'$.actor_id')=?2",rusqlite::params![tombstone,target.to_string()]).map_err(storage)?;
        self.conn
            .execute("DELETE FROM actors WHERE id=?1", [target.to_string()])
            .map_err(storage)?;
        SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new("actor.permanently_deleted")
                .with_actor(*admin)
                .with_details(target.to_string()),
        )?;
        tx.commit().map_err(storage)?;
        Ok(())
    }
    fn admin(&self, id: &ActorId) -> CoreResult<()> {
        if !self
            .get_by_id(id)?
            .is_some_and(|a| a.active && a.role == Role::Admin)
        {
            return Err(CoreError::Unauthorized(
                "Only an active Admin can manage users or change PINs".into(),
            ));
        }
        Ok(())
    }
    pub fn create_as_admin(
        &self,
        admin: &ActorId,
        account: &NewActor,
        pin: Option<&str>,
    ) -> CoreResult<Actor> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(storage)?;
        self.admin(admin)?;
        if let Some(pin) = pin {
            validate_pin(pin)?;
        }
        let actor = self.create(account)?;
        if let Some(pin) = pin {
            self.set_pin_hash(&actor.id, &hash_pin(pin.trim()))?;
        }
        SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new("actor.created")
                .with_actor(*admin)
                .with_details(format!(
                    "{} ({}) [{}]",
                    actor.display_name, actor.id, actor.role
                )),
        )?;
        tx.commit().map_err(storage)?;
        Ok(actor)
    }
    pub fn update_as_admin(
        &self,
        admin: &ActorId,
        target: &ActorId,
        changes: &ActorUpdate,
    ) -> CoreResult<Actor> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(storage)?;
        self.admin(admin)?;
        let old = self.get_by_id(target)?.ok_or_else(|| CoreError::NotFound {
            entity: "actor",
            id: target.to_string(),
        })?;
        let name = changes
            .display_name
            .as_deref()
            .unwrap_or(&old.display_name)
            .trim();
        if name.is_empty() {
            return Err(CoreError::InvalidInput("User name cannot be blank".into()));
        }
        let role = changes.role.unwrap_or(old.role);
        let active = changes.active.unwrap_or(old.active);
        if old.active && old.role == Role::Admin && (!active || role != Role::Admin) {
            let count: i64 = self
                .conn
                .query_row(
                    "SELECT count(*) FROM actors WHERE active=1 AND role='admin'",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if count <= 1 {
                return Err(CoreError::InvalidInput(
                    "Cannot remove or demote the last active administrator".into(),
                ));
            }
        }
        if let Some(pin) = changes.pin.as_deref() {
            validate_pin(pin)?;
        }
        self.conn
            .execute(
                "UPDATE actors SET display_name=?1,role=?2,active=?3 WHERE id=?4",
                rusqlite::params![name, role.to_string(), active, target.to_string()],
            )
            .map_err(storage)?;
        if let Some(pin) = changes.pin.as_deref() {
            self.set_pin_hash(target, &hash_pin(pin.trim()))?;
        }
        if changes.pin.is_some() || role != old.role || !active {
            self.conn
                .execute(
                    "DELETE FROM sessions WHERE actor_id=?1",
                    [target.to_string()],
                )
                .map_err(storage)?;
        }
        // Record who changed the account without recording credentials.
        SqliteAuditRepo::new(self.conn).record(
            &NewAuditEvent::new("actor.updated")
                .with_actor(*admin)
                .with_details(format!(
                    "target={} name={} role={} active={} pin_changed={}",
                    target,
                    name,
                    role,
                    active,
                    changes.pin.is_some()
                )),
        )?;
        let actor = self.get_by_id(target)?.ok_or_else(|| CoreError::NotFound {
            entity: "actor",
            id: target.to_string(),
        })?;
        tx.commit().map_err(storage)?;
        Ok(actor)
    }
}
fn validate_pin(pin: &str) -> CoreResult<()> {
    if pin.trim().len() < 4 {
        Err(CoreError::InvalidInput(
            "PIN must be at least 4 characters".into(),
        ))
    } else {
        Ok(())
    }
}
fn storage(error: rusqlite::Error) -> CoreError {
    CoreError::Storage(error.to_string())
}
