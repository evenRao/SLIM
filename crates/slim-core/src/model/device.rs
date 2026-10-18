//! Hardware device and observation domain models for SLIM.
//!
//! Provides models for registered hardware sensors (IR, serial scanners, RFID/NFC),
//! versioned serial protocol framing, raw device observations, and confirmation workflows.

use std::fmt;
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};
use crate::model::actor::ActorId;
use crate::model::location::LocationId;
use crate::model::operation::OperationId;

/// Supported kinds of hardware input devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    SerialScanner,
    IrBreakbeam,
    IrPresence,
    RfidNfc,
    MockSensor,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceKind::SerialScanner => write!(f, "serial_scanner"),
            DeviceKind::IrBreakbeam => write!(f, "ir_breakbeam"),
            DeviceKind::IrPresence => write!(f, "ir_presence"),
            DeviceKind::RfidNfc => write!(f, "rfid_nfc"),
            DeviceKind::MockSensor => write!(f, "mock_sensor"),
        }
    }
}

impl FromStr for DeviceKind {
    type Err = CoreError;

    fn from_str(s: &str) -> CoreResult<Self> {
        match s.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "serial_scanner" | "serial" | "scanner" => Ok(DeviceKind::SerialScanner),
            "ir_breakbeam" | "breakbeam" | "ir_beam" => Ok(DeviceKind::IrBreakbeam),
            "ir_presence" | "presence" => Ok(DeviceKind::IrPresence),
            "rfid_nfc" | "rfid" | "nfc" => Ok(DeviceKind::RfidNfc),
            "mock_sensor" | "mock" => Ok(DeviceKind::MockSensor),
            _ => Err(CoreError::InvalidInput(format!(
                "invalid device kind '{}', expected serial_scanner, ir_breakbeam, ir_presence, rfid_nfc, or mock_sensor",
                s
            ))),
        }
    }
}

/// A raw physical device observation event.
///
/// Observations describe what a sensor detected, but DO NOT modify stock directly.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceObservation {
    IdentifierScanned { value: String },
    PresenceChanged { zone: String, present: bool },
    BeamInterrupted { zone: String },
    TagDetected { uid: String },
}

impl DeviceObservation {
    pub fn observation_type_str(&self) -> &'static str {
        match self {
            DeviceObservation::IdentifierScanned { .. } => "IdentifierScanned",
            DeviceObservation::PresenceChanged { .. } => "PresenceChanged",
            DeviceObservation::BeamInterrupted { .. } => "BeamInterrupted",
            DeviceObservation::TagDetected { .. } => "TagDetected",
        }
    }
}

/// Workflow review status for a hardware observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    PendingReview,
    Confirmed,
    Rejected,
    Processed,
}

impl fmt::Display for ObservationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObservationStatus::PendingReview => write!(f, "pending_review"),
            ObservationStatus::Confirmed => write!(f, "confirmed"),
            ObservationStatus::Rejected => write!(f, "rejected"),
            ObservationStatus::Processed => write!(f, "processed"),
        }
    }
}

impl FromStr for ObservationStatus {
    type Err = CoreError;

    fn from_str(s: &str) -> CoreResult<Self> {
        match s.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "pending_review" | "pending" => Ok(ObservationStatus::PendingReview),
            "confirmed" => Ok(ObservationStatus::Confirmed),
            "rejected" => Ok(ObservationStatus::Rejected),
            "processed" => Ok(ObservationStatus::Processed),
            _ => Err(CoreError::InvalidInput(format!(
                "invalid observation status '{}', expected pending_review, confirmed, rejected, or processed",
                s
            ))),
        }
    }
}

/// Unique identifier for a recorded device observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ObservationId(pub Uuid);

impl ObservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for ObservationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ObservationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ObservationId {
    type Err = CoreError;

    fn from_str(s: &str) -> CoreResult<Self> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|e| CoreError::InvalidInput(format!("invalid observation ID '{}': {}", s, e)))
    }
}

/// A registered hardware device in the system inventory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisteredDevice {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub port_or_address: Option<String>,
    pub location_id: Option<LocationId>,
    pub settings_json: Option<String>,
    pub active: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// Input model for registering a new hardware device.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewDevice {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub port_or_address: Option<String>,
    pub location_id: Option<LocationId>,
    pub settings_json: Option<String>,
}

impl NewDevice {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: DeviceKind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            port_or_address: None,
            location_id: None,
            settings_json: None,
        }
    }

    pub fn with_port(mut self, port: impl Into<String>) -> Self {
        self.port_or_address = Some(port.into());
        self
    }

    pub fn with_location(mut self, location_id: LocationId) -> Self {
        self.location_id = Some(location_id);
        self
    }

    pub fn with_settings(mut self, settings: impl Into<String>) -> Self {
        self.settings_json = Some(settings.into());
        self
    }

    pub fn validate(&self) -> CoreResult<()> {
        if self.id.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "device ID cannot be empty".to_string(),
            ));
        }
        if self.name.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "device name cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}

/// A persisted observation in the database.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredObservation {
    pub id: ObservationId,
    pub device_id: String,
    pub observation_type: String,
    pub payload: String,
    pub status: ObservationStatus,
    pub processed_operation_id: Option<OperationId>,
    pub created_at: OffsetDateTime,
    pub confirmed_at: Option<OffsetDateTime>,
    pub confirmed_by: Option<ActorId>,
}

// ── Versioned Serial Framing Protocol ──

pub const CURRENT_PROTOCOL_VERSION: u32 = 1;

/// A versioned protocol frame transmitted over serial or streaming connections.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SerialFrame {
    pub v: u32,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(flatten)]
    pub observation: DeviceObservation,
}

