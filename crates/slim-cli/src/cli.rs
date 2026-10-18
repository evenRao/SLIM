//! Clap command-line argument definitions for SLIM.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "slim", version, about = "System for Local Inventory Management", long_about = None)]
pub struct Cli {
    /// Path to the SQLite database file
    #[arg(short, long, global = true)]
    pub db: Option<PathBuf>,

    /// Path to a custom config.toml file
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Override the actor identity for mutating operations
    #[arg(long, global = true)]
    pub actor: Option<String>,

    /// Output results in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Open the local inventory application in your browser
    App(AppArgs),
    /// Launch the interactive Terminal User Interface (Ratatui TUI, default)
    Tui,

    /// Initialize a new SLIM inventory database and run migrations
    Init(InitArgs),

    /// Database operations (status, reconcile, backup)
    Db(DbArgs),

    /// Manage actor identities
    Actor(ActorArgs),

    /// Manage catalog items
    Item(ItemArgs),

    /// Manage storage locations
    Location(LocationArgs),
    /// Manage storage, bases, bins, rules, devices and barcode links
    Catalog(CatalogArgs),

    /// Manage identifiers (barcodes, QR codes, tags)
    Identifier(IdentifierArgs),

    /// Inventory stock operations (receive, issue, transfer, adjust, balance, low)
    Stock(StockArgs),

    /// Query operation and stock movement history
    History(HistoryArgs),

    /// Search items and locations
    Find(FindArgs),

    /// Manage Gridfinity baseplates
    Grid(GridArgs),

    /// Manage Gridfinity bins and placements
    Bin(BinArgs),

    /// Generate and export printable QR codes and labels
    Label(LabelArgs),

    /// Barcode & QR code scanner mode and resolver
    Scan(ScanArgs),

    /// Manage authentication sessions (login, badge, whoami, logout)
    Session(SessionArgs),

    /// Administrative and security audit log
    Audit(AuditArgs),

    /// Automation and event hook engine (worker, rules, history, pending)
    Automation(AutomationArgs),

    /// Alias for automation hooks
    #[command(alias = "hooks")]
    Hook(AutomationArgs),

    /// Manage hardware devices and sensors
    Device(DeviceArgs),

    /// Review and confirm raw physical hardware observations
    Observation(ObservationArgs),

    /// Manage Gridfinity dock LEDs and physical location finding
    Led(LedArgs),
}

// ── Subcommand Args ──

#[derive(Debug, Args, Default)]
pub struct AppArgs {
    /// Do not automatically open the browser
    #[arg(long)]
    pub no_open: bool,
    /// Local port (0 chooses an available port)
    #[arg(long, default_value_t = 0)]
    pub port: u16,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Database path (if omitted, uses default or config path)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommands,
}

#[derive(Debug, Subcommand)]
pub enum DbCommands {
    /// Check migration status and connection parameters
    Status,

    /// Reconcile cached balances against the immutable ledger
    Reconcile(ReconcileArgs),

    /// Create an online backup of the SQLite database
    Backup(BackupArgs),
}

#[derive(Debug, Args)]
pub struct ReconcileArgs {
    /// Automatically repair cached balances from ledger sums
    #[arg(long)]
    pub repair: bool,

    /// Mandatory reason if repairing
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Destination path for the backup file
    pub destination: PathBuf,
}

#[derive(Debug, Args)]
pub struct ActorArgs {
    #[command(subcommand)]
    pub command: ActorCommands,
}

#[derive(Debug, Subcommand)]
pub enum ActorCommands {
    /// Register a new actor
    Add {
        /// Display name of the actor
        name: String,

        /// Optional OS username to map to this actor
        #[arg(long)]
        os_user: Option<String>,

        /// Optional authorization role (viewer, operator, manager, admin; defaults to operator)
        #[arg(long)]
        role: Option<String>,
    },

    /// List all registered actors
    List,

    /// Update an actor's authorization role
    SetRole {
        /// Actor display name or OS username
        name: String,
        /// New role: viewer, operator, manager, admin
        role: String,
    },

    /// Set or update an actor's PIN
    SetPin {
        /// Actor display name or OS username
        name: String,
        /// PIN (at least 4 characters; prompts if omitted)
        #[arg(long)]
        pin: Option<String>,
    },

