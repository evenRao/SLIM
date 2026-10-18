//! SLIM TUI application state, data management, and event handling.

use ratatui::widgets::{ListState, TableState};
use rusqlite::Connection;
use slim_core::error::{CoreError, CoreResult};
use slim_core::model::actor::Actor;
use slim_core::model::automation::{AutomationRule, AutomationRun};
use slim_core::model::gridfinity::{GridBase, GridMap};
use slim_core::model::item::{Item, ItemFilter};
use slim_core::model::location::Location;
use slim_core::model::operation::*;
use slim_core::model::scan::ScanTarget;
use slim_core::model::stock::LowStockAlert;

use slim_core::model::device::{ObservationStatus, RegisteredDevice, StoredObservation};
use slim_core::model::led::DockLedState;
use slim_core::service::{
    ActorRepository, AutomationRepository, DeviceRepository, GridRepository, InventoryService,
    ItemRepository, LedRepository, LocationRepository, ObservationRepository, OutboxRepository,
    ScanResolver,
};
use slim_db::repo::actor::SqliteActorRepo;
use slim_db::repo::automation::SqliteAutomationRepo;
use slim_db::repo::device::SqliteDeviceRepo;
use slim_db::repo::grid::SqliteGridRepo;
use slim_db::repo::inventory::SqliteInventoryService;
use slim_db::repo::item::SqliteItemRepo;
use slim_db::repo::led::SqliteLedRepo;
use slim_db::repo::location::SqliteLocationRepo;
use slim_db::repo::outbox::SqliteOutboxRepo;
use slim_db::repo::scan::SqliteScanResolver;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard = 0,
    Items = 1,
    Locations = 2,
    Gridfinity = 3,
    LowStock = 4,
    History = 5,
    Automation = 6,
    Devices = 7,
}

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Dashboard,
        Tab::Items,
        Tab::Locations,
        Tab::Gridfinity,
        Tab::LowStock,
        Tab::History,
        Tab::Automation,
        Tab::Devices,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Dashboard => "1: Dashboard",
            Tab::Items => "2: Items",
            Tab::Locations => "3: Locations",
            Tab::Gridfinity => "4: Gridfinity",
            Tab::LowStock => "5: Low Stock",
            Tab::History => "6: History",
            Tab::Automation => "7: Automation",
            Tab::Devices => "8: Devices",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub level: NotificationLevel,
    pub message: String,
    pub created_at: Instant,
}

// ── Modals & Form States ──

#[derive(Debug, Clone)]
pub enum Modal {
    Directory(crate::management::Directory),
    RecordEditor(crate::management::RecordEditor),
    RecordAction(crate::management::RecordAction),
    People(crate::people::PeopleForm),
    RemoveUser(crate::people::RemoveUserForm),
    Delete(crate::item_actions::DeleteForm),
    Catalog(crate::catalog::CatalogForm),
    Receive(ReceiveForm),
    Issue(IssueForm),
    Transfer(TransferForm),
    Adjust(AdjustForm),
    Search(SearchForm),
    Scan(ScanForm),
    User(UserForm),
    Help,
}

