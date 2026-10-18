//! Automation domain models for SLIM.
//!
//! Provides models for outbox events, automation rules, trigger conditions,
//! actions, and execution runs.

use std::fmt;
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};

/// Unique identifier for an outbox event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OutboxEventId(pub Uuid);

impl OutboxEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for OutboxEventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OutboxEventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for OutboxEventId {
    type Err = CoreError;

    fn from_str(s: &str) -> CoreResult<Self> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|e| CoreError::InvalidInput(format!("invalid outbox event ID '{}': {}", s, e)))
    }
}

/// Unique identifier for an automation run execution record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AutomationRunId(pub Uuid);

impl AutomationRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for AutomationRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AutomationRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AutomationRunId {
    type Err = CoreError;

    fn from_str(s: &str) -> CoreResult<Self> {
        Uuid::parse_str(s).map(Self).map_err(|e| {
            CoreError::InvalidInput(format!("invalid automation run ID '{}': {}", s, e))
        })
    }
}

/// A durable committed domain event in the outbox pending processing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxEvent {
    pub id: OutboxEventId,
    pub event_type: String,
    pub aggregate_id: String,
    pub payload: String,
    pub created_at: OffsetDateTime,
    pub delivered_at: Option<OffsetDateTime>,
    pub attempt_count: u32,
    pub last_error: Option<String>,
}

/// Action to execute when an automation rule triggers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    Notify {
        message: String,
    },
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Webhook {
        url: String,
        #[serde(default)]
        payload: Option<String>,
    },
}

impl RuleAction {
    pub fn action_type_str(&self) -> &'static str {
        match self {
            RuleAction::Notify { .. } => "notify",
            RuleAction::Command { .. } => "command",
            RuleAction::Webhook { .. } => "webhook",
        }
    }
}

/// Conditions that must be satisfied for a rule to fire.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuleCondition {
    #[serde(default)]
    pub quantity_below: Option<u32>,
    #[serde(default)]
    pub quantity_above: Option<u32>,
    #[serde(default)]
    pub quantity_equals: Option<u32>,
    #[serde(default)]
    pub quantity_delta_below: Option<i64>,
    #[serde(default)]
    pub quantity_delta_above: Option<i64>,
}

fn default_true() -> bool {
    true
}

/// An automation rule configuration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutomationRule {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub event: String,
    #[serde(default)]
    pub item_sku: Option<String>,
    #[serde(default)]
    pub location_name: Option<String>,
    #[serde(default)]
    pub condition: Option<RuleCondition>,
    pub action: RuleAction,
}

impl AutomationRule {
    /// Normalizes event names for flexible matching.
    /// E.g. "stock.below_threshold", "stock.low", and "StockBelowThreshold" normalize identically.
    pub fn normalize_event_name(event: &str) -> String {
        let clean = event
            .trim()
            .replace(['.', '_', '-'], "")
            .to_ascii_lowercase();

        if clean == "stocklow" || clean == "lowstock" {
            "stockbelowthreshold".to_string()
        } else {
            clean
        }
    }

    /// Evaluates whether this rule matches the given event details and payload.
    pub fn matches_event(
        &self,
        event_type: &str,
        item_sku: Option<&str>,
        location_name: Option<&str>,
        payload_json: &serde_json::Value,
    ) -> bool {
        if !self.enabled {
            return false;
        }

        // 1. Match event type
        let rule_event_norm = Self::normalize_event_name(&self.event);
        let actual_event_norm = Self::normalize_event_name(event_type);

        if rule_event_norm != "*"
            && rule_event_norm != "any"
            && rule_event_norm != actual_event_norm
        {
            return false;
        }

        // 2. Match item SKU if specified in rule
        if let Some(ref required_sku) = self.item_sku {
            let actual_sku = item_sku.or_else(|| {
                payload_json
                    .get("item_sku")
                    .or_else(|| payload_json.get("sku"))
                    .and_then(|v| v.as_str())
            });

            match actual_sku {
                Some(sku) if sku.eq_ignore_ascii_case(required_sku) => {}
                _ => return false,
            }
        }

        // 3. Match location name if specified in rule
        if let Some(ref required_loc) = self.location_name {
            let actual_loc = location_name.or_else(|| {
                payload_json
                    .get("location_name")
                    .or_else(|| payload_json.get("location"))
                    .and_then(|v| v.as_str())
            });

            match actual_loc {
                Some(loc) if loc.eq_ignore_ascii_case(required_loc) => {}
                _ => return false,
            }
        }

        // 4. Match numerical conditions if present
        if let Some(ref cond) = self.condition {
            // Extract relevant quantity: "on_hand" or "resulting_qty" or "quantity"
            let on_hand_opt = payload_json
                .get("on_hand")
                .or_else(|| payload_json.get("resulting_qty"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            if let Some(below) = cond.quantity_below {
                match on_hand_opt {
                    Some(qty) if qty < below => {}
                    _ => return false,
                }
            }

            if let Some(above) = cond.quantity_above {
                match on_hand_opt {
                    Some(qty) if qty > above => {}
                    _ => return false,
                }
            }

            if let Some(equals) = cond.quantity_equals {
                match on_hand_opt {
                    Some(qty) if qty == equals => {}
                    _ => return false,
                }
            }

            // Extract delta: "delta_qty" or "quantity"
            let delta_opt = payload_json
                .get("delta_qty")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    payload_json
                        .get("quantity")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as i64)
                });

            if let Some(delta_below) = cond.quantity_delta_below {
                match delta_opt {
                    Some(delta) if delta < delta_below => {}
                    _ => return false,
                }
            }

            if let Some(delta_above) = cond.quantity_delta_above {
                match delta_opt {
                    Some(delta) if delta > delta_above => {}
                    _ => return false,
                }
            }
        }

