//! Actor domain model — represents a person accountable for operations.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorId(pub Uuid);

impl ActorId {
    /// Create a new random actor ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ActorId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for ActorId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Local actor authorization roles in hierarchical order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Read-only access: view, search, inspect
    Viewer = 0,
    /// Transactional access: receive, issue, transfer
    #[default]
    Operator = 1,
    /// Management access: physical adjustment, catalog & bin maintenance
    Manager = 2,
    /// Administrative access: user management, PINs, security audit
    Admin = 3,
}

impl Role {
    /// Can view, search, and inspect catalog, inventory, and locations.
    pub fn can_view(&self) -> bool {
        true
    }

    /// Can execute standard inventory transactions (receive, issue, transfer).
    pub fn can_transact(&self) -> bool {
        *self >= Role::Operator
    }

    /// Can perform physical inventory adjustments (count overrides) and catalog modifications.
    pub fn can_adjust(&self) -> bool {
        *self >= Role::Manager
    }

    /// Can manage users, credentials, roles, and administrative settings.
    pub fn can_admin(&self) -> bool {
        *self >= Role::Admin
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Viewer => write!(f, "viewer"),
            Role::Operator => write!(f, "operator"),
            Role::Manager => write!(f, "manager"),
            Role::Admin => write!(f, "admin"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "viewer" => Ok(Role::Viewer),
            "operator" => Ok(Role::Operator),
            "manager" => Ok(Role::Manager),
            "admin" | "administrator" => Ok(Role::Admin),
            other => Err(crate::error::CoreError::InvalidInput(format!(
                "invalid role '{}', expected viewer, operator, manager, or admin",
                other
            ))),
        }
    }
}

/// A registered actor in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub id: ActorId,
    pub display_name: String,
    pub os_username: Option<String>,
    pub role: Role,
    pub active: bool,
    pub created_at: time::OffsetDateTime,
}

/// Data required to create a new actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewActor {
    pub display_name: String,
    pub os_username: Option<String>,
    pub role: Role,
}

/// Admin-managed account changes. PIN values must never be included in audit details.
#[derive(Clone, Default)]
pub struct ActorUpdate {
    pub display_name: Option<String>,
    pub role: Option<Role>,
    pub active: Option<bool>,
    pub pin: Option<String>,
}

impl NewActor {
    /// Create a new actor with default role (Operator).
    pub fn new(display_name: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            os_username: None,
            role: Role::Operator,
        }
    }

    /// Set the role of the new actor.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Set the OS username of the new actor.
    pub fn with_os_username(mut self, os_user: impl Into<String>) -> Self {
        self.os_username = Some(os_user.into());
        self
    }

    /// Validate the new actor data.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        if self.display_name.trim().is_empty() {
            return Err(crate::error::CoreError::InvalidInput(
                "actor display_name must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_id_round_trip() {
        let id = ActorId::new();
        let s = id.to_string();
        let parsed: ActorId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn role_hierarchy_and_permissions() {
        assert!(Role::Viewer.can_view());
        assert!(!Role::Viewer.can_transact());
        assert!(!Role::Viewer.can_adjust());
        assert!(!Role::Viewer.can_admin());

        assert!(Role::Operator.can_view());
        assert!(Role::Operator.can_transact());
        assert!(!Role::Operator.can_adjust());
        assert!(!Role::Operator.can_admin());

        assert!(Role::Manager.can_view());
        assert!(Role::Manager.can_transact());
        assert!(Role::Manager.can_adjust());
        assert!(!Role::Manager.can_admin());

        assert!(Role::Admin.can_view());
        assert!(Role::Admin.can_transact());
        assert!(Role::Admin.can_adjust());
        assert!(Role::Admin.can_admin());
    }

    #[test]
    fn role_parsing_and_display() {
        let r: Role = "viewer".parse().unwrap();
        assert_eq!(r, Role::Viewer);
        assert_eq!(r.to_string(), "viewer");

        let r: Role = "administrator".parse().unwrap();
        assert_eq!(r, Role::Admin);

        assert!("invalid".parse::<Role>().is_err());
    }

    #[test]
    fn new_actor_rejects_empty_name() {
        let actor = NewActor::new("   ");
        assert!(actor.validate().is_err());
    }

    #[test]
    fn new_actor_accepts_valid_name() {
        let actor = NewActor::new("Alice")
            .with_os_username("alice")
            .with_role(Role::Manager);
        assert!(actor.validate().is_ok());
        assert_eq!(actor.role, Role::Manager);
    }

    #[test]
    fn actor_serde_round_trip() {
        let actor = Actor {
            id: ActorId::new(),
            display_name: "Test".to_string(),
            os_username: None,
            role: Role::Admin,
            active: true,
            created_at: time::OffsetDateTime::now_utc(),
        };
        let json = serde_json::to_string(&actor).unwrap();
        let back: Actor = serde_json::from_str(&json).unwrap();
        assert_eq!(actor.id, back.id);
        assert_eq!(actor.display_name, back.display_name);
        assert_eq!(actor.role, back.role);
    }
}
