//! SQLite implementation of DeviceRepository and ObservationRepository.
//!
//! Manages hardware devices (IR breakbeams, presence detectors, scanners, RFID/NFC)
//! and records raw sensor observations for confirmation workflows.

use rusqlite::{Connection, params};
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::ActorId;
use slim_core::model::device::*;
use slim_core::model::location::LocationId;
use slim_core::model::operation::OperationId;
use slim_core::service::{DeviceRepository, ObservationRepository};
use time::OffsetDateTime;
use uuid::Uuid;

use super::actor::{OptionalExt, format_time, parse_time};

/// SQLite-backed device and observation repository.
pub struct SqliteDeviceRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteDeviceRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

fn row_to_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<RegisteredDevice> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let port_or_address: Option<String> = row.get(3)?;
    let location_str: Option<String> = row.get(4)?;
    let settings_json: Option<String> = row.get(5)?;
    let active_int: i64 = row.get(6)?;
    let created_str: String = row.get(7)?;
    let updated_str: String = row.get(8)?;

    let kind: DeviceKind = kind_str.parse().unwrap_or(DeviceKind::MockSensor);
    let location_id = location_str
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(LocationId);

    let created_at = parse_time(&created_str);
    let updated_at = parse_time(&updated_str);

    Ok(RegisteredDevice {
        id,
        name,
        kind,
        port_or_address,
        location_id,
        settings_json,
        active: active_int != 0,
        created_at,
        updated_at,
    })
}

fn row_to_observation(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredObservation> {
    let id_str: String = row.get(0)?;
    let device_id: String = row.get(1)?;
    let observation_type: String = row.get(2)?;
    let payload: String = row.get(3)?;
    let status_str: String = row.get(4)?;
    let op_str: Option<String> = row.get(5)?;
    let created_str: String = row.get(6)?;
    let confirmed_str: Option<String> = row.get(7)?;
    let actor_str: Option<String> = row.get(8)?;

    let id = ObservationId(Uuid::parse_str(&id_str).unwrap_or_default());
    let status: ObservationStatus = status_str
        .parse()
        .unwrap_or(ObservationStatus::PendingReview);
    let processed_operation_id = op_str
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(OperationId);

    let created_at = parse_time(&created_str);
    let confirmed_at = confirmed_str.as_deref().map(parse_time);
    let confirmed_by = actor_str
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(ActorId);

    Ok(StoredObservation {
        id,
        device_id,
        observation_type,
        payload,
        status,
        processed_operation_id,
        created_at,
        confirmed_at,
        confirmed_by,
    })
}

impl DeviceRepository for SqliteDeviceRepo<'_> {
    fn register_device(&self, device: &NewDevice) -> CoreResult<RegisteredDevice> {
        device.validate()?;

        // Verify location exists if specified
        if let Some(ref loc_id) = device.location_id {
            let loc_exists: bool = self
                .conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM locations WHERE id = ?1 AND archived_at IS NULL",
                    params![loc_id.0.to_string()],
                    |row| row.get(0),
                )
                .map_err(|e| CoreError::Storage(e.to_string()))?;

            if !loc_exists {
                return Err(CoreError::NotFound {
                    entity: "location",
                    id: loc_id.0.to_string(),
                });
            }
        }

        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);

        self.conn
            .execute(
                "INSERT INTO devices (
                    id, name, kind, port_or_address, location_id,
                    settings_json, active, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
                params![
                    device.id,
                    device.name.trim(),
                    device.kind.to_string(),
                    device.port_or_address.as_deref(),
                    device.location_id.map(|id| id.0.to_string()),
                    device.settings_json.as_deref(),
                    now_str,
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(_, Some(ref msg))
                    if msg.contains("UNIQUE constraint failed") =>
                {
                    CoreError::InvalidInput(format!(
                        "device with ID '{}' is already registered",
                        device.id
                    ))
                }
                other => CoreError::Storage(other.to_string()),
            })?;

        Ok(RegisteredDevice {
            id: device.id.clone(),
            name: device.name.trim().to_string(),
            kind: device.kind,
            port_or_address: device.port_or_address.clone(),
            location_id: device.location_id,
            settings_json: device.settings_json.clone(),
            active: true,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_device(&self, id: &str) -> CoreResult<Option<RegisteredDevice>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, kind, port_or_address, location_id,
                        settings_json, active, created_at, updated_at
                 FROM devices WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![id], row_to_device)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn list_devices(&self) -> CoreResult<Vec<RegisteredDevice>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, kind, port_or_address, location_id,
                        settings_json, active, created_at, updated_at
                 FROM devices WHERE archived_at IS NULL
                 ORDER BY created_at ASC",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_device)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut devices = Vec::new();
        for row in rows {
            devices.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(devices)
    }

    fn delete_device(&self, id: &str) -> CoreResult<()> {
        let _ = self
            .conn
            .execute("DELETE FROM observations WHERE device_id = ?1", params![id]);

        let rows = self
            .conn
            .execute("DELETE FROM devices WHERE id = ?1", params![id])
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "device",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    fn update_device_status(&self, id: &str, active: bool) -> CoreResult<()> {
        let now = format_time(OffsetDateTime::now_utc());
        let rows = self
            .conn
            .execute(
                "UPDATE devices SET active = ?1, updated_at = ?2 WHERE id = ?3",
                params![if active { 1 } else { 0 }, now, id],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "device",
                id: id.to_string(),
            });
        }
        Ok(())
    }
}