#[derive(Debug, Clone, Default)]
pub struct ScanForm {
    pub input: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReceiveForm {
    pub item: String,
    pub location: String,
    pub quantity: String,
    pub reason: String,
    pub focused_field: usize,
}

impl ReceiveForm {
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % 4;
    }
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            3
        } else {
            self.focused_field - 1
        };
    }
    pub fn active_field_mut(&mut self) -> &mut String {
        match self.focused_field {
            0 => &mut self.item,
            1 => &mut self.location,
            2 => &mut self.quantity,
            _ => &mut self.reason,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IssueForm {
    pub item: String,
    pub location: String,
    pub quantity: String,
    pub reason: String,
    pub focused_field: usize,
}

impl IssueForm {
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % 4;
    }
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            3
        } else {
            self.focused_field - 1
        };
    }
    pub fn active_field_mut(&mut self) -> &mut String {
        match self.focused_field {
            0 => &mut self.item,
            1 => &mut self.location,
            2 => &mut self.quantity,
            _ => &mut self.reason,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransferForm {
    pub item: String,
    pub from_location: String,
    pub to_location: String,
    pub quantity: String,
    pub reason: String,
    pub focused_field: usize,
}

impl TransferForm {
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % 5;
    }
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            4
        } else {
            self.focused_field - 1
        };
    }
    pub fn active_field_mut(&mut self) -> &mut String {
        match self.focused_field {
            0 => &mut self.item,
            1 => &mut self.from_location,
            2 => &mut self.to_location,
            3 => &mut self.quantity,
            _ => &mut self.reason,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AdjustForm {
    pub item: String,
    pub location: String,
    pub new_quantity: String,
    pub reason: String,
    pub focused_field: usize,
}

impl AdjustForm {
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % 4;
    }
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            3
        } else {
            self.focused_field - 1
        };
    }
    pub fn active_field_mut(&mut self) -> &mut String {
        match self.focused_field {
            0 => &mut self.item,
            1 => &mut self.location,
            2 => &mut self.new_quantity,
            _ => &mut self.reason,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchForm {
    pub query: String,
}

#[derive(Debug, Clone, Default)]
pub struct UserForm {
    pub actors: Vec<Actor>,
    pub selected_idx: usize,
    pub pin: String,
    pub focused_field: usize,
    pub error: Option<String>,
}

impl UserForm {
    pub fn select_next(&mut self) {
        if !self.actors.is_empty() {
            self.selected_idx = (self.selected_idx + 1) % self.actors.len();
        }
    }
    pub fn select_prev(&mut self) {
        if !self.actors.is_empty() {
            self.selected_idx = if self.selected_idx == 0 {
                self.actors.len() - 1
            } else {
                self.selected_idx - 1
            };
        }
    }
}

// ── Presentation models ──

#[derive(Debug, Clone)]
pub struct ItemPresentation {
    pub item: Item,
    pub total_on_hand: u64,
    pub is_low_stock: bool,
    pub balances: Vec<(String, u32)>,
}

#[derive(Debug, Clone)]
pub struct LocationPresentation {
    pub location: Location,
    pub path: String,
    pub depth: usize,
    pub items: Vec<(String, String, u32)>,
}

#[derive(Debug, Clone)]
pub struct OperationDisplayItem {
    pub id: OperationId,
    pub kind: OperationKind,
    pub actor_name: String,
    pub source: OperationSource,
    pub reason: Option<String>,
    pub movement_count: u32,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, Default)]
pub struct DashboardMetrics {
    pub total_items: usize,
    pub total_units: u64,
    pub low_stock_count: usize,
    pub total_locations: usize,
    pub pending_outbox_count: usize,
}

/// The central application state for the TUI.
pub struct App<'a> {
    pub conn: &'a Connection,
    pub actor: Actor,
    pub authenticated: bool,
    pub(crate) authenticated_pin_hash: Option<String>,
    pub current_tab: Tab,
    pub active_modal: Option<Modal>,
    pub should_quit: bool,
    pub edit_cursor: Option<usize>,
    pub selected_grid_bin: usize,
    pub show_deleted: bool,
    pub notification: Option<Notification>,

    // Data caches
    pub metrics: DashboardMetrics,
    pub items: Vec<ItemPresentation>,
    pub locations: Vec<LocationPresentation>,
    pub low_stock_alerts: Vec<LowStockAlert>,
    pub history_entries: Vec<OperationDisplayItem>,
    pub grid_bases: Vec<GridBase>,
    pub active_base_grid_map: Option<GridMap>,
    pub automation_rules: Vec<AutomationRule>,
    pub automation_runs: Vec<AutomationRun>,
    pub pending_events_count: usize,
    pub devices: Vec<RegisteredDevice>,
    pub observations: Vec<StoredObservation>,
    pub dock_leds: Vec<DockLedState>,

    // Widget states
    pub items_table_state: TableState,
    pub locations_list_state: ListState,
    pub low_stock_table_state: TableState,
    pub history_table_state: TableState,
    pub grid_base_list_state: ListState,
    pub automation_table_state: TableState,
    pub devices_table_state: TableState,
    pub observations_table_state: TableState,
}

impl<'a> App<'a> {
    pub fn new(conn: &'a Connection, actor: Actor) -> CoreResult<Self> {
        let authenticated_pin_hash = SqliteActorRepo::new(conn).get_pin_hash(&actor.id)?;
        let mut app = Self {
            conn,
            actor,
            authenticated: true,
            authenticated_pin_hash,
            current_tab: Tab::Dashboard,
            active_modal: None,
            should_quit: false,
            edit_cursor: None,
            selected_grid_bin: 0,
            show_deleted: false,
            notification: None,

            metrics: DashboardMetrics::default(),
            items: Vec::new(),
            locations: Vec::new(),
            low_stock_alerts: Vec::new(),
            history_entries: Vec::new(),
            grid_bases: Vec::new(),
            active_base_grid_map: None,
            automation_rules: Vec::new(),
            automation_runs: Vec::new(),
            pending_events_count: 0,
            devices: Vec::new(),
            observations: Vec::new(),
            dock_leds: Vec::new(),

            items_table_state: TableState::default(),
            locations_list_state: ListState::default(),
            low_stock_table_state: TableState::default(),
            history_table_state: TableState::default(),
            grid_base_list_state: ListState::default(),
            automation_table_state: TableState::default(),
            devices_table_state: TableState::default(),
            observations_table_state: TableState::default(),
        };

        app.refresh_data()?;
        Ok(app)
    }

    /// Start with setup or login inside Ratatui, without exposing inventory.
    pub fn new_locked(conn: &'a Connection) -> CoreResult<Self> {
        let mut app = Self::new(
            conn,
            Actor {
                id: slim_core::model::actor::ActorId(uuid::Uuid::nil()),
                display_name: "Sign in".into(),
                os_username: None,
                role: slim_core::model::actor::Role::Viewer,
                active: true,
                created_at: time::OffsetDateTime::now_utc(),
            },
        )?;
        app.authenticated = false;
        if SqliteActorRepo::new(conn).list()?.is_empty() {
            app.open_catalog(crate::catalog::CatalogKind::Setup);
        } else {
            app.open_user_modal();
        }
        Ok(app)
    }

    pub fn set_notification(&mut self, level: NotificationLevel, message: impl Into<String>) {
        self.notification = Some(Notification {
            level,
            message: message.into(),
            created_at: Instant::now(),
        });
    }

