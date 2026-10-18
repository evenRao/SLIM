//! Service trait definitions (ports) for SLIM.
//!
//! These traits define the interfaces that storage implementations must
//! provide. `slim-core` declares them; `slim-db` implements them.

use crate::error::CoreResult;
use crate::model::actor::*;
use crate::model::auth::*;
use crate::model::automation::*;
use crate::model::device::*;
use crate::model::gridfinity::*;

use crate::model::identifier::*;
use crate::model::item::*;
use crate::model::led::*;
use crate::model::location::*;
use crate::model::operation::*;
use crate::model::stock::*;

// ── Repository traits (data access ports) ──

/// Repository for managing catalog items.
pub trait ItemRepository {
    fn create(&self, item: &NewItem) -> CoreResult<Item>;
    fn get_by_id(&self, id: &ItemId) -> CoreResult<Option<Item>>;
    fn get_by_sku(&self, sku: &str) -> CoreResult<Option<Item>>;
    fn list(&self, filter: &ItemFilter) -> CoreResult<Vec<Item>>;
    fn update(&self, id: &ItemId, update: &ItemUpdate) -> CoreResult<Item>;
    fn archive(&self, id: &ItemId) -> CoreResult<()>;
}

/// Repository for managing hierarchical locations.
pub trait LocationRepository {
    fn create(&self, location: &NewLocation) -> CoreResult<Location>;
    fn get_by_id(&self, id: &LocationId) -> CoreResult<Option<Location>>;
    fn find_by_name(
        &self,
        name: &str,
        parent_id: Option<&LocationId>,
    ) -> CoreResult<Option<Location>>;
    fn list_children(&self, parent_id: Option<&LocationId>) -> CoreResult<Vec<Location>>;
    /// Returns the path from root to the given location.
    fn get_path(&self, id: &LocationId) -> CoreResult<Vec<Location>>;
}

/// Repository for managing actor identities.
pub trait ActorRepository {
    fn create(&self, actor: &NewActor) -> CoreResult<Actor>;
    fn get_by_id(&self, id: &ActorId) -> CoreResult<Option<Actor>>;
    fn get_by_name(&self, name: &str) -> CoreResult<Option<Actor>>;
    fn get_by_os_username(&self, username: &str) -> CoreResult<Option<Actor>>;
    fn list(&self) -> CoreResult<Vec<Actor>>;
}

/// Repository for managing identifiers (barcodes, QR codes, etc.).
pub trait IdentifierRepository {
    fn create(&self, identifier: &NewIdentifier) -> CoreResult<Identifier>;
    fn resolve(&self, kind: &IdentifierKind, value: &str) -> CoreResult<Option<IdentifierTarget>>;
    fn list_for_item(&self, item_id: &ItemId) -> CoreResult<Vec<Identifier>>;
    fn list_for_location(&self, location_id: &LocationId) -> CoreResult<Vec<Identifier>>;
}

// ── Business operation traits ──

/// Service for executing inventory operations (receive, issue, transfer, adjust).
pub trait InventoryService {
    fn receive(&self, cmd: ReceiveStock) -> CoreResult<OperationReceipt>;
    fn issue(&self, cmd: IssueStock) -> CoreResult<OperationReceipt>;
    fn transfer(&self, cmd: TransferStock) -> CoreResult<OperationReceipt>;
    fn adjust(&self, cmd: AdjustStock) -> CoreResult<OperationReceipt>;

    fn get_balance(
        &self,
        item_id: &ItemId,
        location_id: &LocationId,
    ) -> CoreResult<Option<StockBalance>>;
    fn get_item_balances(&self, item_id: &ItemId) -> CoreResult<Vec<StockBalance>>;
    fn get_location_balances(&self, location_id: &LocationId) -> CoreResult<Vec<StockBalance>>;
    fn get_low_stock(&self) -> CoreResult<Vec<LowStockAlert>>;
}

/// Service for querying operation history.
pub trait HistoryQuery {
    fn by_item(&self, item_id: &ItemId, limit: usize) -> CoreResult<Vec<OperationSummary>>;
    fn by_location(
        &self,
        location_id: &LocationId,
        limit: usize,
    ) -> CoreResult<Vec<OperationSummary>>;
    fn by_actor(&self, actor_id: &ActorId, limit: usize) -> CoreResult<Vec<OperationSummary>>;
    fn by_operation(&self, op_id: &OperationId) -> CoreResult<Option<OperationDetail>>;
}

