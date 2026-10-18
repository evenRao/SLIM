//! Hardware device and sensor observation commands.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;
use slim_core::model::device::{
    DeviceKind, DeviceObservation, NewDevice, ObservationId, ObservationStatus, parse_serial_frame,
};
use slim_core::model::operation::{IssueStock, OperationSource, ReceiveStock};
use slim_core::service::{
    DeviceRepository, InventoryService, LocationRepository, ObservationRepository,
};
use slim_db::repo::device::SqliteDeviceRepo;
use slim_db::repo::inventory::SqliteInventoryService;
use slim_db::repo::location::SqliteLocationRepo;
use uuid::Uuid;

use super::resolve::{resolve_actor, resolve_item, resolve_location};
use crate::cli::{
    AddDeviceArgs, ConfirmObservationArgs, DeviceCommands, ListObservationsArgs, ListenDeviceArgs,
    ObservationCommands, RejectObservationArgs, RemoveDeviceArgs, SimulateDeviceArgs,
};

/// Handle `slim device` subcommands.
pub fn handle_device(conn: &Connection, cmd: DeviceCommands, json: bool) -> Result<()> {
    let repo = SqliteDeviceRepo::new(conn);
    let loc_repo = SqliteLocationRepo::new(conn);

    match cmd {
        DeviceCommands::List => {
            let devices = repo.list_devices()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else if devices.is_empty() {
                println!("No devices registered. Register one with 'slim device add'.");
            } else {
                println!(
                    "{:<16} {:<20} {:<16} {:<12} {:<18} Active",
                    "Device ID", "Name", "Kind", "Port", "Location"
                );
                println!("{}", "─".repeat(88));
                for dev in &devices {
                    let loc_name = match dev.location_id {
                        Some(ref lid) => loc_repo
                            .get_by_id(lid)
                            .ok()
                            .flatten()
                            .map(|l| l.name)
                            .unwrap_or_else(|| lid.to_string()),
                        None => "─".to_string(),
                    };
                    let port_str = dev.port_or_address.as_deref().unwrap_or("─");
                    println!(
                        "{:<16} {:<20} {:<16} {:<12} {:<18} {}",
                        dev.id,
                        dev.name,
                        dev.kind,
                        port_str,
                        loc_name,
                        if dev.active { "yes" } else { "no" }
                    );
                }
            }
        }

        DeviceCommands::Add(args) => {
            let AddDeviceArgs {
                id,
                name,
                kind,
                port,
                location,
                baud: _,
            } = *args;

            let parsed_kind: DeviceKind = kind.parse().map_err(|e| anyhow!("{}", e))?;

            let location_id = if let Some(ref loc_name) = location {
                let loc = resolve_location(conn, loc_name)?;
                Some(loc.id)
            } else {
                None
            };

            let new_dev = NewDevice {
                id: id.clone(),
                name: name.clone(),
                kind: parsed_kind,
                port_or_address: port,
                location_id,
                settings_json: None,
            };

            let registered = repo.register_device(&new_dev)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&registered)?);
            } else {
                println!(
                    "Registered device '{}' ({}, kind: {})",
                    registered.id, registered.name, registered.kind
                );
            }
        }

        DeviceCommands::Remove(RemoveDeviceArgs { id }) => {
            repo.delete_device(&id)?;

            if json {
                println!("{{\"removed\": \"{}\"}}", id);
            } else {
                println!("Device '{}' unregistered successfully.", id);
            }
        }

        DeviceCommands::Simulate(args) => {
            let SimulateDeviceArgs {
                device_id,
                event,
                zone,
                value,
                present,
                uid,
            } = *args;

            let obs = match event.to_lowercase().as_str() {
                "beam_interrupted" => DeviceObservation::BeamInterrupted {
                    zone: zone.unwrap_or_else(|| "ZONE-1".to_string()),
                },
                "presence_changed" => DeviceObservation::PresenceChanged {
                    zone: zone.unwrap_or_else(|| "ZONE-1".to_string()),
                    present: present.unwrap_or(true),
                },
                "identifier_scanned" => {
                    let val = value.ok_or_else(|| {
                        anyhow!("--value is required for identifier_scanned event")
                    })?;
                    DeviceObservation::IdentifierScanned { value: val }
                }
                "tag_detected" => {
                    let tag_uid =
                        uid.ok_or_else(|| anyhow!("--uid is required for tag_detected event"))?;
                    DeviceObservation::TagDetected { uid: tag_uid }
                }
                other => bail!(
                    "unknown event type '{}'. Expected: beam_interrupted, presence_changed, identifier_scanned, tag_detected",
                    other
                ),
            };

            let stored = repo.record_observation(&device_id, &obs)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&stored)?);
            } else {
                println!(
                    "Recorded observation {} for device '{}' (status: pending_review)",
                    stored.id, device_id
                );
                println!("Type:    {}", stored.observation_type);
                println!("Payload: {}", stored.payload);
            }
        }

        DeviceCommands::Listen(args) => {
            let ListenDeviceArgs {
                device,
                port,
                baud,
                once,
                poll_interval_ms,
            } = args;

            let (port_name, baud_rate) = if let Some(p) = port {
                (p, baud)
            } else if let Some(ref dev_id) = device {
                let dev = repo
                    .get_device(dev_id)?
                    .ok_or_else(|| anyhow!("device '{}' not found", dev_id))?;
                let p = dev.port_or_address.ok_or_else(|| {
                    anyhow!("device '{}' has no configured port or address", dev_id)
                })?;
                (p, baud)
            } else {
                bail!("Specify either --port <PORT> or --device <DEVICE_ID>");
            };

            println!(
                "Listening for serial frames on '{}' at {} baud (poll: {}ms)...",
                port_name, baud_rate, poll_interval_ms
            );

            let port_res = serialport::new(&port_name, baud_rate)
                .timeout(Duration::from_millis(poll_interval_ms.max(50)))
                .open();

            let serial = match port_res {
                Ok(s) => s,
                Err(e) => {
                    bail!(
                        "Could not open serial port '{}': {}. Ensure the port exists and is not open by another process.",
                        port_name,
                        e
                    );
                }
            };

            let mut reader = BufReader::new(serial);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        std::thread::sleep(Duration::from_millis(poll_interval_ms));
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match parse_serial_frame(trimmed) {
                            Ok(frame) => {
                                let fallback_id =
                                    frame.device_id.as_deref().unwrap_or("unknown_device");
                                let target_dev = device.as_deref().unwrap_or(fallback_id);
                                match repo.record_observation(target_dev, &frame.observation) {
                                    Ok(obs) => {
                                        if json {
                                            println!("{}", serde_json::to_string(&obs)?);
                                        } else {
                                            println!(
                                                "[{}] Observation from '{}': {} (ID: {})",
                                                obs.created_at,
                                                target_dev,
                                                obs.observation_type,
                                                obs.id
                                            );
                                        }

                                        if once {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to record observation: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Malformed serial frame: {}", e);
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                        // Polling timeout, continue looping
                    }
                    Err(e) => {
                        bail!("Serial read error: {}", e);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle `slim observation` subcommands.
pub fn handle_observation(
    conn: &Connection,
    cmd: ObservationCommands,
    cli_actor: Option<&str>,
    config_actor: Option<&str>,
    json: bool,
) -> Result<()> {
    let repo = SqliteDeviceRepo::new(conn);

    match cmd {
        ObservationCommands::List(ListObservationsArgs {
            pending,
            device,
            limit,
        }) => {
            let observations = repo.list_observations(pending, device.as_deref(), limit)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&observations)?);
            } else if observations.is_empty() {
                println!("No observations recorded.");
            } else {
                println!(
                    "{:<36} {:<16} {:<20} {:<15} Status",
                    "Observation ID", "Device ID", "Timestamp", "Type"
                );
                println!("{}", "─".repeat(95));
                for obs in &observations {
                    let ts_str = obs.created_at.to_string();
                    let short_ts = if ts_str.len() > 19 {
                        &ts_str[..19]
                    } else {
                        &ts_str
                    };
                    println!(
                        "{:<36} {:<16} {:<20} {:<15} {:?}",
                        obs.id, obs.device_id, short_ts, obs.observation_type, obs.status,
                    );
                }
            }
        }

        ObservationCommands::Confirm(args) => {
            let ConfirmObservationArgs {
                id,
                sku,
                qty,
                kind,
                location,
                reason,
            } = *args;

            let obs_uuid =
                Uuid::parse_str(&id).map_err(|_| anyhow!("invalid observation UUID '{}'", id))?;
            let obs_id = ObservationId(obs_uuid);

            let obs = repo
                .get_observation(&obs_id)?
                .ok_or_else(|| anyhow!("observation '{}' not found", obs_id))?;

            if obs.status != ObservationStatus::PendingReview {
                bail!(
                    "observation '{}' is not pending review (status: {:?})",
                    obs_id,
                    obs.status
                );
            }

            let actor = resolve_actor(conn, cli_actor, config_actor)?;

            let op_id = if let Some(ref item_sku) = sku {
                let item = resolve_item(conn, item_sku)?;
                let loc = if let Some(ref loc_name) = location {
                    resolve_location(conn, loc_name)?
                } else {
                    let dev = repo
                        .get_device(&obs.device_id)?
                        .ok_or_else(|| anyhow!("device '{}' not found", obs.device_id))?;
                    if let Some(loc_id) = dev.location_id {
                        SqliteLocationRepo::new(conn)
                            .get_by_id(&loc_id)?
                            .ok_or_else(|| anyhow!("mapped location '{}' not found", loc_id))?
                    } else {
                        bail!(
                            "No location specified and device '{}' has no mapped location. Provide --location <LOC>",
                            obs.device_id
                        );
                    }
                };

                let quantity = qty.unwrap_or(1);
                let notes = reason.unwrap_or_else(|| {
                    format!(
                        "Hardware observation {} confirmed by {}",
                        obs_id, actor.display_name
                    )
                });
                let idempotency_key = format!("obs-confirm-{}", obs_id);

                let inv_svc = SqliteInventoryService::new(conn);
                let receipt = match kind.to_lowercase().as_str() {
                    "receive" => inv_svc.receive(ReceiveStock {
                        item_id: item.id,
                        destination_id: loc.id,
                        quantity,
                        source: OperationSource::Cli,
                        actor_id: actor.id,
                        reason: Some(notes),
                        idempotency_key: Some(idempotency_key),
                    })?,
                    "issue" => inv_svc.issue(IssueStock {
                        item_id: item.id,
                        source_id: loc.id,
                        quantity,
                        source: OperationSource::Cli,
                        actor_id: actor.id,
                        reason: Some(notes),
                        idempotency_key: Some(idempotency_key),
                    })?,
                    other => {
                        bail!(
                            "Invalid operation kind '{}'. Use 'issue' or 'receive'.",
                            other
                        )
                    }
                };

                Some(receipt.operation_id)
            } else {
                None
            };

            repo.confirm_observation(&obs_id, &actor.id, op_id.as_ref())?;

            if json {
                let updated = repo.get_observation(&obs_id)?;
                println!("{}", serde_json::to_string_pretty(&updated)?);
            } else {
                println!(
                    "Observation {} confirmed by actor '{}'.",
                    obs_id, actor.display_name
                );
                if let Some(ref op) = op_id {
                    println!("Triggered stock operation: {}", op);
                }
            }
        }

        ObservationCommands::Reject(RejectObservationArgs { id, reason }) => {
            let obs_uuid =
                Uuid::parse_str(&id).map_err(|_| anyhow!("invalid observation UUID '{}'", id))?;
            let obs_id = ObservationId(obs_uuid);

            let actor = resolve_actor(conn, cli_actor, config_actor)?;
            repo.reject_observation(&obs_id, &actor.id)?;

            if json {
                let updated = repo.get_observation(&obs_id)?;
                println!("{}", serde_json::to_string_pretty(&updated)?);
            } else {
                println!(
                    "Observation {} rejected by actor '{}'.",
                    obs_id, actor.display_name
                );
                if let Some(ref r) = reason {
                    println!("Reason: {}", r);
                }
            }
        }
    }

    Ok(())
}