impl ObservationRepository for SqliteDeviceRepo<'_> {
    fn record_observation(
        &self,
        device_id: &str,
        observation: &DeviceObservation,
    ) -> CoreResult<StoredObservation> {
        // Validate device exists
        let dev = self.get_device(device_id)?;
        if dev.as_ref().is_none_or(|d| !d.active) {
            return Err(CoreError::NotFound {
                entity: "device",
                id: device_id.to_string(),
            });
        }

        let obs_id = ObservationId::new();
        let now = OffsetDateTime::now_utc();
        let now_str = format_time(now);
        let obs_type = observation.observation_type_str();
        let payload = serde_json::to_string(observation).map_err(|e| {
            CoreError::InvalidInput(format!("failed to serialize observation: {}", e))
        })?;

        self.conn
            .execute(
                "INSERT INTO observations (
                    id, device_id, observation_type, payload, status, created_at
                 ) VALUES (?1, ?2, ?3, ?4, 'pending_review', ?5)",
                params![obs_id.0.to_string(), device_id, obs_type, payload, now_str,],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(StoredObservation {
            id: obs_id,
            device_id: device_id.to_string(),
            observation_type: obs_type.to_string(),
            payload,
            status: ObservationStatus::PendingReview,
            processed_operation_id: None,
            created_at: now,
            confirmed_at: None,
            confirmed_by: None,
        })
    }

    fn list_observations(
        &self,
        pending_only: bool,
        device_id: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<StoredObservation>> {
        let mut query = String::from(
            "SELECT id, device_id, observation_type, payload, status,
                    processed_operation_id, created_at, confirmed_at, confirmed_by
             FROM observations WHERE 1=1",
        );

        let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

        if pending_only {
            query.push_str(" AND status = 'pending_review'");
        }

        if let Some(dev) = device_id {
            params_vec.push(rusqlite::types::Value::Text(dev.to_string()));
            query.push_str(&format!(" AND device_id = ?{}", params_vec.len()));
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");
        params_vec.push(rusqlite::types::Value::Integer(limit as i64));

        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_vec), row_to_observation)
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut observations = Vec::new();
        for row in rows {
            observations.push(row.map_err(|e| CoreError::Storage(e.to_string()))?);
        }
        Ok(observations)
    }

    fn get_observation(&self, id: &ObservationId) -> CoreResult<Option<StoredObservation>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, device_id, observation_type, payload, status,
                        processed_operation_id, created_at, confirmed_at, confirmed_by
                 FROM observations WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![id.0.to_string()], row_to_observation)
            .optional()
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        Ok(result)
    }

    fn confirm_observation(
        &self,
        id: &ObservationId,
        actor_id: &ActorId,
        operation_id: Option<&OperationId>,
    ) -> CoreResult<()> {
        let now = format_time(OffsetDateTime::now_utc());
        let rows = self
            .conn
            .execute(
                "UPDATE observations
                 SET status = 'confirmed', confirmed_at = ?1, confirmed_by = ?2, processed_operation_id = ?3
                 WHERE id = ?4 AND status = 'pending_review'",
                params![
                    now,
                    actor_id.0.to_string(),
                    operation_id.map(|op| op.0.to_string()),
                    id.0.to_string(),
                ],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "pending observation",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    fn reject_observation(&self, id: &ObservationId, actor_id: &ActorId) -> CoreResult<()> {
        let now = format_time(OffsetDateTime::now_utc());
        let rows = self
            .conn
            .execute(
                "UPDATE observations
                 SET status = 'rejected', confirmed_at = ?1, confirmed_by = ?2
                 WHERE id = ?3 AND status = 'pending_review'",
                params![now, actor_id.0.to_string(), id.0.to_string()],
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "pending observation",
                id: id.to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_memory;
    use crate::migrate::run_migrations;

    #[test]
    fn test_device_crud_lifecycle() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        let repo = SqliteDeviceRepo::new(&conn);

        let new_dev = NewDevice::new(
            "scanner-01",
            "Front Desk Scanner",
            DeviceKind::SerialScanner,
        )
        .with_port("COM3")
        .with_settings(r#"{"baud_rate": 9600}"#);

        let registered = repo.register_device(&new_dev).unwrap();
        assert_eq!(registered.id, "scanner-01");
        assert_eq!(registered.name, "Front Desk Scanner");
        assert_eq!(registered.kind, DeviceKind::SerialScanner);
        assert_eq!(registered.port_or_address.as_deref(), Some("COM3"));
        assert!(registered.active);

        // Duplicate ID rejected
        assert!(repo.register_device(&new_dev).is_err());

        // Fetch
        let fetched = repo.get_device("scanner-01").unwrap().unwrap();
        assert_eq!(fetched.id, registered.id);

        // Update status
        repo.update_device_status("scanner-01", false).unwrap();
        let disabled = repo.get_device("scanner-01").unwrap().unwrap();
        assert!(!disabled.active);

        // Delete
        repo.delete_device("scanner-01").unwrap();
        assert!(repo.get_device("scanner-01").unwrap().is_none());
    }

    #[test]
    fn test_observation_workflow_confirm_and_reject() {
        let conn = open_memory().unwrap();
        run_migrations(&conn).unwrap();
        let repo = SqliteDeviceRepo::new(&conn);

        // Register device
        repo.register_device(&NewDevice::new(
            "ir-shelf-1",
            "IR Shelf Detector",
            DeviceKind::IrBreakbeam,
        ))
        .unwrap();

        // Record observation
        let obs = repo
            .record_observation(
                "ir-shelf-1",
                &DeviceObservation::BeamInterrupted {
                    zone: "Drawer 1".to_string(),
                },
            )
            .unwrap();
        assert_eq!(obs.status, ObservationStatus::PendingReview);
        assert_eq!(obs.device_id, "ir-shelf-1");

        // List pending
        let pending = repo.list_observations(true, None, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, obs.id);

        // Create actor to confirm
        let now_str = format_time(OffsetDateTime::now_utc());
        let actor_id = ActorId::new();
        conn.execute(
            "INSERT INTO actors (id, display_name, role, active, created_at)
             VALUES (?1, 'Operator Dan', 'operator', 1, ?2)",
            params![actor_id.0.to_string(), now_str],
        )
        .unwrap();

        // Confirm observation
        repo.confirm_observation(&obs.id, &actor_id, None).unwrap();

        let confirmed = repo.get_observation(&obs.id).unwrap().unwrap();
        assert_eq!(confirmed.status, ObservationStatus::Confirmed);
        assert_eq!(confirmed.confirmed_by, Some(actor_id));
        assert!(confirmed.confirmed_at.is_some());

        // Now pending list is empty
        assert_eq!(repo.list_observations(true, None, 10).unwrap().len(), 0);

        // Record second observation and reject it
        let obs2 = repo
            .record_observation(
                "ir-shelf-1",
                &DeviceObservation::PresenceChanged {
                    zone: "Drawer 1".to_string(),
                    present: false,
                },
            )
            .unwrap();

        repo.reject_observation(&obs2.id, &actor_id).unwrap();
        let rejected = repo.get_observation(&obs2.id).unwrap().unwrap();
        assert_eq!(rejected.status, ObservationStatus::Rejected);
    }
}