    /// Set, update, or clear an actor's badge token
    SetBadge {
        /// Actor display name or OS username
        name: String,
        /// Scanned badge token (omit or pass --clear to remove)
        token: Option<String>,
        /// Clear the actor's badge token
        #[arg(long)]
        clear: bool,
    },
    /// Rename a user (Admin only)
    Edit {
        name: String,
        #[arg(long)]
        new_name: String,
    },
    /// Deactivate a user while preserving history (Admin only)
    #[command(alias = "delete")]
    Remove { name: String },
    /// Reactivate a removed user (Admin only)
    Restore { name: String },
    /// Permanently delete a removed account (Admin only)
    Purge {
        name: String,
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Debug, Args)]
pub struct ItemArgs {
    #[command(subcommand)]
    pub command: ItemCommands,
}

#[derive(Debug, Subcommand)]
pub enum ItemCommands {
    /// Add a new catalog item
    Add {
        /// Unique Stock Keeping Unit (optional)
        #[arg(short, long)]
        sku: Option<String>,

        /// Item display name
        #[arg(short, long)]
        name: String,

        /// Base counting unit (default: 'each')
        #[arg(short, long, default_value = "each")]
        unit: String,

        /// Optional item description
        #[arg(long)]
        description: Option<String>,
    },

    /// List catalog items
    List {
        /// Include archived items
        #[arg(short, long)]
        all: bool,

        /// Filter items by name or SKU
        #[arg(short, long)]
        search: Option<String>,
    },

    /// Show detailed item information
    Show {
        /// SKU or name of the item
        item: String,
    },

    /// Archive an item (soft-delete)
    #[command(alias = "delete")]
    Archive {
        /// SKU or name of the item
        item: String,
    },
    /// Restore a deleted item (Admin only)
    Restore { item: String },
    /// Permanently erase a deleted item and its history (Admin only)
    Purge {
        /// Deleted item's UUID, SKU, or unique name
        item: String,
        /// Required confirmation: DELETE PERMANENTLY
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Debug, Args)]
pub struct LocationArgs {
    #[command(subcommand)]
    pub command: LocationCommands,
}

#[derive(Debug, Args)]
pub struct CatalogArgs {
    /// storage, base, bin, rule, device, or barcode
    pub kind: String,
    /// list, add, edit, delete, restore, or purge
    #[arg(default_value = "list")]
    pub action: String,
    #[arg(long)]
    pub id: Option<String>,
    /// Field assignment, e.g. --field name=Toolbox --field kind=custom
    #[arg(long = "field")]
    pub fields: Vec<String>,
    #[arg(long)]
    pub deleted: bool,
    /// DELETE, RESTORE, or DELETE PERMANENTLY
    #[arg(long, default_value = "")]
    pub confirm: String,
}

#[derive(Debug, Subcommand)]
pub enum LocationCommands {
    /// Add a new location
    Add {
        /// Location name
        #[arg(short, long)]
        name: String,

        /// Kind: room, cabinet, drawer, shelf, storage, bin, compartment
        #[arg(short, long)]
        kind: String,

        /// Name of the parent location
        #[arg(short, long)]
        parent: Option<String>,
    },

    /// List locations in hierarchy
    List,
}

#[derive(Debug, Args)]
pub struct IdentifierArgs {
    #[command(subcommand)]
    pub command: IdentifierCommands,
}

#[derive(Debug, Subcommand)]
pub enum IdentifierCommands {
    /// Associate an identifier (barcode, QR code) with an item or location
    Add {
        /// Kind: barcode, qr, nfc, rfid, manual
        #[arg(short, long)]
        kind: String,

        /// Identifier value (scanned code)
        #[arg(short, long)]
        value: String,

        /// Item SKU or name to associate with
        #[arg(short, long)]
        item: Option<String>,

        /// Location name to associate with
        #[arg(short, long)]
        location: Option<String>,
    },

    /// Resolve an identifier to its target item or location
    Resolve {
        /// Kind: barcode, qr, nfc, rfid, manual
        #[arg(short, long)]
        kind: String,

        /// Scanned code value
        #[arg(short, long)]
        value: String,
    },
}

#[derive(Debug, Args)]
pub struct StockArgs {
    #[command(subcommand)]
    pub command: StockCommands,
}

#[derive(Debug, Subcommand)]
pub enum StockCommands {
    /// Receive new stock into a location
    Receive {
        /// Item SKU or name
        item: String,

        /// Quantity to receive (positive integer)
        #[arg(short, long)]
        qty: u32,

        /// Destination location name
        #[arg(short, long)]
        to: String,

        /// Reason or reference for the receipt
        #[arg(short, long)]
        reason: Option<String>,

        /// Optional idempotency key
        #[arg(long)]
        idempotency_key: Option<String>,
    },