/// Service for reconciling cached balances against the movement ledger.
pub trait ReconciliationService {
    fn check(&self) -> CoreResult<ReconciliationReport>;
    fn repair(&self, actor_id: &ActorId, reason: &str) -> CoreResult<ReconciliationReport>;
}

/// Service for managing stock thresholds.
pub trait ThresholdRepository {
    fn set(&self, threshold: &StockThreshold) -> CoreResult<()>;
    fn get_for_item(&self, item_id: &ItemId) -> CoreResult<Vec<StockThreshold>>;
    fn remove(&self, item_id: &ItemId, location_id: Option<&LocationId>) -> CoreResult<()>;
}

/// Repository for managing Gridfinity baseplates, bins, and spatial placements.
pub trait GridRepository {
    fn create_base(&self, base: &NewGridBase) -> CoreResult<GridBase>;
    fn get_base_by_id(&self, id: &GridBaseId) -> CoreResult<Option<GridBase>>;
    fn get_base_by_code(&self, code: &str) -> CoreResult<Option<GridBase>>;
    fn list_bases(&self, location_id: Option<&LocationId>) -> CoreResult<Vec<GridBase>>;

    fn create_bin(
        &self,
        bin: &NewGridfinityBin,
        location_id: &LocationId,
    ) -> CoreResult<GridfinityBin>;
    fn get_bin_by_id(&self, id: &GridfinityBinId) -> CoreResult<Option<GridfinityBin>>;
    fn get_bin_by_code(&self, code: &str) -> CoreResult<Option<GridfinityBin>>;
    fn get_bin_by_location_id(&self, location_id: &LocationId)
    -> CoreResult<Option<GridfinityBin>>;
    fn list_bins(&self) -> CoreResult<Vec<GridfinityBin>>;

    fn place_bin(&self, cmd: &PlaceBinCmd) -> CoreResult<BinPlacement>;
    fn remove_bin(&self, bin_id: &GridfinityBinId) -> CoreResult<()>;
    fn get_active_placements(&self, base_id: &GridBaseId) -> CoreResult<Vec<ActivePlacementInfo>>;
    fn get_active_placement_for_bin(
        &self,
        bin_id: &GridfinityBinId,
    ) -> CoreResult<Option<ActivePlacementInfo>>;
    fn render_grid_map(&self, base_id: &GridBaseId) -> CoreResult<GridMap>;
}

/// Port trait for resolving scanned barcode or QR strings to domain entities.
pub trait ScanResolver {
    /// Resolves an arbitrary scanned code, URI, barcode, or alias to an Item, Bin, Location, or Unregistered.
    fn resolve_scan(&self, code: &str) -> CoreResult<crate::model::scan::ScanTarget>;
}

/// Service for managing authentication credentials, sessions, and roles.
pub trait AuthService {
    /// Authenticate an actor by display name and PIN. Returns a new session.
    fn login_with_pin(&self, name: &str, pin: &str) -> CoreResult<Session>;

    /// Authenticate an actor by scanned badge token. Returns a new session.
    fn login_with_badge(&self, badge_token: &str) -> CoreResult<Session>;

    /// Validate an active session token and return the associated actor.
    fn validate_session(&self, token: &str) -> CoreResult<AuthenticatedSession>;

    /// Terminate (delete) an active session.
    fn terminate_session(&self, token: &str) -> CoreResult<()>;

    /// Set or update an actor's PIN.
    fn set_pin(&self, actor_id: &ActorId, pin: &str) -> CoreResult<()>;

    /// Set or update an actor's badge token (or clear it if None).
    fn set_badge_token(&self, actor_id: &ActorId, badge: Option<&str>) -> CoreResult<()>;

    /// Set or update an actor's authorization role.
    fn set_role(&self, actor_id: &ActorId, role: Role) -> CoreResult<()>;
}

/// Repository for persisting and querying administrative and security audit events.
pub trait AuditRepository {
    /// Record a new audit event in the system log.
    fn record(&self, event: &NewAuditEvent) -> CoreResult<AuditEvent>;

    /// Query recorded audit events matching the specified filter.
    fn list(&self, filter: &AuditFilter) -> CoreResult<Vec<AuditEvent>>;
}

/// Repository for managing committed events in the durable event outbox.
pub trait OutboxRepository {
    /// Fetch up to `limit` pending outbox events ordered by creation timestamp.
    fn fetch_pending(&self, limit: usize) -> CoreResult<Vec<OutboxEvent>>;

