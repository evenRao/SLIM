//! Location domain model — represents a hierarchical physical place.
//!
//! Locations form a tree: Room > Cabinet > Drawer > Bin > Compartment.
//! Bins are location subtypes (ADR-01), not a separate table.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LocationId(pub Uuid);

impl LocationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LocationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for LocationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for LocationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// The kind of location in the hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationKind {
    Room,
    Cabinet,
    Drawer,
    Shelf,
    Storage,
    Bin,
    Compartment,
    Custom,
}

impl LocationKind {
    /// Whether this kind of location can directly hold stock.
    pub fn can_hold_stock(&self) -> bool {
        matches!(
            self,
            LocationKind::Drawer
                | LocationKind::Shelf
                | LocationKind::Storage
                | LocationKind::Bin
                | LocationKind::Compartment
                | LocationKind::Custom
        )
    }
}

impl std::fmt::Display for LocationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LocationKind::Room => "room",
            LocationKind::Cabinet => "cabinet",
            LocationKind::Drawer => "drawer",
            LocationKind::Shelf => "shelf",
            LocationKind::Storage => "storage",
            LocationKind::Bin => "bin",
            LocationKind::Compartment => "compartment",
            LocationKind::Custom => "custom",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for LocationKind {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "room" => Ok(LocationKind::Room),
            "cabinet" => Ok(LocationKind::Cabinet),
            "drawer" => Ok(LocationKind::Drawer),
            "shelf" => Ok(LocationKind::Shelf),
            "storage" => Ok(LocationKind::Storage),
            "bin" => Ok(LocationKind::Bin),
            "compartment" => Ok(LocationKind::Compartment),
            "custom" | "box" | "bag" | "rack" | "pallet" => Ok(LocationKind::Custom),
            _ => Err(crate::error::CoreError::InvalidInput(format!(
                "unknown location kind: {s}"
            ))),
        }
    }
}

/// A location in the physical hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub id: LocationId,
    pub parent_id: Option<LocationId>,
    pub name: String,
    pub kind: LocationKind,
    pub can_hold_stock: bool,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
}

/// Data required to create a new location.
#[derive(Debug, Clone)]
pub struct NewLocation {
    pub parent_id: Option<LocationId>,
    pub name: String,
    pub kind: LocationKind,
}

impl NewLocation {
    /// Validate the new location data.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::CoreError::InvalidInput(
                "location name must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_id_round_trip() {
        let id = LocationId::new();
        let s = id.to_string();
        let parsed: LocationId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn location_kind_round_trip() {
        for kind in [
            LocationKind::Room,
            LocationKind::Cabinet,
            LocationKind::Drawer,
            LocationKind::Shelf,
            LocationKind::Storage,
            LocationKind::Bin,
            LocationKind::Compartment,
            LocationKind::Custom,
        ] {
            let s = kind.to_string();
            let parsed: LocationKind = s.parse().unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn location_kind_stock_holding() {
        assert!(!LocationKind::Room.can_hold_stock());
        assert!(!LocationKind::Cabinet.can_hold_stock());
        assert!(LocationKind::Drawer.can_hold_stock());
        assert!(LocationKind::Bin.can_hold_stock());
        assert!(LocationKind::Compartment.can_hold_stock());
    }

    #[test]
    fn new_location_rejects_empty_name() {
        let loc = NewLocation {
            parent_id: None,
            name: "  ".to_string(),
            kind: LocationKind::Room,
        };
        assert!(loc.validate().is_err());
    }

    #[test]
    fn location_serde_round_trip() {
        let loc = Location {
            id: LocationId::new(),
            parent_id: None,
            name: "Workshop".to_string(),
            kind: LocationKind::Room,
            can_hold_stock: false,
            created_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&loc).unwrap();
        let back: Location = serde_json::from_str(&json).unwrap();
        assert_eq!(loc.id, back.id);
        assert_eq!(loc.name, back.name);
    }

    #[test]
    fn location_kind_invalid_str() {
        let res: Result<LocationKind, _> = "spaceship".parse();
        assert!(res.is_err());
    }

    #[test]
    fn location_id_default_and_invalid_parse() {
        let id1 = LocationId::default();
        let id2 = LocationId::default();
        assert_ne!(id1, id2);

        let res: Result<LocationId, _> = "invalid-uuid".parse();
        assert!(res.is_err());
    }

    #[test]
    fn new_location_accepts_valid() {
        let loc = NewLocation {
            parent_id: Some(LocationId::new()),
            name: "Bin 01".to_string(),
            kind: LocationKind::Bin,
        };
        assert!(loc.validate().is_ok());
    }
}
