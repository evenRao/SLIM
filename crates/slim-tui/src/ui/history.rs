//! History / Audit Log tab rendering for SLIM TUI.
use super::theme;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use slim_core::model::operation::OperationKind;

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let rows: Vec<Row> = app
        .history_entries
        .iter()
        .map(|op| {
            let kind_style = match op.kind {
                OperationKind::Receive => Style::default().fg(theme::ACCENT).bold(),
                OperationKind::Issue => Style::default().fg(theme::ERROR).bold(),
                OperationKind::Transfer => Style::default().fg(theme::ACCENT).bold(),
                OperationKind::Adjust => Style::default().fg(theme::WARNING).bold(),
            };

            let time_str =
                op.created_at.date().to_string() + " " + &op.created_at.time().to_string()[..8];
            Row::new(vec![
                Cell::from(time_str).fg(theme::MUTED),
                Cell::from(op.kind.to_string()).style(kind_style),
                Cell::from(op.actor_name.clone()),
                Cell::from(op.source.to_string()).fg(theme::MUTED),
                Cell::from(op.movement_count.to_string()),
                Cell::from(op.reason.clone().unwrap_or_else(|| "-".to_string())),
            ])
        })
        .collect();

    let header = Row::new(vec![
        Cell::from("Timestamp").bold().fg(theme::ACCENT),
        Cell::from("Kind").bold().fg(theme::ACCENT),
        Cell::from("Actor").bold().fg(theme::ACCENT),
        Cell::from("Source").bold().fg(theme::ACCENT),
        Cell::from("Movements").bold().fg(theme::ACCENT),
        Cell::from("Reason").bold().fg(theme::ACCENT),
    ])
    .bottom_margin(1);

    let table = Table::new(
        rows,
        [
            Constraint::Length(19),
            Constraint::Length(8),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .borders(Borders::ALL)
            .title(format!(
                " Operation Audit Ledger ({}) ",
                app.history_entries.len()
            ))
            .border_style(Style::default().fg(theme::MUTED)),
    )
    .row_highlight_style(
        Style::default()
            .bg(theme::SELECTED)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, &mut app.history_table_state);
}
