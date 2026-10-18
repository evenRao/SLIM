//! Authentication, session, and audit domain models.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::model::actor::{Actor, ActorId};

/// Unique identifier for an authentication session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for SessionId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Active authentication session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub actor_id: ActorId,
    pub token: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

impl Session {
    /// Returns true if the session has passed its expiration time.
    pub fn is_expired(&self) -> bool {
        OffsetDateTime::now_utc() >= self.expires_at
    }
}

/// An authenticated session with the resolved actor details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedSession {
    pub session: Session,
    pub actor: Actor,
}

/// Unique identifier for an audit log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuditEventId(pub Uuid);

impl AuditEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AuditEventId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AuditEventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AuditEventId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Administrative and security audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: AuditEventId,
    pub actor_id: Option<ActorId>,
    pub action: String,
    pub details: Option<String>,
    pub created_at: OffsetDateTime,
}

/// Data required to record a new audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAuditEvent {
    pub actor_id: Option<ActorId>,
    pub action: String,
    pub details: Option<String>,
}

impl NewAuditEvent {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            actor_id: None,
            action: action.into(),
            details: None,
        }
    }

    pub fn with_actor(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

/// Query filter for audit logs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditFilter {
    pub actor_id: Option<ActorId>,
    pub action: Option<String>,
    pub limit: Option<usize>,
}

/// Hashes a user PIN using salted SHA-256.
pub fn hash_pin(pin: &str) -> String {
    use sha2::{Digest, Sha256};
    let salt = Uuid::new_v4().simple().to_string();
    let mut hasher = Sha256::new();
    hasher.update(format!("{}:{}", salt, pin).as_bytes());
    let hash_hex = format!("{:x}", hasher.finalize());
    format!("{}${}", salt, hash_hex)
}

/// Verifies a plain PIN against a stored salted hash.
pub fn verify_pin(pin: &str, stored_hash: &str) -> bool {
    use sha2::{Digest, Sha256};
    let parts: Vec<&str> = stored_hash.split('$').collect();
    if parts.len() != 2 {
        return false;
    }
    let salt = parts[0];
    let expected_hash = parts[1];

    let mut hasher = Sha256::new();
    hasher.update(format!("{}:{}", salt, pin).as_bytes());
    let computed_hex = format!("{:x}", hasher.finalize());
    computed_hex == expected_hash
}

/// Generates a cryptographically random session token string.
pub fn generate_session_token() -> String {
    format!("sess_{}", Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    #[test]
    fn pin_hashing_and_verification() {
        let pin = "1234";
        let hashed = hash_pin(pin);
        assert!(hashed.contains('$'));
        assert!(verify_pin(pin, &hashed));
        assert!(!verify_pin("wrong", &hashed));
        assert!(!verify_pin("", &hashed));
        assert!(!verify_pin(pin, "corrupted_hash"));
    }

    #[test]
    fn session_expiration() {
        let now = OffsetDateTime::now_utc();
        let session = Session {
            id: SessionId::new(),
            actor_id: ActorId::new(),
            token: generate_session_token(),
            created_at: now - Duration::hours(2),
            expires_at: now - Duration::hours(1),
        };
        assert!(session.is_expired());

        let active_session = Session {
            id: SessionId::new(),
            actor_id: ActorId::new(),
            token: generate_session_token(),
            created_at: now,
            expires_at: now + Duration::hours(8),
        };
        assert!(!active_session.is_expired());
    }

    #[test]
    fn audit_event_builder() {
        let actor_id = ActorId::new();
        let event = NewAuditEvent::new("user.login")
            .with_actor(actor_id)
            .with_details("PIN authentication successful");

        assert_eq!(event.action, "user.login");
        assert_eq!(event.actor_id, Some(actor_id));
        assert_eq!(
            event.details,
            Some("PIN authentication successful".to_string())
        );
    }
}