    /// Refresh all cached data from the SQLite database.
    pub fn refresh_data(&mut self) -> CoreResult<()> {
        let item_repo = SqliteItemRepo::new(self.conn);
        let loc_repo = SqliteLocationRepo::new(self.conn);
        let inv_service = SqliteInventoryService::new(self.conn);

        // 1. Load Items & Balances
        let raw_items = item_repo.list(&ItemFilter::default())?;
        let view_items = if self.show_deleted {
            item_repo
                .list(&ItemFilter {
                    include_archived: true,
                    search: None,
                })?
                .into_iter()
                .filter(|item| item.archived_at.is_some())
                .collect()
        } else {
            raw_items.clone()
        };
        let low_stock = inv_service.get_low_stock()?;
        let low_stock_item_ids: std::collections::HashSet<_> =
            low_stock.iter().map(|a| a.item_id).collect();

        let mut items_pres = Vec::with_capacity(raw_items.len());
        let mut total_units: u64 = 0;

        for item in &view_items {
            let balances = inv_service.get_item_balances(&item.id)?;
            let total_on_hand: u64 = balances.iter().map(|b| u64::from(b.qty)).sum();
            total_units += total_on_hand;

            let mut location_balances = Vec::new();
            for b in balances {
                if b.qty > 0 {
                    let path_str = match loc_repo.get_path(&b.location_id) {
                        Ok(p) => p
                            .into_iter()
                            .map(|l| l.name)
                            .collect::<Vec<_>>()
                            .join(" > "),
                        Err(_) => b.location_id.to_string(),
                    };
                    location_balances.push((path_str, b.qty));
                }
            }

            items_pres.push(ItemPresentation {
                item: item.clone(),
                total_on_hand,
                is_low_stock: low_stock_item_ids.contains(&item.id),
                balances: location_balances,
            });
        }

        // 2. Load Locations & Tree
        let mut loc_pres = Vec::new();
        self.build_location_tree(&loc_repo, &inv_service, &raw_items, None, 0, &mut loc_pres)?;

        // 3. Load Low Stock Alerts
        self.low_stock_alerts = low_stock;

        // 4. Load History (last 50 operations)
        let operations_history = self.query_recent_operations(50)?;

        // 5. Compute Metrics
        self.metrics = DashboardMetrics {
            total_items: raw_items.len(),
            total_units,
            low_stock_count: self.low_stock_alerts.len(),
            total_locations: loc_pres.len(),
            pending_outbox_count: 0,
        };

        self.items = items_pres;
        self.locations = loc_pres;
        self.history_entries = operations_history;

        // Ensure selection states remain valid
        self.items_table_state.select(if self.items.is_empty() {
            None
        } else {
            Some(
                self.items_table_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.items.len() - 1),
            )
        });
        if !self.locations.is_empty() && self.locations_list_state.selected().is_none() {
            self.locations_list_state.select(Some(0));
        }
        if !self.low_stock_alerts.is_empty() && self.low_stock_table_state.selected().is_none() {
            self.low_stock_table_state.select(Some(0));
        }
        if !self.history_entries.is_empty() && self.history_table_state.selected().is_none() {
            self.history_table_state.select(Some(0));
        }
        // 6. Load Gridfinity baseplates & active grid map
        let grid_repo = SqliteGridRepo::new(self.conn);
        self.grid_bases = grid_repo.list_bases(None).unwrap_or_default();
        if !self.grid_bases.is_empty() {
            let sel = self.grid_base_list_state.selected().unwrap_or(0);
            let valid_sel = sel.min(self.grid_bases.len() - 1);
            self.grid_base_list_state.select(Some(valid_sel));
            self.active_base_grid_map = grid_repo
                .render_grid_map(&self.grid_bases[valid_sel].id)
                .ok();
        } else {
            self.active_base_grid_map = None;
            self.grid_base_list_state.select(None);
        }

        // 7. Load Automation Rules, Runs & Outbox status
        let auto_repo = SqliteAutomationRepo::new(self.conn);
        self.automation_rules = auto_repo.list_rules().unwrap_or_default();
        self.automation_runs = auto_repo.list_runs(50).unwrap_or_default();
        let outbox_repo = SqliteOutboxRepo::new(self.conn);
        self.pending_events_count = outbox_repo.pending_count().unwrap_or(0);
        self.metrics.pending_outbox_count = self.pending_events_count;
        if !self.automation_runs.is_empty() && self.automation_table_state.selected().is_none() {
            self.automation_table_state.select(Some(0));
        }

        // 8. Load Hardware Devices & Observations
        let dev_repo = SqliteDeviceRepo::new(self.conn);
        self.devices = dev_repo.list_devices().unwrap_or_default();
        self.observations = dev_repo
            .list_observations(false, None, 50)
            .unwrap_or_default();
        if !self.devices.is_empty() && self.devices_table_state.selected().is_none() {
            self.devices_table_state.select(Some(0));
        }
        if !self.observations.is_empty() && self.observations_table_state.selected().is_none() {
            self.observations_table_state.select(Some(0));
        }

        // 9. Load Dock LED States
        let led_repo = SqliteLedRepo::new(self.conn);
        let _ = led_repo.clear_expired_leds(time::OffsetDateTime::now_utc());
        self.dock_leds = led_repo.list_dock_leds(None).unwrap_or_default();

