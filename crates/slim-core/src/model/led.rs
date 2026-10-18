//! LED routing, dock targeting, and priority state machine models.
//!
//! Implements Section 9 of the Gridfinity development plan:
//! - Priority 1: Assigned dock is MISSING (pulsing red/orange)
//! - Priority 2: LOCATE operation for item/bin (cyan/blue attention pulse, time-limited)
//! - Priority 3: LOW STOCK from recorded ledger (amber steady or slow pulse)
//! - Priority 4: Normal / idle (LEDs off)
//! - Special: Unknown / disconnected dock (safe fallback)

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

use super::gridfinity::{GridBaseId, Rotation};
use crate::error::{CoreError, CoreResult};

/// Standard RGB color for Missing Bin state: pulsing red/orange.
pub const COLOR_MISSING: [u8; 3] = [255, 40, 0];

/// Standard RGB color for Locate Attention state: cyan/blue.
pub const COLOR_LOCATE: [u8; 3] = [0, 200, 255];

/// Standard RGB color for Low Stock warning: amber/yellow.
pub const COLOR_LOW_STOCK: [u8; 3] = [255, 180, 0];

/// Standard RGB color for Normal / Off state.
pub const COLOR_OFF: [u8; 3] = [0, 0, 0];

/// Default locate pulse TTL in seconds.
pub const DEFAULT_LOCATE_TTL_SECS: u64 = 30;

/// Logical operating mode for a dock LED.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LedMode {
    #[default]
    Off,
    Locate,
    LowStock,
    Missing,
    Custom,
}

impl LedMode {
    /// Priority level for deterministic arbitration.
    /// Higher values win conflicts.
    pub fn priority_rank(&self) -> u8 {
        match self {
            LedMode::Missing => 4,
            LedMode::Locate => 3,
            LedMode::LowStock => 2,
            LedMode::Custom => 1,
            LedMode::Off => 0,
        }
    }

    /// Return the standard default RGB color for this mode.
    pub fn default_color(&self) -> [u8; 3] {
        match self {
            LedMode::Missing => COLOR_MISSING,
            LedMode::Locate => COLOR_LOCATE,
            LedMode::LowStock => COLOR_LOW_STOCK,
            LedMode::Custom => [255, 255, 255],
            LedMode::Off => COLOR_OFF,
        }
    }
}

impl fmt::Display for LedMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedMode::Off => write!(f, "off"),
            LedMode::Locate => write!(f, "locate"),
            LedMode::LowStock => write!(f, "low_stock"),
            LedMode::Missing => write!(f, "missing"),
            LedMode::Custom => write!(f, "custom"),
        }
    }
}

impl FromStr for LedMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "off" => Ok(LedMode::Off),
            "locate" | "find" => Ok(LedMode::Locate),
            "low_stock" | "low" => Ok(LedMode::LowStock),
            "missing" | "absent" => Ok(LedMode::Missing),
            "custom" => Ok(LedMode::Custom),
            other => Err(CoreError::InvalidInput(format!(
                "unknown LED mode: {other}"
            ))),
        }
    }
}

/// Helper function to format a standardized dock identifier.
/// e.g. `"BASE-A:3:1"`
pub fn format_dock_id(base_code: &str, x: u32, y: u32) -> String {
    format!("{base_code}:{x}:{y}")
}

/// Helper function to parse a dock identifier into `(base_code, x, y)`.
pub fn parse_dock_id(dock_id: &str) -> Option<(String, u32, u32)> {
    let parts: Vec<&str> = dock_id.split(':').collect();
    if parts.len() == 3 {
        let base_code = parts[0].to_string();
        let x = parts[1].parse::<u32>().ok()?;
        let y = parts[2].parse::<u32>().ok()?;
        Some((base_code, x, y))
    } else {
        None
    }
}

/// Calculate all grid docks covered by a bin given its placement and footprint.
///
/// Handles 1x1, 2x1, 2x3, and rotated multi-cell footprints.
pub fn calculate_covered_docks(
    base_code: &str,
    anchor_x: u32,
    anchor_y: u32,
    width_units: u32,
    depth_units: u32,
    rotation: Rotation,
) -> Vec<(String, u32, u32)> {
    let (eff_w, eff_d) = match rotation {
        Rotation::Rot0 => (width_units, depth_units),
        Rotation::Rot90 => (depth_units, width_units),
    };

    let mut docks = Vec::with_capacity((eff_w * eff_d) as usize);
    for dx in 0..eff_w {
        for dy in 0..eff_d {
            let x = anchor_x + dx;
            let y = anchor_y + dy;
            let dock_id = format_dock_id(base_code, x, y);
            docks.push((dock_id, x, y));
        }
    }
    docks
}