impl SerialFrame {
    pub fn new(device_id: Option<String>, observation: DeviceObservation) -> Self {
        Self {
            v: CURRENT_PROTOCOL_VERSION,
            device_id,
            observation,
        }
    }
}

/// Errors related to hardware device communication and serial protocol framing.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DeviceError {
    #[error("connection error: {0}")]
    Connection(String),

    #[error("unsupported protocol version {version}, expected {expected}")]
    UnsupportedProtocolVersion { version: u32, expected: u32 },

    #[error("malformed frame: {0}")]
    MalformedFrame(String),

    #[error("device I/O error: {0}")]
    Io(String),
}

/// Parse a raw line into a validated `SerialFrame`.
pub fn parse_serial_frame(line: &str) -> Result<SerialFrame, DeviceError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(DeviceError::MalformedFrame("empty line".to_string()));
    }

    let frame: SerialFrame = serde_json::from_str(trimmed)
        .map_err(|e| DeviceError::MalformedFrame(format!("JSON parse error: {}", e)))?;

    if frame.v != CURRENT_PROTOCOL_VERSION {
        return Err(DeviceError::UnsupportedProtocolVersion {
            version: frame.v,
            expected: CURRENT_PROTOCOL_VERSION,
        });
    }

    Ok(frame)
}

/// Encode a `SerialFrame` into a newline-delimited JSON string.
pub fn format_serial_frame(frame: &SerialFrame) -> Result<String, DeviceError> {
    let json = serde_json::to_string(frame)
        .map_err(|e| DeviceError::MalformedFrame(format!("serialization error: {}", e)))?;
    Ok(format!("{}\n", json))
}

/// Abstract adapter interface for hardware inputs.
pub trait DeviceAdapter: Send {
    fn device_id(&self) -> &str;
    fn device_kind(&self) -> DeviceKind;
    fn poll_events(&mut self) -> Result<Vec<DeviceObservation>, DeviceError>;
}

/// In-memory mock adapter for testing hardware sensing without physical hardware.
pub struct MockDeviceAdapter {
    pub id: String,
    pub kind: DeviceKind,
    pub queue: Vec<DeviceObservation>,
}

impl MockDeviceAdapter {
    pub fn new(id: impl Into<String>, kind: DeviceKind) -> Self {
        Self {
            id: id.into(),
            kind,
            queue: Vec::new(),
        }
    }

    pub fn inject(&mut self, observation: DeviceObservation) {
        self.queue.push(observation);
    }
}

impl DeviceAdapter for MockDeviceAdapter {
    fn device_id(&self) -> &str {
        &self.id
    }

    fn device_kind(&self) -> DeviceKind {
        self.kind
    }

    fn poll_events(&mut self) -> Result<Vec<DeviceObservation>, DeviceError> {
        let events = std::mem::take(&mut self.queue);
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_kind_round_trip() {
        for kind in [
            DeviceKind::SerialScanner,
            DeviceKind::IrBreakbeam,
            DeviceKind::IrPresence,
            DeviceKind::RfidNfc,
            DeviceKind::MockSensor,
        ] {
            let s = kind.to_string();
            let parsed: DeviceKind = s.parse().unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn test_serial_protocol_framing_success() {
        // 1. Beam interrupted frame
        let json_line = r#"{"v":1,"device_id":"ir-01","type":"beam_interrupted","zone":"GF-001"}"#;
        let frame = parse_serial_frame(json_line).unwrap();
        assert_eq!(frame.v, 1);
        assert_eq!(frame.device_id.as_deref(), Some("ir-01"));
        assert_eq!(
            frame.observation,
            DeviceObservation::BeamInterrupted {
                zone: "GF-001".to_string()
            }
        );

        // 2. Format back to string
        let formatted = format_serial_frame(&frame).unwrap();
        let reparsed = parse_serial_frame(&formatted).unwrap();
        assert_eq!(frame, reparsed);

        // 3. Identifier scanned frame
        let scan_line = r#"{"v":1,"type":"identifier_scanned","value":"M3X10-BIN"}"#;
        let scan_frame = parse_serial_frame(scan_line).unwrap();
        assert_eq!(
            scan_frame.observation,
            DeviceObservation::IdentifierScanned {
                value: "M3X10-BIN".to_string()
            }
        );
    }

    #[test]
    fn test_serial_protocol_version_mismatch() {
        let json_line = r#"{"v":99,"type":"beam_interrupted","zone":"GF-001"}"#;
        let res = parse_serial_frame(json_line);
        assert!(matches!(
            res,
            Err(DeviceError::UnsupportedProtocolVersion {
                version: 99,
                expected: 1
            })
        ));
    }

    #[test]
    fn test_serial_protocol_malformed_json() {
        let res = parse_serial_frame("not a valid json");
        assert!(matches!(res, Err(DeviceError::MalformedFrame(_))));
    }

    #[test]
    fn test_mock_device_adapter() {
        let mut adapter = MockDeviceAdapter::new("test-adapter", DeviceKind::IrBreakbeam);
        assert_eq!(adapter.device_id(), "test-adapter");
        assert_eq!(adapter.device_kind(), DeviceKind::IrBreakbeam);

        // Initially empty
        assert_eq!(adapter.poll_events().unwrap().len(), 0);

        // Inject observations
        adapter.inject(DeviceObservation::BeamInterrupted {
            zone: "GF-001".to_string(),
        });
        adapter.inject(DeviceObservation::PresenceChanged {
            zone: "GF-001".to_string(),
            present: true,
        });

        let polled = adapter.poll_events().unwrap();
        assert_eq!(polled.len(), 2);
        assert_eq!(
            polled[0],
            DeviceObservation::BeamInterrupted {
                zone: "GF-001".to_string()
            }
        );

        // Polling drains the queue
        assert_eq!(adapter.poll_events().unwrap().len(), 0);
    }
}
