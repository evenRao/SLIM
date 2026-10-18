//! Overview: concise metrics and readable recent activity.
use super::theme::*;
use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Min(5),
    ])
    .split(area);
    let cards = Layout::horizontal([Constraint::Fill(1); 3])
        .spacing(1)
        .split(sections[0]);
    for (index, (label, number)) in [
        ("Items", app.metrics.total_items.to_string()),
        ("Stock on hand", app.metrics.total_units.to_string()),
        ("Low stock", app.metrics.low_stock_count.to_string()),
    ]
    .into_iter()
    .enumerate()
    {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(" {number}"),
                    Style::default()
                        .fg(if index == 2 && app.metrics.low_stock_count > 0 {
                            WARNING
                        } else {
                            ACCENT
                        })
                        .bold(),
                )),
                Line::from(Span::styled(
                    if index == 0 {
                        format!(" {} locations", app.metrics.total_locations)
                    } else if index == 1 {
                        " units in storage".into()
                    } else {
                        " need restocking".into()
                    },
                    Style::default().fg(MUTED),
                )),
            ])
            .block(panel(label)),
            cards[index],
        );
    }
    if app.history_entries.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Ready when you are",
                    Style::default().fg(TEXT).bold(),
                )),
                Line::from(""),
                Line::from("  n  Add an item to your catalog"),
                Line::from("  r  Receive stock into a location"),
                Line::from("  2  Browse and edit inventory"),
                Line::from(""),
                Line::from(Span::styled(
                    "  Changes are saved after each successful operation.",
                    Style::default().fg(MUTED),
                )),
            ])
            .block(panel("Recent Inventory Activity")),
            sections[2],
        );
        return;
    }
    let compact = area.width < 80;
    let rows = app.history_entries.iter().map(|op| {
        let stamp = format!(
            "{} {:02}:{:02}",
            op.created_at.date(),
            op.created_at.hour(),
            op.created_at.minute()
        );
        let mut cells = vec![
            Cell::from(stamp).fg(MUTED),
            Cell::from(op.kind.to_string()).fg(ACCENT),
        ];
        if !compact {
            cells.push(Cell::from(op.actor_name.clone()));
        }
        cells.push(
            Cell::from(
                op.reason
                    .clone()
                    .unwrap_or_else(|| format!("{} movement(s)", op.movement_count)),
            )
            .fg(MUTED),
        );
        Row::new(cells).height(2)
    });
    let mut headings = vec!["When", "Action"];
    let mut widths = vec![Constraint::Length(17), Constraint::Length(10)];
    if !compact {
        headings.push("By");
        widths.push(Constraint::Length(16));
    }
    headings.push("Details");
    widths.push(Constraint::Min(12));
    frame.render_widget(
        Table::new(rows, widths)
            .header(
                Row::new(headings)
                    .style(Style::default().fg(MUTED).bold())
                    .bottom_margin(1),
            )
            .block(panel("Recent Inventory Activity")),
        sections[2],
    );
}
