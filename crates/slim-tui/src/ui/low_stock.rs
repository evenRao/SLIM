//! Low stock alerts tab rendering for SLIM TUI.
use super::theme;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .split(area);

    let rows: Vec<Row> = if app.low_stock_alerts.is_empty() {
        vec![Row::new(vec![
            Cell::from("-").fg(theme::MUTED),
            Cell::from("All items are currently above minimum stock thresholds!").fg(theme::ACCENT),
            Cell::from("-"),
            Cell::from("-"),
            Cell::from("-"),
            Cell::from("0"),
        ])]
    } else {
        app.low_stock_alerts
            .iter()
            .map(|alert| {
                let deficit = alert.min_qty.saturating_sub(alert.on_hand);
                Row::new(vec![
                    Cell::from(alert.item_sku.clone().unwrap_or_else(|| "-".to_string())),
                    Cell::from(alert.item_name.clone()).bold(),
                    Cell::from(alert.location_name.clone()),
                    Cell::from(alert.on_hand.to_string())
                        .fg(theme::ERROR)
                        .bold(),
                    Cell::from(alert.min_qty.to_string()).fg(theme::WARNING),
                    Cell::from(format!("-{deficit}")).fg(theme::ERROR).bold(),
                ])
            })
            .collect()
    };

    let header = Row::new(vec![
        Cell::from("SKU").bold().fg(theme::ACCENT),
        Cell::from("Item Name").bold().fg(theme::ACCENT),
        Cell::from("Location").bold().fg(theme::ACCENT),
        Cell::from("On Hand").bold().fg(theme::ACCENT),
        Cell::from("Min Threshold").bold().fg(theme::ACCENT),
        Cell::from("Deficit").bold().fg(theme::ACCENT),
    ])
    .bottom_margin(1);

    let count = app.low_stock_alerts.len();
    let border_color = if count > 0 {
        theme::ERROR
    } else {
        theme::ACCENT
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Min(10),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .borders(Borders::ALL)
            .title(format!(" Low Stock Alerts ({count}) "))
            .border_style(Style::default().fg(border_color)),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(50, 20, 20))
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, chunks[0], &mut app.low_stock_table_state);

    // Tip at bottom
    let tip =
        Paragraph::new(" Tip: Press 'r' to quickly receive stock to replenish the selected item ")
            .style(Style::default().fg(theme::MUTED));
    frame.render_widget(tip, chunks[1]);
}
