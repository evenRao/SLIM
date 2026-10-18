//! Stock balance and threshold domain models.

use serde::{Deserialize, Serialize};

use super::item::ItemId;
use super::location::LocationId;

/// Current stock balance for an item at a location (cached projection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockBalance {
    pub item_id: ItemId,
    pub location_id: LocationId,
    pub qty: u32,
    pub updated_at: time::OffsetDateTime,
}

/// A stock threshold definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockThreshold {
    pub item_id: ItemId,
    /// If None, this is the global threshold for the item.
    pub stock_location_id: Option<LocationId>,
    pub min_qty: u32,
}

/// Alert for items below their threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowStockAlert {
    pub item_id: ItemId,
    pub item_name: String,
    pub item_sku: Option<String>,
    pub location_id: LocationId,
    pub location_name: String,
    pub on_hand: u32,
    pub min_qty: u32,
}

/// Result of a reconciliation check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationReport {
    pub checked_at: time::OffsetDateTime,
    pub total_balances: usize,
    pub mismatches: Vec<ReconciliationMismatch>,
    pub repaired: bool,
}

/// A single mismatch between cached balance and ledger sum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationMismatch {
    pub item_id: ItemId,
    pub location_id: LocationId,
    pub cached_qty: i64,
    pub ledger_qty: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_balance_serde_round_trip() {
        let bal = StockBalance {
            item_id: ItemId::new(),
            location_id: LocationId::new(),
            qty: 42,
            updated_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&bal).unwrap();
        let back: StockBalance = serde_json::from_str(&json).unwrap();
        assert_eq!(bal.item_id, back.item_id);
        assert_eq!(bal.qty, back.qty);
    }

    #[test]
    fn low_stock_alert_serde_round_trip() {
        let alert = LowStockAlert {
            item_id: ItemId::new(),
            item_name: "M3x10 Screw".to_string(),
            item_sku: Some("M3X10".to_string()),
            location_id: LocationId::new(),
            location_name: "Drawer 1".to_string(),
            on_hand: 5,
            min_qty: 50,
        };
        let json = serde_json::to_string(&alert).unwrap();
        let back: LowStockAlert = serde_json::from_str(&json).unwrap();
        assert_eq!(alert.item_name, back.item_name);
        assert_eq!(alert.on_hand, back.on_hand);
    }

    #[test]
    fn reconciliation_mismatch_serde() {
        let m = ReconciliationMismatch {
            item_id: ItemId::new(),
            location_id: LocationId::new(),
            cached_qty: 50,
            ledger_qty: 48,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: ReconciliationMismatch = serde_json::from_str(&json).unwrap();
        assert_eq!(m.cached_qty, back.cached_qty);
        assert_eq!(m.ledger_qty, back.ledger_qty);
    }

    #[test]
    fn stock_threshold_serde_round_trip() {
        let t = StockThreshold {
            item_id: ItemId::new(),
            stock_location_id: Some(LocationId::new()),
            min_qty: 25,
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: StockThreshold = serde_json::from_str(&json).unwrap();
        assert_eq!(t.item_id, back.item_id);
        assert_eq!(t.min_qty, back.min_qty);
    }

    #[test]
    fn reconciliation_report_serde_round_trip() {
        let report = ReconciliationReport {
            checked_at: time::OffsetDateTime::now_utc(),
            total_balances: 5,
            mismatches: vec![],
            repaired: true,
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: ReconciliationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report.total_balances, back.total_balances);
        assert!(back.repaired);
    }
}
