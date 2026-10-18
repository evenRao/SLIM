//! Inventory table and a focused detail panel.
use super::theme::*;
use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let wide = area.width >= 110;
    let chunks = Layout::default()
        .direction(if wide {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if wide {
            [Constraint::Percentage(60), Constraint::Percentage(40)]
        } else {
            [Constraint::Min(6), Constraint::Length(6)]
        })
        .spacing(1)
        .split(area);
    let compact = chunks[0].width < 75;
    let mut headings = vec!["Item"];
    let mut widths = vec![Constraint::Min(15)];
    if !compact {
        headings.push("SKU");
        widths.push(Constraint::Length(13));
    }
    headings.extend(["On hand", "Status"]);
    widths.extend([Constraint::Length(10), Constraint::Length(9)]);
    let rows = app.items.iter().map(|pres| {
        let mut cells = vec![Cell::from(pres.item.name.clone())];
        if !compact {
            cells.push(Cell::from(pres.item.sku.clone().unwrap_or_else(|| "—".into())).fg(MUTED));
        }
        cells.push(Cell::from(pres.total_on_hand.to_string()).fg(TEXT).bold());
        cells.push(
            Cell::from(if pres.item.archived_at.is_some() {
                "Deleted"
            } else if pres.is_low_stock {
                "Low"
            } else {
                "In stock"
            })
            .fg(if pres.is_low_stock { WARNING } else { ACCENT }),
        );
        Row::new(cells).height(2)
    });
    frame.render_stateful_widget(
        Table::new(rows, widths)
            .header(
                Row::new(headings)
                    .style(Style::default().fg(MUTED).bold())
                    .bottom_margin(1),
            )
            .block(panel(format!(
                "{} · {}",
                if app.show_deleted {
                    "Deleted items"
                } else {
                    "Catalog Items"
                },
                app.items.len()
            )))
            .row_highlight_style(Style::default().bg(SELECTED).fg(TEXT).bold())
            .highlight_symbol("▌ "),
        chunks[0],
        &mut app.items_table_state,
    );
    let details = match app
        .items_table_state
        .selected()
        .and_then(|i| app.items.get(i))
    {
        Some(pres) => {
            let balances = if pres.item.archived_at.is_some() {
                "Deleted. R restores; d permanently erases this item and its history.".into()
            } else if pres.balances.is_empty() {
                "No stock yet. Press r to receive.".into()
            } else {
                pres.balances
                    .iter()
                    .map(|(path, qty)| format!("{path}: {qty}"))
                    .collect::<Vec<_>>()
                    .join("  ·  ")
            };
            let mut lines = vec![
                Line::from(Span::styled(
                    &pres.item.name,
                    Style::default().fg(TEXT).bold(),
                )),
                Line::from(Span::styled(
                    format!(
                        "{}  ·  {} {}",
                        pres.item.sku.as_deref().unwrap_or("No SKU"),
                        pres.total_on_hand,
                        pres.item.unit
                    ),
                    Style::default().fg(ACCENT),
                )),
            ];
            if wide {
                lines.extend([
                    Line::from(""),
                    Line::from(pres.item.description.clone().unwrap_or_default()),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Stock by location",
                        Style::default().fg(MUTED).bold(),
                    )),
                ]);
            }
            lines.push(Line::from(balances));
            lines
        }
        None if app.show_deleted => vec![
            Line::from("No deleted items."),
            Line::from("Press v to return to active inventory."),
        ],
        None => vec![
            Line::from("Your catalog is empty."),
            Line::from("Press n to add your first item."),
        ],
    };
    frame.render_widget(
        Paragraph::new(details)
            .wrap(Wrap { trim: false })
            .block(panel("Item Details")),
        chunks[1],
    );
}