        Ok(())
    }

    fn query_recent_operations(&self, limit: usize) -> CoreResult<Vec<OperationDisplayItem>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT o.id, o.kind, a.display_name, o.source, o.reason, o.created_at,
                    COUNT(m.id) as movement_count
             FROM operations o
             JOIN actors a ON o.actor_id = a.id
             LEFT JOIN stock_movements m ON o.id = m.operation_id
             GROUP BY o.id
             ORDER BY o.created_at DESC
             LIMIT ?1",
            )
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map([limit], |row| {
                let op_id_str: String = row.get(0)?;
                let kind_str: String = row.get(1)?;
                let actor_name: String = row.get(2)?;
                let source_str: String = row.get(3)?;
                let reason: Option<String> = row.get(4)?;
                let created_at_str: String = row.get(5)?;
                let movement_count: u32 = row.get(6)?;

                let op_id = op_id_str.parse().unwrap_or_default();
                let kind = kind_str.parse().unwrap_or(OperationKind::Receive);
                let source = source_str.parse().unwrap_or(OperationSource::Cli);
                let created_at = time::OffsetDateTime::parse(
                    &created_at_str,
                    &time::format_description::well_known::Rfc3339,
                )
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);

                Ok(OperationDisplayItem {
                    id: op_id,
                    kind,
                    actor_name,
                    source,
                    reason,
                    movement_count,
                    created_at,
                })
            })
            .map_err(|e| CoreError::Storage(e.to_string()))?;

        let mut results = Vec::new();
        for item in rows.flatten() {
            results.push(item);
        }
        Ok(results)
    }

    fn build_location_tree(
        &self,
        loc_repo: &SqliteLocationRepo,
        inv_service: &SqliteInventoryService,
        items: &[Item],
        parent_id: Option<&slim_core::model::location::LocationId>,
        depth: usize,
        out: &mut Vec<LocationPresentation>,
    ) -> CoreResult<()> {
        let children = loc_repo.list_children(parent_id)?;
        for loc in children {
            let path = loc_repo
                .get_path(&loc.id)?
                .into_iter()
                .map(|l| l.name)
                .collect::<Vec<_>>()
                .join(" > ");

            let balances = inv_service.get_location_balances(&loc.id)?;
            let mut stocked_items = Vec::new();
            for b in balances {
                if b.qty > 0
                    && let Some(it) = items.iter().find(|i| i.id == b.item_id)
                {
                    stocked_items.push((
                        it.sku.clone().unwrap_or_default(),
                        it.name.clone(),
                        b.qty,
                    ));
                }
            }

            out.push(LocationPresentation {
                location: loc.clone(),
                path,
                depth,
                items: stocked_items,
            });

            self.build_location_tree(loc_repo, inv_service, items, Some(&loc.id), depth + 1, out)?;
        }
        Ok(())
    }

    // ── Navigation methods ──

    pub fn next_tab(&mut self) {
        let idx = (self.current_tab as usize + 1) % Tab::ALL.len();
        self.current_tab = Tab::ALL[idx];
    }

    pub fn prev_tab(&mut self) {
        let idx = if self.current_tab as usize == 0 {
            Tab::ALL.len() - 1
        } else {
            self.current_tab as usize - 1
        };
        self.current_tab = Tab::ALL[idx];
    }

    pub fn switch_tab(&mut self, tab: Tab) {
        if tab != Tab::Items {
            self.show_deleted = false;
        }
        self.current_tab = tab;
        if let Err(error) = self.refresh_data() {
            self.set_notification(NotificationLevel::Error, error.to_string());
        }
    }

    pub fn move_selection_down(&mut self) {
        match self.current_tab {
            Tab::Dashboard => {}
            Tab::Items => {
                if !self.items.is_empty() {
                    let next = match self.items_table_state.selected() {
                        Some(i) => (i + 1).min(self.items.len() - 1),
                        None => 0,
                    };
                    self.items_table_state.select(Some(next));
                }
            }
            Tab::Locations => {
                if !self.locations.is_empty() {
                    let next = match self.locations_list_state.selected() {
                        Some(i) => (i + 1).min(self.locations.len() - 1),
                        None => 0,
                    };
                    self.locations_list_state.select(Some(next));
                }
            }
            Tab::Gridfinity => {
                if !self.grid_bases.is_empty() {
                    let next = match self.grid_base_list_state.selected() {
                        Some(i) => (i + 1).min(self.grid_bases.len() - 1),
                        None => 0,
                    };
                    self.grid_base_list_state.select(Some(next));
                    self.update_active_grid_map();
                }
            }
            Tab::LowStock => {
                if !self.low_stock_alerts.is_empty() {
                    let next = match self.low_stock_table_state.selected() {
                        Some(i) => (i + 1).min(self.low_stock_alerts.len() - 1),
                        None => 0,
                    };
                    self.low_stock_table_state.select(Some(next));
                }
            }
            Tab::History => {
                if !self.history_entries.is_empty() {
                    let next = match self.history_table_state.selected() {
                        Some(i) => (i + 1).min(self.history_entries.len() - 1),
                        None => 0,
                    };
                    self.history_table_state.select(Some(next));
                }
            }
            Tab::Automation => {
                if !self.automation_runs.is_empty() {
                    let next = match self.automation_table_state.selected() {
                        Some(i) => (i + 1).min(self.automation_runs.len() - 1),
                        None => 0,
                    };
                    self.automation_table_state.select(Some(next));
                }
            }
            Tab::Devices => {
                if !self.observations.is_empty() {
                    let next = match self.observations_table_state.selected() {
                        Some(i) => (i + 1).min(self.observations.len() - 1),
                        None => 0,
                    };
                    self.observations_table_state.select(Some(next));
                }
            }
        }
    }

    pub fn move_selection_up(&mut self) {
        match self.current_tab {
            Tab::Dashboard => {}
            Tab::Items => {
                if !self.items.is_empty() {
                    let prev = match self.items_table_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.items_table_state.select(Some(prev));
                }
            }
            Tab::Locations => {
                if !self.locations.is_empty() {
                    let prev = match self.locations_list_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.locations_list_state.select(Some(prev));
                }
            }
            Tab::Gridfinity => {
                if !self.grid_bases.is_empty() {
                    let prev = match self.grid_base_list_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.grid_base_list_state.select(Some(prev));
                    self.update_active_grid_map();
                }
            }
            Tab::LowStock => {
                if !self.low_stock_alerts.is_empty() {
                    let prev = match self.low_stock_table_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.low_stock_table_state.select(Some(prev));
                }
            }
            Tab::History => {
                if !self.history_entries.is_empty() {
                    let prev = match self.history_table_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.history_table_state.select(Some(prev));
                }
            }
            Tab::Automation => {
                if !self.automation_runs.is_empty() {
                    let prev = match self.automation_table_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.automation_table_state.select(Some(prev));
                }
            }
            Tab::Devices => {
                if !self.observations.is_empty() {
                    let prev = match self.observations_table_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    self.observations_table_state.select(Some(prev));
                }
            }
        }
    }

    pub fn update_active_grid_map(&mut self) {
        self.selected_grid_bin = 0;
        if let Some(sel) = self.grid_base_list_state.selected()
            && let Some(base) = self.grid_bases.get(sel)
        {
            let grid_repo = SqliteGridRepo::new(self.conn);
            self.active_base_grid_map = grid_repo.render_grid_map(&base.id).ok();
        }
    }

    pub fn confirm_selected_observation(&mut self) -> CoreResult<()> {
        if !self.actor.role.can_transact() {
            self.set_notification(
                NotificationLevel::Warning,
                format!(
                    "Role '{}' cannot review observations (Operator required)",
                    self.actor.role
                ),
            );
            return Ok(());
        }

        if let Some(sel) = self.observations_table_state.selected()
            && let Some(obs) = self.observations.get(sel).cloned()
        {
            if obs.status == ObservationStatus::PendingReview {
                let dev_repo = SqliteDeviceRepo::new(self.conn);
                dev_repo.confirm_observation(&obs.id, &self.actor.id, None)?;
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Observation {} confirmed", obs.id),
                );
                self.refresh_data()?;
            } else {
                self.set_notification(
                    NotificationLevel::Warning,
                    format!("Observation is already {:?}", obs.status),
                );
            }
        }
        Ok(())
    }

    pub fn reject_selected_observation(&mut self) -> CoreResult<()> {
        if !self.actor.role.can_transact() {
            self.set_notification(
                NotificationLevel::Warning,
                format!(
                    "Role '{}' cannot review observations (Operator required)",
                    self.actor.role
                ),
            );
            return Ok(());
        }

        if let Some(sel) = self.observations_table_state.selected()
            && let Some(obs) = self.observations.get(sel).cloned()
        {
            if obs.status == ObservationStatus::PendingReview {
                let dev_repo = SqliteDeviceRepo::new(self.conn);
                dev_repo.reject_observation(&obs.id, &self.actor.id)?;
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Observation {} rejected", obs.id),
                );
                self.refresh_data()?;
            } else {
                self.set_notification(
                    NotificationLevel::Warning,
                    format!("Observation is already {:?}", obs.status),
                );
            }
        }
        Ok(())
    }

    pub fn locate_selected_grid_bin(&mut self) -> CoreResult<()> {
        if let Some(ref gm) = self.active_base_grid_map {
            if let Some(first_placed) = gm.placed_bins.get(self.selected_grid_bin) {
                let led_repo = SqliteLedRepo::new(self.conn);
                let now = time::OffsetDateTime::now_utc();
                let states = led_repo.locate_target(&first_placed.bin_code, Some(30), now)?;
                self.set_notification(
                    NotificationLevel::Success,
                    format!(
                        "Illuminated {} dock LEDs for bin '{}'",
                        states.len(),
                        first_placed.bin_code
                    ),
                );
                self.refresh_data()?;
            } else {
                self.set_notification(
                    NotificationLevel::Info,
                    "No bins placed on this baseplate to locate",
                );
            }
        }
        Ok(())
    }

    // ── Modal Openers ──

    pub fn open_receive_modal(&mut self) {
        if !self.actor.role.can_transact() {
            self.set_notification(
                NotificationLevel::Warning,
                format!(
                    "Role '{}' cannot receive stock (Operator required)",
                    self.actor.role
                ),
            );
            return;
        }
        let item_hint = self.selected_item_identifier();
        self.active_modal = Some(Modal::Receive(ReceiveForm {
            item: item_hint.unwrap_or_default(),
            location: String::new(),
            quantity: String::new(),
            reason: String::new(),
            focused_field: 0,
        }));
    }

    pub fn open_issue_modal(&mut self) {
        if !self.actor.role.can_transact() {
            self.set_notification(
                NotificationLevel::Warning,
                format!(
                    "Role '{}' cannot issue stock (Operator required)",
                    self.actor.role
                ),
            );
            return;
        }
        let item_hint = self.selected_item_identifier();
        self.active_modal = Some(Modal::Issue(IssueForm {
            item: item_hint.unwrap_or_default(),
            location: String::new(),
            quantity: String::new(),
            reason: String::new(),
            focused_field: 0,
        }));
    }

    pub fn open_transfer_modal(&mut self) {
        if !self.actor.role.can_transact() {
            self.set_notification(
                NotificationLevel::Warning,
                format!(
                    "Role '{}' cannot transfer stock (Operator required)",
                    self.actor.role
                ),
            );
            return;
        }
        let item_hint = self.selected_item_identifier();
        self.active_modal = Some(Modal::Transfer(TransferForm {
            item: item_hint.unwrap_or_default(),
            from_location: String::new(),
            to_location: String::new(),
            quantity: String::new(),
            reason: String::new(),
            focused_field: 0,
        }));
    }

    pub fn open_adjust_modal(&mut self) {
        if !self.actor.role.can_adjust() {
            self.set_notification(
                NotificationLevel::Warning,
                format!(
                    "Role '{}' cannot adjust inventory (Manager or Admin required)",
                    self.actor.role
                ),
            );
            return;
        }
        let item_hint = self.selected_item_identifier();
        self.active_modal = Some(Modal::Adjust(AdjustForm {
            item: item_hint.unwrap_or_default(),
            location: String::new(),
            new_quantity: String::new(),
            reason: String::new(),
            focused_field: 0,
        }));
    }

    pub fn open_user_modal(&mut self) {
        let repo = SqliteActorRepo::new(self.conn);
        let actors = repo
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.active)
            .collect::<Vec<_>>();
        let selected_idx = actors
            .iter()
            .position(|a| a.id == self.actor.id)
            .unwrap_or(0);
        self.active_modal = Some(Modal::User(UserForm {
            actors,
            selected_idx,
            pin: String::new(),
            focused_field: 0,
            error: None,
        }));
    }

    pub fn open_search_modal(&mut self) {
        self.active_modal = Some(Modal::Search(SearchForm::default()));
    }

    pub fn open_scan_modal(&mut self) {
        self.active_modal = Some(Modal::Scan(ScanForm::default()));
    }

    pub fn open_help_modal(&mut self) {
        self.active_modal = Some(Modal::Help);
    }

    pub fn close_modal(&mut self) {
        if !self.authenticated {
            self.should_quit = true;
        }
        self.edit_cursor = None;
        self.active_modal = None;
    }

    fn selected_item_identifier(&self) -> Option<String> {
        if self.current_tab == Tab::Items {
            if let Some(i) = self.items_table_state.selected()
                && let Some(pres) = self.items.get(i)
            {
                return pres
                    .item
                    .sku
                    .clone()
                    .or_else(|| Some(pres.item.name.clone()));
            }
        } else if self.current_tab == Tab::LowStock
            && let Some(i) = self.low_stock_table_state.selected()
            && let Some(alert) = self.low_stock_alerts.get(i)
        {
            return alert
                .item_sku
                .clone()
                .or_else(|| Some(alert.item_name.clone()));
        }
        None
    }

    // ── Form Submissions ──

    pub fn submit_active_modal(&mut self) {
        let modal = self.active_modal.clone();
        match modal {
            Some(Modal::RecordEditor(form)) => self.submit_record_editor(form),
            Some(Modal::RecordAction(form)) => self.submit_record_action(form),
            Some(Modal::Directory(_)) => self.directory_edit(false),
            Some(Modal::People(_)) => self.open_selected_person(false),
            Some(Modal::RemoveUser(form)) => self.submit_remove_user(form),
            Some(Modal::Delete(form)) => self.submit_delete(form),
            Some(Modal::Catalog(form)) => self.submit_catalog(form),
            Some(Modal::Receive(form)) => self.submit_receive(form),
            Some(Modal::Issue(form)) => self.submit_issue(form),
            Some(Modal::Transfer(form)) => self.submit_transfer(form),
            Some(Modal::Adjust(form)) => self.submit_adjust(form),
            Some(Modal::Search(form)) => self.submit_search(form),
            Some(Modal::Scan(form)) => self.submit_scan(form),
            Some(Modal::User(form)) => self.submit_user(form),
            Some(Modal::Help) => self.close_modal(),
            None => {}
        }
    }

    fn submit_user(&mut self, mut form: UserForm) {
        if let Some(target_actor) = form.actors.get(form.selected_idx).cloned() {
            let repo = SqliteActorRepo::new(self.conn);
            let target_actor = match repo.get_by_id(&target_actor.id) {
                Ok(Some(actor)) if actor.active => actor,
                _ => {
                    form.error = Some("Account is unavailable".into());
                    self.active_modal = Some(Modal::User(form));
                    return;
                }
            };
            let pin_hash = match repo.get_pin_hash(&target_actor.id) {
                Ok(hash) if target_actor.active => hash,
                _ => {
                    self.set_notification(NotificationLevel::Error, "Account is unavailable");
                    return;
                }
            };
            if let Some(ref hash) = pin_hash
                && !slim_core::model::auth::verify_pin(&form.pin, hash)
            {
                form.error = Some("Invalid PIN for actor".to_string());
                self.active_modal = Some(Modal::User(form));
                return;
            }
            let prev_name = self.actor.display_name.clone();
            self.actor = target_actor;
            self.authenticated = true;
            self.authenticated_pin_hash = pin_hash;
            self.set_notification(
                NotificationLevel::Success,
                format!(
                    "Switched active actor from '{}' to '{}' [{}]",
                    prev_name, self.actor.display_name, self.actor.role
                ),
            );
            self.close_modal();
        }
    }

    fn submit_receive(&mut self, form: ReceiveForm) {
        let qty: u32 = match form.quantity.trim().parse() {
            Ok(q) if q > 0 => q,
            _ => {
                self.set_notification(
                    NotificationLevel::Error,
                    "Quantity must be a positive integer",
                );
                return;
            }
        };

        let (item_id, item_label) = match self.resolve_item(&form.item) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let (dest_id, dest_label) = match self.resolve_location(&form.location) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let inv_service = SqliteInventoryService::new(self.conn);
        let cmd = ReceiveStock {
            actor_id: self.actor.id,
            item_id,
            destination_id: dest_id,
            quantity: qty,
            source: OperationSource::Tui,
            reason: if form.reason.trim().is_empty() {
                None
            } else {
                Some(form.reason.trim().to_string())
            },
            idempotency_key: None,
        };

        match inv_service.receive(cmd) {
            Ok(_) => {
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Received {qty} × {item_label} into {dest_label}"),
                );
                self.close_modal();
                let _ = self.refresh_data();
            }
            Err(e) => {
                self.set_notification(NotificationLevel::Error, format!("Receive failed: {e}"));
            }
        }
    }

    fn submit_issue(&mut self, form: IssueForm) {
        let qty: u32 = match form.quantity.trim().parse() {
            Ok(q) if q > 0 => q,
            _ => {
                self.set_notification(
                    NotificationLevel::Error,
                    "Quantity must be a positive integer",
                );
                return;
            }
        };

        let (item_id, item_label) = match self.resolve_item(&form.item) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let (src_id, src_label) = match self.resolve_location(&form.location) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let inv_service = SqliteInventoryService::new(self.conn);
        let cmd = IssueStock {
            actor_id: self.actor.id,
            item_id,
            source_id: src_id,
            quantity: qty,
            source: OperationSource::Tui,
            reason: if form.reason.trim().is_empty() {
                None
            } else {
                Some(form.reason.trim().to_string())
            },
            idempotency_key: None,
        };

        match inv_service.issue(cmd) {
            Ok(_) => {
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Issued {qty} × {item_label} from {src_label}"),
                );
                self.close_modal();
                let _ = self.refresh_data();
            }
            Err(e) => {
                self.set_notification(NotificationLevel::Error, format!("Issue failed: {e}"));
            }
        }
    }

    fn submit_transfer(&mut self, form: TransferForm) {
        let qty: u32 = match form.quantity.trim().parse() {
            Ok(q) if q > 0 => q,
            _ => {
                self.set_notification(
                    NotificationLevel::Error,
                    "Quantity must be a positive integer",
                );
                return;
            }
        };

        let (item_id, item_label) = match self.resolve_item(&form.item) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let (from_id, from_label) = match self.resolve_location(&form.from_location) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let (to_id, to_label) = match self.resolve_location(&form.to_location) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let inv_service = SqliteInventoryService::new(self.conn);
        let cmd = TransferStock {
            actor_id: self.actor.id,
            item_id,
            from_id,
            to_id,
            quantity: qty,
            source: OperationSource::Tui,
            reason: if form.reason.trim().is_empty() {
                None
            } else {
                Some(form.reason.trim().to_string())
            },
            idempotency_key: None,
        };

        match inv_service.transfer(cmd) {
            Ok(_) => {
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Transferred {qty} × {item_label} ({from_label} -> {to_label})"),
                );
                self.close_modal();
                let _ = self.refresh_data();
            }
            Err(e) => {
                self.set_notification(NotificationLevel::Error, format!("Transfer failed: {e}"));
            }
        }
    }

    fn submit_adjust(&mut self, form: AdjustForm) {
        let new_qty: u32 = match form.new_quantity.trim().parse() {
            Ok(q) => q,
            _ => {
                self.set_notification(NotificationLevel::Error, "Quantity must be non-negative");
                return;
            }
        };

        if form.reason.trim().is_empty() {
            self.set_notification(
                NotificationLevel::Error,
                "Reason is mandatory for stock adjustment",
            );
            return;
        }

        let (item_id, item_label) = match self.resolve_item(&form.item) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let (loc_id, loc_label) = match self.resolve_location(&form.location) {
            Ok(res) => res,
            Err(e) => {
                self.set_notification(NotificationLevel::Error, e);
                return;
            }
        };

        let inv_service = SqliteInventoryService::new(self.conn);
        let cmd = AdjustStock {
            actor_id: self.actor.id,
            item_id,
            location_id: loc_id,
            new_quantity: new_qty,
            source: OperationSource::Tui,
            reason: form.reason.trim().to_string(),
            idempotency_key: None,
        };

        match inv_service.adjust(cmd) {
            Ok(_) => {
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Adjusted {item_label} at {loc_label} to {new_qty}"),
                );
                self.close_modal();
                let _ = self.refresh_data();
            }
            Err(e) => {
                self.set_notification(NotificationLevel::Error, format!("Adjustment failed: {e}"));
            }
        }
    }

    fn submit_search(&mut self, form: SearchForm) {
        let q = form.query.trim().to_lowercase();
        if q.is_empty() {
            self.close_modal();
            return;
        }

        // Search in items
        if let Some(idx) = self.items.iter().position(|it| {
            it.item
                .sku
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&q)
                || it.item.name.to_lowercase().contains(&q)
        }) {
            self.current_tab = Tab::Items;
            self.items_table_state.select(Some(idx));
            self.set_notification(
                NotificationLevel::Info,
                format!("Found item: {}", self.items[idx].item.name),
            );
            self.close_modal();
            return;
        }

        // Search in locations
        if let Some(idx) = self.locations.iter().position(|loc| {
            loc.location.name.to_lowercase().contains(&q) || loc.path.to_lowercase().contains(&q)
        }) {
            self.current_tab = Tab::Locations;
            self.locations_list_state.select(Some(idx));
            self.set_notification(
                NotificationLevel::Info,
                format!("Found location: {}", self.locations[idx].location.name),
            );
            self.close_modal();
            return;
        }

        self.set_notification(
            NotificationLevel::Warning,
            format!("No item or location matching '{q}'"),
        );
        self.close_modal();
    }

    fn submit_scan(&mut self, form: ScanForm) {
        let code = form.input.trim();
        if code.is_empty() {
            self.close_modal();
            return;
        }

        let resolver = SqliteScanResolver::new(self.conn);
        match resolver.resolve_scan(code) {
            Ok(ScanTarget::Item(item)) => {
                self.current_tab = Tab::Items;
                if let Some(idx) = self.items.iter().position(|it| it.item.id == item.id) {
                    self.items_table_state.select(Some(idx));
                }
                self.set_notification(
                    NotificationLevel::Success,
                    format!(
                        "Scanned Item: {} ({})",
                        item.name,
                        item.sku.as_deref().unwrap_or("none")
                    ),
                );
                self.close_modal();
            }
            Ok(ScanTarget::Bin { bin, location: _ }) => {
                self.current_tab = Tab::Gridfinity;
                let grid_repo = SqliteGridRepo::new(self.conn);
                if let Ok(Some(placement)) = grid_repo.get_active_placement_for_bin(&bin.id)
                    && let Some(b_idx) = self
                        .grid_bases
                        .iter()
                        .position(|gb| gb.id == placement.placement.base_id)
                {
                    self.grid_base_list_state.select(Some(b_idx));
                    self.update_active_grid_map();
                }
                self.set_notification(
                    NotificationLevel::Success,
                    format!(
                        "Scanned Gridfinity Bin: {} ({}x{}x{}U)",
                        bin.bin_code, bin.width_units, bin.depth_units, bin.height_units
                    ),
                );
                self.close_modal();
            }
            Ok(ScanTarget::Location(location)) => {
                self.current_tab = Tab::Locations;
                if let Some(idx) = self
                    .locations
                    .iter()
                    .position(|loc| loc.location.id == location.id)
                {
                    self.locations_list_state.select(Some(idx));
                }
                self.set_notification(
                    NotificationLevel::Success,
                    format!("Scanned Location: {} [{}]", location.name, location.kind),
                );
                self.close_modal();
            }
            Ok(ScanTarget::Unregistered { scanned_code }) => {
                self.set_notification(
                    NotificationLevel::Warning,
                    format!("Unregistered barcode: '{}'", scanned_code),
                );
                self.close_modal();
            }
            Err(e) => {
                self.set_notification(
                    NotificationLevel::Error,
                    format!("Scan resolution error: {e}"),
                );
                self.close_modal();
            }
        }
    }

    // ── Resolution Helpers ──

    fn resolve_item(
        &self,
        input: &str,
    ) -> Result<(slim_core::model::item::ItemId, String), String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Item name or SKU is required".to_string());
        }

        let item_repo = SqliteItemRepo::new(self.conn);
        if let Ok(id) = trimmed.parse()
            && let Ok(Some(item)) = item_repo.get_by_id(&id)
            && item.archived_at.is_none()
        {
            return Ok((item.id, item.name));
        }
        if let Ok(Some(item)) = item_repo.get_by_sku(trimmed) {
            let label = item.sku.clone().unwrap_or_else(|| item.name.clone());
            return Ok((item.id, label));
        }

        let all = item_repo
            .list(&ItemFilter::default())
            .map_err(|e| e.to_string())?;
        let matches: Vec<_> = all
            .into_iter()
            .filter(|i| {
                i.name.eq_ignore_ascii_case(trimmed)
                    || i.sku
                        .as_deref()
                        .map(|s| s.eq_ignore_ascii_case(trimmed))
                        .unwrap_or(false)
            })
            .collect();

        if matches.len() == 1 {
            let item = &matches[0];
            let label = item.sku.clone().unwrap_or_else(|| item.name.clone());
            Ok((item.id, label))
        } else if matches.is_empty() {
            Err(format!("Item '{trimmed}' not found"))
        } else {
            Err(format!(
                "Item name '{trimmed}' is ambiguous ({count} matches)",
                count = matches.len()
            ))
        }
    }

    pub(crate) fn resolve_location(
        &self,
        input: &str,
    ) -> Result<(slim_core::model::location::LocationId, String), String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Location name is required".to_string());
        }

        let loc_repo = SqliteLocationRepo::new(self.conn);
        if let Ok(id) = trimmed.parse()
            && let Ok(Some(loc)) = loc_repo.get_by_id(&id)
        {
            return Ok((loc.id, loc.name));
        }
        if let Ok(Some(loc)) = loc_repo.find_by_name(trimmed, None) {
            return Ok((loc.id, loc.name));
        }

        let matches: Vec<_> = self
            .locations
            .iter()
            .filter(|l| {
                l.location.name.eq_ignore_ascii_case(trimmed)
                    || l.path.eq_ignore_ascii_case(trimmed)
            })
            .collect();

        if matches.len() == 1 {
            Ok((matches[0].location.id, matches[0].location.name.clone()))
        } else if matches.is_empty() {
            Err(format!("Location '{trimmed}' not found"))
        } else {
            Err(format!("Location name '{trimmed}' is ambiguous"))
        }
    }
}
