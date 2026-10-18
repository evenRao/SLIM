//! Export real Ratatui frames as cell data for repeatable visual inspection.
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
use slim_core::{
    model::{
        actor::{NewActor, Role},
        item::NewItem,
        location::{LocationKind, NewLocation},
        operation::{OperationSource, ReceiveStock},
        stock::StockThreshold,
    },
    service::{
        ActorRepository, InventoryService, ItemRepository, LocationRepository, ThresholdRepository,
    },
};
use slim_db::repo::{
    actor::SqliteActorRepo,
    inventory::{SqliteInventoryService, SqliteThresholdRepo},
    item::SqliteItemRepo,
    location::SqliteLocationRepo,
};
use slim_tui::{
    App,
    app::{Modal, ReceiveForm, Tab},
};

fn rgb(color: Color, background: bool) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        _ if background => [11, 18, 32],
        _ => [229, 238, 248],
    }
}
fn main() -> anyhow::Result<()> {
    let connection = slim_db::connection::open_memory()?;
    slim_db::migrate::run_migrations(&connection)?;
    let actor = SqliteActorRepo::new(&connection)
        .create(&NewActor::new("Alex Morgan").with_role(Role::Admin))?;
    let location = SqliteLocationRepo::new(&connection).create(&NewLocation {
        name: "Workshop / Drawer A".into(),
        kind: LocationKind::Drawer,
        parent_id: None,
    })?;
    for (name, sku, quantity, minimum) in [
        ("M3 socket head screws", "M3-10", 124, 20),
        ("Brass heat-set inserts", "INSERT-M3", 8, 25),
        ("Black cable ties", "TIE-150", 45, 15),
    ] {
        let item = SqliteItemRepo::new(&connection).create(&NewItem {
            name: name.into(),
            sku: Some(sku.into()),
            unit: "each".into(),
            description: Some("Workshop supplies".into()),
            metadata: None,
        })?;
        SqliteThresholdRepo::new(&connection).set(&StockThreshold {
            item_id: item.id,
            stock_location_id: None,
            min_qty: minimum,
        })?;
        SqliteInventoryService::new(&connection).receive(ReceiveStock {
            actor_id: actor.id,
            item_id: item.id,
            destination_id: location.id,
            quantity,
            source: OperationSource::Tui,
            reason: Some("New shipment".into()),
            idempotency_key: None,
        })?;
    }
    let unused = SqliteItemRepo::new(&connection).create(&NewItem {
        name: "Unused demo part".into(),
        sku: Some("UNUSED".into()),
        unit: "each".into(),
        description: None,
        metadata: None,
    })?;
    let mut app = App::new(&connection, actor)?;
    let directory = std::path::Path::new("target/tui-preview");
    std::fs::create_dir_all(directory)?;
    for (name, tab, modal) in [
        ("overview", Tab::Dashboard, false),
        ("inventory", Tab::Items, false),
        ("receive", Tab::Items, true),
        ("delete", Tab::Items, false),
    ] {
        app.switch_tab(tab);
        if modal {
            app.active_modal = Some(Modal::Receive(ReceiveForm {
                item: "M3-10".into(),
                location: location.name.clone(),
                quantity: "1".into(),
                reason: String::new(),
                focused_field: 2,
            }));
        }
        if name == "delete" {
            app.items_table_state
                .select(app.items.iter().position(|p| p.item.id == unused.id));
            app.open_delete_modal(false);
        }
        let mut terminal = Terminal::new(TestBackend::new(126, 34))?;
        terminal.draw(|f| slim_tui::ui::render(&mut app, f))?;
        let buffer = terminal.backend().buffer();
        let cells: Vec<_> = buffer
            .content
            .iter()
            .map(|c| {
                (
                    c.symbol(),
                    rgb(c.fg, false),
                    rgb(c.bg, true),
                    c.modifier.contains(Modifier::BOLD),
                )
            })
            .collect();
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec(&serde_json::json!({"width":126,"height":34,"cells":cells}))?,
        )?;
    }
    println!("Ratatui previews exported to {}", directory.display());
    Ok(())
}