/// Persisted or active logical state of an individual dock LED.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockLedState {
    pub dock_id: String,
    pub base_id: GridBaseId,
    pub x: u32,
    pub y: u32,
    pub mode: LedMode,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub expires_at: Option<OffsetDateTime>,
    pub reason: Option<String>,
    pub updated_at: OffsetDateTime,
}

impl DockLedState {
    /// Check whether a time-limited state (e.g. locate pulse) has expired.
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        if let Some(exp) = self.expires_at {
            now >= exp
        } else {
            false
        }
    }

    /// Return RGB color as a 3-byte array.
    pub fn rgb(&self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
}

/// Physical occupancy status for a dock anchor sensor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockPresenceState {
    pub dock_id: String,
    pub base_id: GridBaseId,
    pub x: u32,
    pub y: u32,
    pub occupied: bool,
    pub last_seen_at: OffsetDateTime,
    pub status: String, // "present", "missing", "unknown"
}

/// Serial protocol frame for hardware LED commands (Version 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedProtocolFrame {
    #[serde(rename = "set_led")]
    SetLed {
        v: u8,
        command_id: Uuid,
        dock_id: String,
        mode: String,
        rgb: [u8; 3],
        #[serde(skip_serializing_if = "Option::is_none")]
        ttl_ms: Option<u64>,
    },
    #[serde(rename = "clear_led")]
    ClearLed {
        v: u8,
        command_id: Uuid,
        dock_id: String,
    },
    #[serde(rename = "ack")]
    Ack {
        v: u8,
        controller_id: String,
        command_id: Uuid,
        result: String,
    },
}

impl LedProtocolFrame {
    /// Create a versioned `set_led` command.
    pub fn set_led(
        dock_id: impl Into<String>,
        mode: LedMode,
        rgb: [u8; 3],
        ttl_ms: Option<u64>,
    ) -> Self {
        Self::SetLed {
            v: 1,
            command_id: Uuid::new_v4(),
            dock_id: dock_id.into(),
            mode: mode.to_string(),
            rgb,
            ttl_ms,
        }
    }

    /// Create a versioned `clear_led` command.
    pub fn clear_led(dock_id: impl Into<String>) -> Self {
        Self::ClearLed {
            v: 1,
            command_id: Uuid::new_v4(),
            dock_id: dock_id.into(),
        }
    }

    /// Format frame as a single newline-delimited JSON line.
    pub fn to_json_line(&self) -> CoreResult<String> {
        let json = serde_json::to_string(self)
            .map_err(|e| CoreError::InvalidInput(format!("failed to serialize LED frame: {e}")))?;
        Ok(format!("{json}\n"))
    }
}

