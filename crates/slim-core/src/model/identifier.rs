//! Identifier domain model — maps barcodes, QR codes, etc. to entities.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::item::ItemId;
use super::location::LocationId;

/// Unique identifier for an identifier record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentifierId(pub Uuid);

impl IdentifierId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for IdentifierId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for IdentifierId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for IdentifierId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// The kind of identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierKind {
    Barcode,
    Qr,
    Nfc,
    Rfid,
    Manual,
}

impl std::fmt::Display for IdentifierKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            IdentifierKind::Barcode => "barcode",
            IdentifierKind::Qr => "qr",
            IdentifierKind::Nfc => "nfc",
            IdentifierKind::Rfid => "rfid",
            IdentifierKind::Manual => "manual",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for IdentifierKind {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "barcode" => Ok(IdentifierKind::Barcode),
            "qr" => Ok(IdentifierKind::Qr),
            "nfc" => Ok(IdentifierKind::Nfc),
            "rfid" => Ok(IdentifierKind::Rfid),
            "manual" => Ok(IdentifierKind::Manual),
            _ => Err(crate::error::CoreError::InvalidInput(format!(
                "unknown identifier kind: {s}"
            ))),
        }
    }
}

/// What an identifier resolves to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IdentifierTarget {
    Item(ItemId),
    Location(LocationId),
}

/// A registered identifier record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identifier {
    pub id: IdentifierId,
    pub kind: IdentifierKind,
    pub value: String,
    pub target: IdentifierTarget,
    pub created_at: time::OffsetDateTime,
}

/// Data required to create a new identifier.
#[derive(Debug, Clone)]
pub struct NewIdentifier {
    pub kind: IdentifierKind,
    pub value: String,
    pub target: IdentifierTarget,
}

impl NewIdentifier {
    /// Validate the new identifier data.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.value.trim().is_empty() {
            return Err(crate::error::CoreError::InvalidInput(
                "identifier value must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_kind_round_trip() {
        for kind in [
            IdentifierKind::Barcode,
            IdentifierKind::Qr,
            IdentifierKind::Nfc,
            IdentifierKind::Rfid,
            IdentifierKind::Manual,
        ] {
            let s = kind.to_string();
            let parsed: IdentifierKind = s.parse().unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn new_identifier_rejects_empty_value() {
        let ident = NewIdentifier {
            kind: IdentifierKind::Barcode,
            value: "  ".to_string(),
            target: IdentifierTarget::Item(ItemId::new()),
        };
        assert!(ident.validate().is_err());
    }

    #[test]
    fn identifier_serde_round_trip() {
        let ident = Identifier {
            id: IdentifierId::new(),
            kind: IdentifierKind::Qr,
            value: "INV:ITEM:M3X10".to_string(),
            target: IdentifierTarget::Item(ItemId::new()),
            created_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&ident).unwrap();
        let back: Identifier = serde_json::from_str(&json).unwrap();
        assert_eq!(ident.id, back.id);
        assert_eq!(ident.value, back.value);
    }

    #[test]
    fn identifier_target_location_serde() {
        let loc_id = LocationId::new();
        let target = IdentifierTarget::Location(loc_id);
        let json = serde_json::to_string(&target).unwrap();
        let back: IdentifierTarget = serde_json::from_str(&json).unwrap();
        match back {
            IdentifierTarget::Location(l) => assert_eq!(l, loc_id),
            _ => panic!("expected location target"),
        }
    }

    #[test]
    fn identifier_kind_invalid_str() {
        let res: Result<IdentifierKind, _> = "unknown-scanner".parse();
        assert!(res.is_err());
    }

    #[test]
    fn identifier_id_default_and_invalid_parse() {
        let id1 = IdentifierId::default();
        let id2 = IdentifierId::default();
        assert_ne!(id1, id2);

        let res: Result<IdentifierId, _> = "bad-uuid".parse();
        assert!(res.is_err());
    }

    #[test]
    fn new_identifier_accepts_valid() {
        let ident = NewIdentifier {
            kind: IdentifierKind::Manual,
            value: "BOX-A".to_string(),
            target: IdentifierTarget::Location(LocationId::new()),
        };
        assert!(ident.validate().is_ok());
    }
}