        true
    }
}

/// Execution outcome status of an automation action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    Success,
    Failed,
    Skipped,
}

// suggestion: see derive_more or similar crate to automatically
// generate Display and FromStr impl. using proc. macro

impl fmt::Display for AutomationRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutomationRunStatus::Success => write!(f, "success"),
            AutomationRunStatus::Failed => write!(f, "failed"),
            AutomationRunStatus::Skipped => write!(f, "skipped"),
        }
    }
}

impl FromStr for AutomationRunStatus {
    type Err = CoreError;

    fn from_str(s: &str) -> CoreResult<Self> {
        match s.to_ascii_lowercase().as_str() {
            "success" => Ok(AutomationRunStatus::Success),
            "failed" => Ok(AutomationRunStatus::Failed),
            "skipped" => Ok(AutomationRunStatus::Skipped),
            _ => Err(CoreError::InvalidInput(format!(
                "invalid automation run status '{}', expected 'success', 'failed', or 'skipped'",
                s
            ))),
        }
    }
}

/// Record of an executed automation rule action.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutomationRun {
    pub id: AutomationRunId,
    pub rule_id: String,
    pub outbox_id: Option<OutboxEventId>,
    pub event_type: String,
    pub action_type: String,
    pub status: AutomationRunStatus,
    pub output: Option<String>,
    pub error_message: Option<String>,
    pub executed_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_matching_event_normalization() {
        let rule = AutomationRule {
            id: "low-m3".to_string(),
            name: Some("Low M3 Alert".to_string()),
            enabled: true,
            event: "stock.below_threshold".to_string(),
            item_sku: Some("M3X10".to_string()),
            location_name: None,
            condition: Some(RuleCondition {
                quantity_below: Some(50),
                ..Default::default()
            }),
            action: RuleAction::Notify {
                message: "Low stock alert".to_string(),
            },
        };

        // Matches when event is StockBelowThreshold and on_hand is 25
        let payload = serde_json::json!({
            "item_id": "abc",
            "on_hand": 25,
            "min_qty": 50,
        });
        assert!(rule.matches_event("StockBelowThreshold", Some("M3X10"), None, &payload));

        // Matches with "stock.low" alias
        let mut rule2 = rule.clone();
        rule2.event = "stock.low".to_string();
        assert!(rule2.matches_event("StockBelowThreshold", Some("M3X10"), None, &payload));

        // Does not match when on_hand >= quantity_below
        let payload_high = serde_json::json!({
            "item_id": "abc",
            "on_hand": 60,
        });
        assert!(!rule.matches_event("StockBelowThreshold", Some("M3X10"), None, &payload_high));

        // Does not match different SKU
        assert!(!rule.matches_event("StockBelowThreshold", Some("M4X20"), None, &payload));

        // Does not match different event
        assert!(!rule.matches_event("StockReceived", Some("M3X10"), None, &payload));
    }

    #[test]
    fn test_wildcard_rule_matches_any_event() {
        let rule = AutomationRule {
            id: "audit-all".to_string(),
            name: None,
            enabled: true,
            event: "*".to_string(),
            item_sku: None,
            location_name: None,
            condition: None,
            action: RuleAction::Notify {
                message: "Activity logged".to_string(),
            },
        };

        let payload = serde_json::json!({});
        assert!(rule.matches_event("StockReceived", None, None, &payload));
        assert!(rule.matches_event("StockIssued", None, None, &payload));
    }

    #[test]
    fn test_disabled_rule_never_matches() {
        let rule = AutomationRule {
            id: "disabled-rule".to_string(),
            name: None,
            enabled: false,
            event: "*".to_string(),
            item_sku: None,
            location_name: None,
            condition: None,
            action: RuleAction::Notify {
                message: "Never fires".to_string(),
            },
        };

        let payload = serde_json::json!({});
        assert!(!rule.matches_event("StockReceived", None, None, &payload));
    }

    #[test]
    fn test_serde_round_trip() {
        let rule = AutomationRule {
            id: "rule-1".to_string(),
            name: Some("Rule One".to_string()),
            enabled: true,
            event: "stock.received".to_string(),
            item_sku: Some("M3X10".to_string()),
            location_name: Some("GF-001".to_string()),
            condition: Some(RuleCondition {
                quantity_above: Some(10),
                ..Default::default()
            }),
            action: RuleAction::Command {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
            },
        };

        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: AutomationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }
}