/// Deterministic resolution of dock LED states according to the priority hierarchy.
///
/// Priority rule (§9.3):
/// 1. `Missing`: If assigned bin is absent/missing -> Red/Orange
/// 2. `Locate`: If user invoked locate and not expired -> Cyan/Blue
/// 3. `LowStock`: If stock on hand <= threshold -> Amber
/// 4. `Off`: Normal, present, idle -> Off
pub fn resolve_dock_led_priority(
    is_missing: bool,
    locate_expires_at: Option<OffsetDateTime>,
    is_low_stock: bool,
    now: OffsetDateTime,
) -> (LedMode, [u8; 3], Option<OffsetDateTime>, Option<String>) {
    if is_missing {
        (
            LedMode::Missing,
            COLOR_MISSING,
            None,
            Some("Assigned bin is missing from dock".to_string()),
        )
    } else if let Some(exp) = locate_expires_at {
        if exp > now {
            (
                LedMode::Locate,
                COLOR_LOCATE,
                Some(exp),
                Some("Active locate request".to_string()),
            )
        } else if is_low_stock {
            (
                LedMode::LowStock,
                COLOR_LOW_STOCK,
                None,
                Some("Inventory below threshold".to_string()),
            )
        } else {
            (LedMode::Off, COLOR_OFF, None, None)
        }
    } else if is_low_stock {
        (
            LedMode::LowStock,
            COLOR_LOW_STOCK,
            None,
            Some("Inventory below threshold".to_string()),
        )
    } else {
        (LedMode::Off, COLOR_OFF, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_covered_docks_single_cell() {
        let docks = calculate_covered_docks("BASE-1", 2, 3, 1, 1, Rotation::Rot0);
        assert_eq!(docks.len(), 1);
        assert_eq!(docks[0], ("BASE-1:2:3".to_string(), 2, 3));
    }

    #[test]
    fn test_covered_docks_multi_cell_unrotated() {
        let docks = calculate_covered_docks("BASE-A", 1, 1, 2, 3, Rotation::Rot0);
        assert_eq!(docks.len(), 6);
        assert!(docks.contains(&("BASE-A:1:1".to_string(), 1, 1)));
        assert!(docks.contains(&("BASE-A:2:1".to_string(), 2, 1)));
        assert!(docks.contains(&("BASE-A:1:2".to_string(), 1, 2)));
        assert!(docks.contains(&("BASE-A:2:2".to_string(), 2, 2)));
        assert!(docks.contains(&("BASE-A:1:3".to_string(), 1, 3)));
        assert!(docks.contains(&("BASE-A:2:3".to_string(), 2, 3)));
    }

    #[test]
    fn test_covered_docks_multi_cell_rotated_90() {
        // 2x3 rotated 90 becomes 3x2 on the grid
        let docks = calculate_covered_docks("BASE-A", 0, 0, 2, 3, Rotation::Rot90);
        assert_eq!(docks.len(), 6);
        // x in 0..3, y in 0..2
        assert!(docks.contains(&("BASE-A:2:1".to_string(), 2, 1)));
        assert!(docks.contains(&("BASE-A:0:0".to_string(), 0, 0)));
        assert!(!docks.contains(&("BASE-A:0:2".to_string(), 0, 2)));
    }

    #[test]
    fn test_priority_resolution_missing_beats_locate() {
        let now = OffsetDateTime::now_utc();
        let exp = now + time::Duration::seconds(30);

        // When both missing and locate are active, MISSING wins
        let (mode, rgb, _, reason) = resolve_dock_led_priority(true, Some(exp), true, now);
        assert_eq!(mode, LedMode::Missing);
        assert_eq!(rgb, COLOR_MISSING);
        assert!(reason.unwrap().contains("missing"));
    }

    #[test]
    fn test_priority_resolution_locate_beats_low_stock() {
        let now = OffsetDateTime::now_utc();
        let exp = now + time::Duration::seconds(30);

        let (mode, rgb, expires, _) = resolve_dock_led_priority(false, Some(exp), true, now);
        assert_eq!(mode, LedMode::Locate);
        assert_eq!(rgb, COLOR_LOCATE);
        assert_eq!(expires, Some(exp));
    }

    #[test]
    fn test_priority_resolution_low_stock_when_locate_expired() {
        let now = OffsetDateTime::now_utc();
        let expired = now - time::Duration::seconds(5);

        let (mode, rgb, _, _) = resolve_dock_led_priority(false, Some(expired), true, now);
        assert_eq!(mode, LedMode::LowStock);
        assert_eq!(rgb, COLOR_LOW_STOCK);
    }

    #[test]
    fn test_priority_resolution_normal_idle() {
        let now = OffsetDateTime::now_utc();
        let (mode, rgb, _, _) = resolve_dock_led_priority(false, None, false, now);
        assert_eq!(mode, LedMode::Off);
        assert_eq!(rgb, COLOR_OFF);
    }

    #[test]
    fn test_parse_and_format_dock_id() {
        let dock_id = format_dock_id("BASE-DRAWER", 4, 2);
        assert_eq!(dock_id, "BASE-DRAWER:4:2");

        let parsed = parse_dock_id(&dock_id).unwrap();
        assert_eq!(parsed, ("BASE-DRAWER".to_string(), 4, 2));

        assert!(parse_dock_id("invalid").is_none());
    }

    #[test]
    fn test_protocol_frame_serialization() {
        let frame =
            LedProtocolFrame::set_led("BASE-A:1:2", LedMode::Locate, COLOR_LOCATE, Some(30000));
        let line = frame.to_json_line().unwrap();
        assert!(line.contains("\"type\":\"set_led\""));
        assert!(line.contains("\"v\":1"));
        assert!(line.contains("\"dock_id\":\"BASE-A:1:2\""));
        assert!(line.contains("\"mode\":\"locate\""));
        assert!(line.contains("\"ttl_ms\":30000"));
        assert!(line.ends_with('\n'));
    }
}