    /// Mark an outbox event as successfully delivered.
    fn mark_delivered(&self, id: &OutboxEventId) -> CoreResult<()>;

    /// Record a failed attempt for an outbox event with error details.
    fn record_attempt_error(&self, id: &OutboxEventId, error_msg: &str) -> CoreResult<()>;

    /// Return the count of undelivered (pending) outbox events.
    fn pending_count(&self) -> CoreResult<usize>;

    /// Get an outbox event by its ID.
    fn get_by_id(&self, id: &OutboxEventId) -> CoreResult<Option<OutboxEvent>>;
}

/// Repository for managing automation rules and recording execution runs.
pub trait AutomationRepository {
    /// Persist or update an automation rule in the database.
    fn save_rule(&self, rule: &AutomationRule) -> CoreResult<()>;

    /// Retrieve an automation rule by ID.
    fn get_rule(&self, id: &str) -> CoreResult<Option<AutomationRule>>;

    /// List all stored automation rules.
    fn list_rules(&self) -> CoreResult<Vec<AutomationRule>>;

    /// Delete an automation rule by ID.
    fn delete_rule(&self, id: &str) -> CoreResult<()>;

    /// Record an automation execution run in history.
    fn record_run(&self, run: &AutomationRun) -> CoreResult<()>;

    /// Query execution run history ordered by most recent first.
    fn list_runs(&self, limit: usize) -> CoreResult<Vec<AutomationRun>>;
}

/// Repository for managing hardware devices and sensor bindings.
pub trait DeviceRepository {
    /// Register a new device in the system registry.
    fn register_device(&self, device: &NewDevice) -> CoreResult<RegisteredDevice>;

    /// Get a registered device by its ID.
    fn get_device(&self, id: &str) -> CoreResult<Option<RegisteredDevice>>;

    /// List all registered devices.
    fn list_devices(&self) -> CoreResult<Vec<RegisteredDevice>>;

    /// Delete/unregister a device from the system.
    fn delete_device(&self, id: &str) -> CoreResult<()>;

    /// Update a device's active status.
    fn update_device_status(&self, id: &str, active: bool) -> CoreResult<()>;
}

/// Repository for persisting and querying raw physical device observations.
pub trait ObservationRepository {
    /// Record a new raw observation event from a sensor.
    fn record_observation(
        &self,
        device_id: &str,
        observation: &DeviceObservation,
    ) -> CoreResult<StoredObservation>;

    /// Query recorded observations.
    fn list_observations(
        &self,
        pending_only: bool,
        device_id: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<StoredObservation>>;

    /// Get an observation by its unique ID.
    fn get_observation(&self, id: &ObservationId) -> CoreResult<Option<StoredObservation>>;

    /// Confirm an observation with operator attribution and optional resulting stock operation.
    fn confirm_observation(
        &self,
        id: &ObservationId,
        actor_id: &ActorId,
        operation_id: Option<&OperationId>,
    ) -> CoreResult<()>;

    /// Reject an observation with operator attribution.
    fn reject_observation(&self, id: &ObservationId, actor_id: &ActorId) -> CoreResult<()>;
}

/// Repository for managing physical dock LED states, presence sensors, and priority arbitration.
pub trait LedRepository {
    /// Set or update the logical LED state for a specific dock.
    fn set_dock_led(&self, state: &DockLedState) -> CoreResult<()>;

    /// Retrieve the current LED state for a dock.
    fn get_dock_led(&self, dock_id: &str) -> CoreResult<Option<DockLedState>>;

    /// List active LED states across baseplates (optionally filtered by base ID).
    fn list_dock_leds(&self, base_id: Option<&GridBaseId>) -> CoreResult<Vec<DockLedState>>;

    /// Clear/reset the LED state for a dock.
    fn clear_dock_led(&self, dock_id: &str) -> CoreResult<()>;

    /// Clear all expired LED states (e.g. timed-out locate pulses).
    fn clear_expired_leds(&self, now: time::OffsetDateTime) -> CoreResult<usize>;

    /// Record/update the physical occupancy state of a dock.
    fn set_dock_presence(&self, presence: &DockPresenceState) -> CoreResult<()>;

    /// Get the physical occupancy state of a dock.
    fn get_dock_presence(&self, dock_id: &str) -> CoreResult<Option<DockPresenceState>>;

    /// List physical occupancy states for a baseplate.
    fn list_dock_presence(&self, base_id: &GridBaseId) -> CoreResult<Vec<DockPresenceState>>;
}
