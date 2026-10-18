//! Domain error types for SLIM.

use thiserror::Error;

/// Core domain errors that can occur in any SLIM operation.
#[derive(Debug, Error)]
pub enum CoreError {
    /// The requested entity was not found.
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    /// A duplicate SKU was provided.
    #[error("duplicate SKU: {0}")]
    DuplicateSku(String),

    /// A duplicate identifier (kind, value) was provided.
    #[error("duplicate identifier: ({kind}, {value})")]
    DuplicateIdentifier { kind: String, value: String },

    /// Insufficient stock for the requested operation.
    #[error("insufficient stock: requested {requested}, available {available}")]
    InsufficientStock { requested: u32, available: u32 },

    /// Invalid input provided to an operation.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// An idempotency key conflict was detected (operation already exists).
    #[error("idempotency conflict: operation already exists for key {0}")]
    IdempotencyConflict(String),

    /// The actor does not have authorization to perform this operation.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// A database or storage error occurred.
    #[error("storage error: {0}")]
    Storage(String),
}

/// Convenience result type for core operations.
pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_formatting() {
        let err1 = CoreError::NotFound {
            entity: "item",
            id: "M3X10".to_string(),
        };
        assert_eq!(err1.to_string(), "item not found: M3X10");

        let err2 = CoreError::DuplicateSku("ESP32".to_string());
        assert_eq!(err2.to_string(), "duplicate SKU: ESP32");

        let err3 = CoreError::DuplicateIdentifier {
            kind: "qr".to_string(),
            value: "123".to_string(),
        };
        assert_eq!(err3.to_string(), "duplicate identifier: (qr, 123)");

        let err4 = CoreError::InsufficientStock {
            requested: 10,
            available: 4,
        };
        assert_eq!(
            err4.to_string(),
            "insufficient stock: requested 10, available 4"
        );

        let err5 = CoreError::InvalidInput("empty reason".to_string());
        assert_eq!(err5.to_string(), "invalid input: empty reason");

        let err6 = CoreError::IdempotencyConflict("KEY-1".to_string());
        assert_eq!(
            err6.to_string(),
            "idempotency conflict: operation already exists for key KEY-1"
        );

        let err7 = CoreError::Storage("disk full".to_string());
        assert_eq!(err7.to_string(), "storage error: disk full");
    }
}
