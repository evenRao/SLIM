//! Operation and stock movement domain models.
//!
//! An operation is the transaction envelope (receive, issue, transfer, adjust).
//! Stock movements are the immutable ledger lines within an operation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::actor::ActorId;
use super::item::ItemId;
use super::location::LocationId;

/// Unique identifier for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub Uuid);

impl OperationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OperationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for OperationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Unique identifier for a stock movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MovementId(pub Uuid);

impl MovementId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MovementId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MovementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for MovementId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// The kind of inventory operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Receive,
    Issue,
    Transfer,
    Adjust,
}

impl std::fmt::Display for OperationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OperationKind::Receive => "receive",
            OperationKind::Issue => "issue",
            OperationKind::Transfer => "transfer",
            OperationKind::Adjust => "adjust",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for OperationKind {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "receive" => Ok(OperationKind::Receive),
            "issue" => Ok(OperationKind::Issue),
            "transfer" => Ok(OperationKind::Transfer),
            "adjust" => Ok(OperationKind::Adjust),
            _ => Err(crate::error::CoreError::InvalidInput(format!(
                "unknown operation kind: {s}"
            ))),
        }
    }
}

/// The source/input method of an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationSource {
    Cli,
    Tui,
    Scanner,
    Import,
}

impl std::fmt::Display for OperationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OperationSource::Cli => "cli",
            OperationSource::Tui => "tui",
            OperationSource::Scanner => "scanner",
            OperationSource::Import => "import",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for OperationSource {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cli" => Ok(OperationSource::Cli),
            "tui" => Ok(OperationSource::Tui),
            "scanner" => Ok(OperationSource::Scanner),
            "import" => Ok(OperationSource::Import),
            _ => Err(crate::error::CoreError::InvalidInput(format!(
                "unknown operation source: {s}"
            ))),
        }
    }
}

/// A recorded inventory operation (transaction header).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: OperationId,
    pub kind: OperationKind,
    pub actor_id: ActorId,
    pub source: OperationSource,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: time::OffsetDateTime,
}

/// An immutable stock movement (ledger line).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockMovement {
    pub id: MovementId,
    pub operation_id: OperationId,
    pub item_id: ItemId,
    pub location_id: LocationId,
    pub delta_qty: i64,
    pub created_at: time::OffsetDateTime,
}

// ── Command structs (inputs to the inventory service) ──

/// Command to receive stock.
#[derive(Debug, Clone)]
pub struct ReceiveStock {
    pub actor_id: ActorId,
    pub item_id: ItemId,
    pub destination_id: LocationId,
    pub quantity: u32,
    pub source: OperationSource,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
}

