//! Item domain model — represents a tracked catalog item (SKU).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemId(pub Uuid);

impl ItemId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ItemId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for ItemId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// A catalog item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub sku: Option<String>,
    pub name: String,
    pub unit: String,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub archived_at: Option<time::OffsetDateTime>,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
}

/// Data required to create a new item.
#[derive(Debug, Clone)]
pub struct NewItem {
    pub sku: Option<String>,
    pub name: String,
    pub unit: String,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl NewItem {
    /// Validate the new item data.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::CoreError::InvalidInput(
                "item name must not be empty".to_string(),
            ));
        }
        if self.unit.trim().is_empty() {
            return Err(crate::error::CoreError::InvalidInput(
                "item unit must not be empty".to_string(),
            ));
        }
        if let Some(ref sku) = self.sku
            && sku.trim().is_empty()
        {
            return Err(crate::error::CoreError::InvalidInput(
                "item SKU must not be empty if provided".to_string(),
            ));
        }
        Ok(())
    }
}

/// Fields that can be updated on an existing item.
#[derive(Debug, Clone, Default)]
pub struct ItemUpdate {
    pub sku: Option<Option<String>>,
    pub name: Option<String>,
    pub unit: Option<String>,
    pub description: Option<Option<String>>,
    pub metadata: Option<Option<serde_json::Value>>,
}

/// Filters for listing items.
#[derive(Debug, Clone, Default)]
pub struct ItemFilter {
    pub search: Option<String>,
    pub include_archived: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_id_round_trip() {
        let id = ItemId::new();
        let s = id.to_string();
        let parsed: ItemId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn new_item_rejects_empty_name() {
        let item = NewItem {
            sku: Some("TEST".to_string()),
            name: "".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        };
        assert!(item.validate().is_err());
    }

    #[test]
    fn new_item_rejects_empty_unit() {
        let item = NewItem {
            sku: None,
            name: "Test".to_string(),
            unit: "  ".to_string(),
            description: None,
            metadata: None,
        };
        assert!(item.validate().is_err());
    }

    #[test]
    fn new_item_rejects_whitespace_sku() {
        let item = NewItem {
            sku: Some("  ".to_string()),
            name: "Test".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        };
        assert!(item.validate().is_err());
    }

    #[test]
    fn new_item_accepts_no_sku() {
        let item = NewItem {
            sku: None,
            name: "Test".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
        };
        assert!(item.validate().is_ok());
    }

    #[test]
    fn item_serde_round_trip() {
        let item = Item {
            id: ItemId::new(),
            sku: Some("M3X10".to_string()),
            name: "M3x10 Screw".to_string(),
            unit: "each".to_string(),
            description: Some("Stainless steel".to_string()),
            metadata: None,
            archived_at: None,
            created_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: Item = serde_json::from_str(&json).unwrap();
        assert_eq!(item.id, back.id);
        assert_eq!(item.sku, back.sku);
    }

    #[test]
    fn item_with_metadata_serde() {
        let metadata = serde_json::json!({
            "color": "silver",
            "thread_pitch": 0.5,
            "head_type": "socket"
        });
        let item = Item {
            id: ItemId::new(),
            sku: Some("M3X10-SS".to_string()),
            name: "M3x10 Socket Screw".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: Some(metadata.clone()),
            archived_at: None,
            created_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: Item = serde_json::from_str(&json).unwrap();
        assert_eq!(back.metadata, Some(metadata));
    }

    #[test]
    fn item_id_default() {
        let id1 = ItemId::default();
        let id2 = ItemId::default();
        assert_ne!(id1, id2);
    }

    #[test]
    fn item_id_from_str_invalid() {
        let res: Result<ItemId, _> = "invalid-uuid".parse();
        assert!(res.is_err());
    }

    #[test]
    fn item_update_and_filter_default() {
        let update = ItemUpdate::default();
        assert!(update.name.is_none());
        assert!(update.unit.is_none());

        let filter = ItemFilter::default();
        assert!(filter.search.is_none());
        assert!(!filter.include_archived);
    }
}