    /// Issue (consume/remove) stock from a location
    Issue {
        /// Item SKU or name
        item: String,

        /// Quantity to issue (positive integer)
        #[arg(short, long)]
        qty: u32,

        /// Source location name
        #[arg(short, long)]
        from: String,

        /// Reason or reference for consumption
        #[arg(short, long)]
        reason: Option<String>,

        /// Optional idempotency key
        #[arg(long)]
        idempotency_key: Option<String>,
    },

    /// Transfer stock from one location to another atomically
    Transfer {
        /// Item SKU or name
        item: String,

        /// Quantity to transfer (positive integer)
        #[arg(short, long)]
        qty: u32,

        /// Source location name
        #[arg(short, long)]
        from: String,

        /// Destination location name
        #[arg(short, long)]
        to: String,

        /// Reason or reference for the transfer
        #[arg(short, long)]
        reason: Option<String>,

        /// Optional idempotency key
        #[arg(long)]
        idempotency_key: Option<String>,
    },

    /// Adjust stock to a specific physical count
    Adjust {
        /// Item SKU or name
        item: String,

        /// Target absolute quantity (>= 0)
        #[arg(short, long)]
        set: u32,

        /// Location name where count was verified
        #[arg(short, long)]
        at: String,

        /// Mandatory reason for the adjustment
        #[arg(short, long)]
        reason: String,

        /// Optional idempotency key
        #[arg(long)]
        idempotency_key: Option<String>,
    },

    /// Show current balance(s) for an item across all locations
    Balance {
        /// Item SKU or name
        item: String,
    },

    /// List items currently below their minimum stock threshold
    Low,

    /// Set a minimum stock threshold for an item
    Threshold {
        /// Item SKU or name
        item: String,

        /// Minimum quantity threshold
        #[arg(short, long)]
        min: u32,

        /// Optional location name (if omitted, sets a global threshold)
        #[arg(short, long)]
        location: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// Filter history by item SKU or name
    #[arg(short, long)]
    pub item: Option<String>,

    /// Filter history by location name
    #[arg(short, long)]
    pub location: Option<String>,

    /// Show full details for a specific operation ID
    #[arg(long)]
    pub op: Option<String>,

    /// Maximum number of records to return
    #[arg(long, default_value = "20")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    /// Search query (matches SKU, name, or identifier)
    pub query: String,

    /// Illuminate dock LEDs for the found item or bin
    #[arg(long)]
    pub locate: bool,

    /// Duration in seconds for locate pulse (default: 30)
    #[arg(long)]
    pub ttl: Option<u64>,
}

#[derive(Debug, Args)]
pub struct GridArgs {
    #[command(subcommand)]
    pub command: GridCommands,
}

#[derive(Debug, Subcommand)]
pub enum GridCommands {
    /// Create a new Gridfinity baseplate in a location
    Create {
        /// Code for the baseplate, e.g. BASE-A
        #[arg(long)]
        code: String,

        /// Parent location name (e.g. drawer, shelf)
        #[arg(long)]
        parent: String,

        /// Width in grid units (columns, X)
        #[arg(long)]
        cols: u32,

        /// Depth in grid units (rows, Y)
        #[arg(long)]
        rows: u32,

        /// Pitch in millimeters (default: 42.0)
        #[arg(long)]
        pitch: Option<f64>,
    },

    /// List registered Gridfinity baseplates
    List,

    /// Show baseplate details and interactive 2D layout map
    Show {
        /// Baseplate code
        base: String,
    },
}

#[derive(Debug, Args)]
pub struct BinArgs {
    #[command(subcommand)]
    pub command: BinCommands,
}

#[derive(Debug, Subcommand)]
pub enum BinCommands {
    /// Create a new Gridfinity bin
    Create {
        /// Unique bin code, e.g. B-017
        #[arg(long)]
        code: String,

        /// Width in Gridfinity units (X dimension)
        #[arg(short, long)]
        width: u32,

        /// Depth in Gridfinity units (Y dimension)
        #[arg(long)]
        depth: u32,

        /// Height in Gridfinity 7mm units (Z dimension)
        #[arg(long)]
        height: u32,

        /// Number of internal compartments (default: 1)
        #[arg(long, default_value = "1")]
        compartments: u32,

        /// Optional parent location name
        #[arg(long)]
        parent: Option<String>,
    },

    /// List registered Gridfinity bins
    List,