impl ReceiveStock {
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.quantity == 0 {
            return Err(crate::error::CoreError::InvalidInput(
                "receive quantity must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

/// Command to issue (consume/remove) stock.
#[derive(Debug, Clone)]
pub struct IssueStock {
    pub actor_id: ActorId,
    pub item_id: ItemId,
    pub source_id: LocationId,
    pub quantity: u32,
    pub source: OperationSource,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
}

impl IssueStock {
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.quantity == 0 {
            return Err(crate::error::CoreError::InvalidInput(
                "issue quantity must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

/// Command to transfer stock between locations.
#[derive(Debug, Clone)]
pub struct TransferStock {
    pub actor_id: ActorId,
    pub item_id: ItemId,
    pub from_id: LocationId,
    pub to_id: LocationId,
    pub quantity: u32,
    pub source: OperationSource,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
}

impl TransferStock {
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.quantity == 0 {
            return Err(crate::error::CoreError::InvalidInput(
                "transfer quantity must be greater than zero".to_string(),
            ));
        }
        if self.from_id == self.to_id {
            return Err(crate::error::CoreError::InvalidInput(
                "transfer source and destination must be different".to_string(),
            ));
        }
        Ok(())
    }
}

/// Command to adjust stock to a specific quantity.
#[derive(Debug, Clone)]
pub struct AdjustStock {
    pub actor_id: ActorId,
    pub item_id: ItemId,
    pub location_id: LocationId,
    pub new_quantity: u32,
    pub source: OperationSource,
    pub reason: String,
    pub idempotency_key: Option<String>,
}

impl AdjustStock {
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.reason.trim().is_empty() {
            return Err(crate::error::CoreError::InvalidInput(
                "adjustment reason must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

/// Receipt returned after a successful inventory operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationReceipt {
    pub operation_id: OperationId,
    pub kind: OperationKind,
    pub movements: Vec<StockMovement>,
    pub resulting_balances: Vec<crate::model::stock::StockBalance>,
}

/// Summary of an operation for history queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationSummary {
    pub operation: Operation,
    pub movements: Vec<StockMovement>,
}

/// Detailed view of an operation including related entity names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDetail {
    pub operation: Operation,
    pub movements: Vec<StockMovement>,
    pub actor_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_kind_round_trip() {
        for kind in [
            OperationKind::Receive,
            OperationKind::Issue,
            OperationKind::Transfer,
            OperationKind::Adjust,
        ] {
            let s = kind.to_string();
            let parsed: OperationKind = s.parse().unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn receive_rejects_zero_qty() {
        let cmd = ReceiveStock {
            actor_id: ActorId::new(),
            item_id: ItemId::new(),
            destination_id: LocationId::new(),
            quantity: 0,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn issue_rejects_zero_qty() {
        let cmd = IssueStock {
            actor_id: ActorId::new(),
            item_id: ItemId::new(),
            source_id: LocationId::new(),
            quantity: 0,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn transfer_rejects_zero_qty() {
        let cmd = TransferStock {
            actor_id: ActorId::new(),
            item_id: ItemId::new(),
            from_id: LocationId::new(),
            to_id: LocationId::new(),
            quantity: 0,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn transfer_rejects_same_source_dest() {
        let loc = LocationId::new();
        let cmd = TransferStock {
            actor_id: ActorId::new(),
            item_id: ItemId::new(),
            from_id: loc,
            to_id: loc,
            quantity: 10,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn adjust_rejects_empty_reason() {
        let cmd = AdjustStock {
            actor_id: ActorId::new(),
            item_id: ItemId::new(),
            location_id: LocationId::new(),
            new_quantity: 50,
            source: OperationSource::Cli,
            reason: "  ".to_string(),
            idempotency_key: None,
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn adjust_accepts_valid_reason() {
        let cmd = AdjustStock {
            actor_id: ActorId::new(),
            item_id: ItemId::new(),
            location_id: LocationId::new(),
            new_quantity: 50,
            source: OperationSource::Cli,
            reason: "Physical count".to_string(),
            idempotency_key: None,
        };
        assert!(cmd.validate().is_ok());
    }

    #[test]
    fn operation_serde_round_trip() {
        let op = Operation {
            id: OperationId::new(),
            kind: OperationKind::Receive,
            actor_id: ActorId::new(),
            source: OperationSource::Cli,
            reason: Some("Test".to_string()),
            idempotency_key: None,
            created_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(op.id, back.id);
        assert_eq!(op.kind, back.kind);
    }

    #[test]
    fn operation_source_round_trip() {
        for src in [
            OperationSource::Cli,
            OperationSource::Tui,
            OperationSource::Scanner,
            OperationSource::Import,
        ] {
            let s = src.to_string();
            let parsed: OperationSource = s.parse().unwrap();
            assert_eq!(src, parsed);
        }
    }

    #[test]
    fn operation_source_invalid() {
        let res: Result<OperationSource, _> = "magic".parse();
        assert!(res.is_err());
    }

    #[test]
    fn operation_kind_invalid() {
        let res: Result<OperationKind, _> = "magic".parse();
        assert!(res.is_err());
    }

    #[test]
    fn operation_and_movement_id_defaults() {
        let op1 = OperationId::default();
        let op2 = OperationId::default();
        assert_ne!(op1, op2);

        let mv1 = MovementId::default();
        let mv2 = MovementId::default();
        assert_ne!(mv1, mv2);

        let res_op: Result<OperationId, _> = "bad".parse();
        assert!(res_op.is_err());

        let res_mv: Result<MovementId, _> = "bad".parse();
        assert!(res_mv.is_err());
    }

    #[test]
    fn stock_movement_serde_round_trip() {
        let mv = StockMovement {
            id: MovementId::new(),
            operation_id: OperationId::new(),
            item_id: ItemId::new(),
            location_id: LocationId::new(),
            delta_qty: -42,
            created_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&mv).unwrap();
        let back: StockMovement = serde_json::from_str(&json).unwrap();
        assert_eq!(mv.id, back.id);
        assert_eq!(mv.delta_qty, back.delta_qty);
    }

    #[test]
    fn operation_receipt_serde() {
        let receipt = OperationReceipt {
            operation_id: OperationId::new(),
            kind: OperationKind::Receive,
            movements: vec![],
            resulting_balances: vec![],
        };
        let json = serde_json::to_string(&receipt).unwrap();
        let back: OperationReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(receipt.operation_id, back.operation_id);
    }
}