    /// Show bin details, dimensions, placement, and contained stock
    Show {
        /// Bin code
        bin: String,
    },

    /// Place a bin onto a baseplate at grid coordinates (X, Y)
    Place {
        /// Bin code to place
        bin: String,

        /// Destination baseplate code
        #[arg(short, long)]
        grid: String,

        /// Column coordinate (X, 0-indexed)
        #[arg(short, long)]
        x: u32,

        /// Row coordinate (Y, 0-indexed)
        #[arg(short, long)]
        y: u32,

        /// Rotation in degrees: 0 or 90
        #[arg(short, long, default_value = "0")]
        rotation: String,
    },

    /// Remove a bin from its baseplate
    Remove {
        /// Bin code to remove
        bin: String,
    },
}

#[derive(Debug, Args)]
pub struct LabelArgs {
    #[command(subcommand)]
    pub command: LabelCommands,
}

#[derive(Debug, Subcommand)]
pub enum LabelCommands {
    /// Generate a raw QR code for arbitrary text, URI, SKU, or bin code
    Qr {
        /// Content payload to encode into QR
        payload: String,

        /// Output format as SVG instead of terminal ANSI
        #[arg(long)]
        svg: bool,

        /// Optional file path to save QR output to
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Generate a complete printable label with QR code for an item
    Item {
        /// SKU or name of the item
        sku: String,

        /// Optional file path to save SVG QR code to
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Generate a complete printable label with QR code for a Gridfinity bin
    Bin {
        /// Unique bin code
        bin: String,

        /// Optional file path to save SVG QR code to
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Generate a complete printable label with QR code for a storage location
    Location {
        /// Name of the location
        name: String,

        /// Optional file path to save SVG QR code to
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    #[command(subcommand)]
    pub command: Option<ScanCommands>,
}

#[derive(Debug, Subcommand)]
pub enum ScanCommands {
    /// Resolve an arbitrary scanned barcode, QR code, or SKU
    Resolve {
        /// Scanned barcode, QR text, or URI
        code: String,
    },
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommands,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommands {
    /// Log in with actor username and PIN
    Login {
        /// Actor display name or OS username
        name: String,

        /// Plaintext PIN (if omitted, prompts on stdin)
        #[arg(long)]
        pin: Option<String>,
    },

    /// Log in by scanning an NFC or barcode badge token
    Badge {
        /// Scanned badge token
        token: String,
    },

    /// Display currently authenticated session details
    Whoami,

    /// Log out and terminate the active session
    Logout,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[command(subcommand)]
    pub command: AuditCommands,
}

#[derive(Debug, Subcommand)]
pub enum AuditCommands {
    /// List administrative and security audit events
    List {
        /// Filter by actor name
        #[arg(long)]
        actor: Option<String>,

        /// Filter by action name substring
        #[arg(long)]
        action: Option<String>,

        /// Maximum number of audit events to display (default: 50)
        #[arg(long, default_value = "50")]
        limit: usize,
    },
}

#[derive(Debug, Args)]
pub struct AutomationArgs {
    #[command(subcommand)]
    pub command: AutomationCommands,
}

#[derive(Debug, Subcommand)]
pub enum AutomationCommands {
    /// Run the outbox automation worker to process pending events
    Worker(WorkerArgs),

    /// Manage automation rules
    Rules(RulesArgs),

    /// View automation execution history
    History(AutomationHistoryArgs),

    /// View pending events in the durable outbox
    Pending(PendingArgs),

    /// Test an automation rule against an event or dry run
    Test(TestRuleArgs),
}

#[derive(Debug, Args)]
pub struct WorkerArgs {
    /// Process pending events once and exit
    #[arg(long)]
    pub once: bool,

    /// Polling interval in seconds when running continuously
    #[arg(short, long, default_value = "5")]
    pub poll_interval: u64,

    /// Enable local command/script execution for allowed commands
    #[arg(long)]
    pub allow_commands: bool,

    /// Maximum events to process per batch
    #[arg(long, default_value = "50")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct RulesArgs {
    #[command(subcommand)]
    pub command: RulesCommands,
}

#[derive(Debug, Subcommand)]
pub enum RulesCommands {
    /// List all configured automation rules (from DB and config files)
    List,

    /// Add an automation rule to the database
    Add(Box<AddRuleArgs>),

    /// Remove an automation rule from the database
    Remove(RemoveRuleArgs),

    /// Import rules from a TOML file into the database
    Import(ImportRulesArgs),
}

#[derive(Debug, Args)]
pub struct AddRuleArgs {
    /// Unique identifier for the rule
    pub id: String,

    /// Human-readable rule name
    #[arg(short, long)]
    pub name: Option<String>,

    /// Event trigger (e.g. stock.below_threshold, stock.received, stock.issued)
    #[arg(short, long)]
    pub event: String,

    /// Filter by item SKU
    #[arg(long)]
    pub sku: Option<String>,

    /// Filter by location name
    #[arg(long)]
    pub location: Option<String>,

    /// Condition: fire when on-hand quantity is below threshold
    #[arg(long)]
    pub quantity_below: Option<u32>,

    /// Action type: notify, command, or webhook
    #[arg(long, default_value = "notify")]
    pub action_type: String,

    /// Notification message or webhook payload
    #[arg(short, long)]
    pub message: Option<String>,

    /// Command to execute (if action_type is command)
    #[arg(long)]
    pub command: Option<String>,

    /// Arguments for command (separated by space)
    #[arg(long, num_args = 1..)]
    pub args: Option<Vec<String>>,

    /// Webhook URL (if action_type is webhook)
    #[arg(long)]
    pub url: Option<String>,
}

#[derive(Debug, Args)]
pub struct RemoveRuleArgs {
    /// Rule identifier to remove
    pub id: String,
}

#[derive(Debug, Args)]
pub struct ImportRulesArgs {
    /// Path to TOML rules file
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct AutomationHistoryArgs {
    /// Maximum number of runs to display
    #[arg(short, long, default_value = "20")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct PendingArgs {
    /// Maximum number of pending outbox events to display
    #[arg(short, long, default_value = "20")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct TestRuleArgs {
    /// Rule ID to test
    pub rule_id: String,

    /// Simulated event type (default: rule's trigger event)
    #[arg(short, long)]
    pub event: Option<String>,

    /// Simulated item SKU
    #[arg(long)]
    pub sku: Option<String>,

    /// Simulated location name
    #[arg(long)]
    pub location: Option<String>,

    /// Simulated on-hand quantity
    #[arg(long)]
    pub qty: Option<u32>,

    /// Allow command execution during test
    #[arg(long)]
    pub allow_commands: bool,
}

#[derive(Debug, Args)]
pub struct DeviceArgs {
    #[command(subcommand)]
    pub command: DeviceCommands,
}

#[derive(Debug, Subcommand)]
pub enum DeviceCommands {
    /// List all registered hardware devices
    List,

    /// Register a new hardware device
    Add(Box<AddDeviceArgs>),

    /// Remove a registered hardware device
    Remove(RemoveDeviceArgs),

    /// Simulate / inject an observation from a device
    Simulate(Box<SimulateDeviceArgs>),

    /// Listen for streaming observations from a serial port or device feed
    Listen(ListenDeviceArgs),
}

#[derive(Debug, Args)]
pub struct AddDeviceArgs {
    /// Unique identifier for the device (e.g. scanner-01, ir-shelf-1)
    pub id: String,

    /// Descriptive name
    #[arg(short, long)]
    pub name: String,

    /// Device kind: serial_scanner, ir_breakbeam, ir_presence, rfid_nfc, mock_sensor
    #[arg(short, long)]
    pub kind: String,

    /// Serial port or connection address (e.g. COM3, /dev/ttyUSB0)
    #[arg(short, long)]
    pub port: Option<String>,

    /// Associated storage location name or SKU
    #[arg(short, long)]
    pub location: Option<String>,

    /// Serial baud rate (default: 9600)
    #[arg(long, default_value = "9600")]
    pub baud: u32,
}

#[derive(Debug, Args)]
pub struct RemoveDeviceArgs {
    /// Device ID to unregister
    pub id: String,
}

#[derive(Debug, Args)]
pub struct SimulateDeviceArgs {
    /// Device ID reporting the observation
    pub device_id: String,

    /// Event type: beam_interrupted, presence_changed, identifier_scanned, tag_detected
    #[arg(short, long)]
    pub event: String,

    /// Monitored compartment or zone name
    #[arg(short, long)]
    pub zone: Option<String>,

    /// Scanned barcode/QR identifier value
    #[arg(short, long)]
    pub value: Option<String>,

    /// Presence state (true for present, false for absent)
    #[arg(long)]
    pub present: Option<bool>,

    /// RFID/NFC tag UID
    #[arg(long)]
    pub uid: Option<String>,
}

#[derive(Debug, Args)]
pub struct ListenDeviceArgs {
    /// Registered device ID to listen on
    #[arg(long)]
    pub device: Option<String>,

    /// Serial port override (e.g. COM3, /dev/ttyUSB0)
    #[arg(short, long)]
    pub port: Option<String>,

    /// Baud rate for serial port (default: 9600)
    #[arg(long, default_value = "9600")]
    pub baud: u32,

    /// Receive one event and exit
    #[arg(long)]
    pub once: bool,

    /// Polling interval in milliseconds
    #[arg(long, default_value = "100")]
    pub poll_interval_ms: u64,
}

#[derive(Debug, Args)]
pub struct ObservationArgs {
    #[command(subcommand)]
    pub command: ObservationCommands,
}

#[derive(Debug, Subcommand)]
pub enum ObservationCommands {
    /// List hardware device observations
    List(ListObservationsArgs),

    /// Confirm a pending observation and attribute to an operator
    Confirm(Box<ConfirmObservationArgs>),

    /// Reject an invalid or accidental observation
    Reject(RejectObservationArgs),
}

#[derive(Debug, Args)]
pub struct ListObservationsArgs {
    /// Show only pending observations requiring operator review
    #[arg(long)]
    pub pending: bool,

    /// Filter by device ID
    #[arg(long)]
    pub device: Option<String>,

    /// Maximum number of observations to display (default: 30)
    #[arg(short, long, default_value = "30")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ConfirmObservationArgs {
    /// Observation ID to confirm
    pub id: String,

    /// Item SKU to transact (required if creating stock operation)
    #[arg(short, long)]
    pub sku: Option<String>,

    /// Quantity removed or added
    #[arg(short, long)]
    pub qty: Option<u32>,

    /// Operation kind: issue or receive (default: issue)
    #[arg(short, long, default_value = "issue")]
    pub kind: String,

    /// Location name or ID (defaults to device's mapped location)
    #[arg(short, long)]
    pub location: Option<String>,

    /// Reason or reference notes
    #[arg(short, long)]
    pub reason: Option<String>,
}

#[derive(Debug, Args)]
pub struct RejectObservationArgs {
    /// Observation ID to reject
    pub id: String,

    /// Rejection reason
    #[arg(short, long)]
    pub reason: Option<String>,
}

#[derive(Debug, Args)]
pub struct LedArgs {
    #[command(subcommand)]
    pub command: LedCommands,
}

#[derive(Debug, Subcommand)]
pub enum LedCommands {
    /// Show current LED indicator states across baseplates
    Status(LedStatusArgs),

    /// Illuminate dock LEDs for an item SKU or Gridfinity bin
    Locate(LedLocateArgs),

    /// Clear LED indicators (reset to priority evaluation or off)
    Clear(LedClearArgs),

    /// Simulate physical dock presence / bin removal
    SimulatePresence(LedSimulatePresenceArgs),

    /// Evaluate LED priority state machine and print serial protocol frames
    Evaluate(LedEvaluateArgs),
}

#[derive(Debug, Args)]
pub struct LedStatusArgs {
    /// Filter by baseplate code or ID
    #[arg(short, long)]
    pub base: Option<String>,
}

#[derive(Debug, Args)]
pub struct LedLocateArgs {
    /// Item SKU or Gridfinity bin code to locate
    pub target: String,

    /// Duration in seconds for locate pulse (default: 30)
    #[arg(short, long)]
    pub ttl: Option<u64>,

    /// Clear locate indication instead of setting it
    #[arg(long)]
    pub clear: bool,
}

#[derive(Debug, Args)]
pub struct LedClearArgs {
    /// Target dock identifier (e.g. BASE-A:3:1)
    #[arg(long)]
    pub dock: Option<String>,

    /// Target baseplate code or ID
    #[arg(short, long)]
    pub base: Option<String>,

    /// Clear all active LED states
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct LedSimulatePresenceArgs {
    /// Target dock identifier (e.g. BASE-A:3:1)
    pub dock_id: String,

    /// Occupancy state: true for present, false for removed/missing
    #[arg(long, action = clap::ArgAction::Set)]
    pub occupied: bool,

    /// Presence status text (present, missing, unknown)
    #[arg(long, default_value = "present")]
    pub status: String,
}

#[derive(Debug, Args)]
pub struct LedEvaluateArgs {
    /// Baseplate code or ID (if omitted, evaluates all bases)
    #[arg(short, long)]
    pub base: Option<String>,

    /// Output raw newline-delimited JSON serial protocol frames
    #[arg(long)]
    pub serial: bool,
}
