//! Locations tab rendering for SLIM TUI.
use super::theme;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // ── Location Tree List (Left) ──
    let items: Vec<ListItem> = app
        .locations
        .iter()
        .map(|loc| {
            let indent = "  ".repeat(loc.depth);
            let prefix = if loc.depth == 0 { "• " } else { "└─ " };

            let kind_tag = format!(" [{}]", loc.location.kind);
            let line = Line::from(vec![
                Span::raw(indent),
                Span::styled(prefix, Style::default().fg(theme::MUTED)),
                Span::styled(&loc.location.name, Style::default().fg(theme::TEXT).bold()),
                Span::styled(kind_tag, Style::default().fg(theme::ACCENT)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(" Location Hierarchy ({}) ", app.locations.len()))
                .border_style(Style::default().fg(theme::MUTED)),
        )
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, chunks[0], &mut app.locations_list_state);

    // ── Location Details & Contained Stock (Right) ──
    let selected_idx = app.locations_list_state.selected().unwrap_or(0);
    if let Some(loc_pres) = app.locations.get(selected_idx) {
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(10), Constraint::Min(6)])
            .split(chunks[1]);

        let kind_str = loc_pres.location.kind.to_string();
        let mut info_lines = vec![
            Line::from(vec![
                Span::styled(
                    "Name:           ",
                    Style::default().fg(theme::ACCENT).bold(),
                ),
                Span::raw(&loc_pres.location.name),
            ]),
            Line::from(vec![
                Span::styled(
                    "Kind:           ",
                    Style::default().fg(theme::ACCENT).bold(),
                ),
                Span::raw(kind_str),
            ]),
            Line::from(vec![
                Span::styled(
                    "Path:           ",
                    Style::default().fg(theme::ACCENT).bold(),
                ),
                Span::raw(&loc_pres.path),
            ]),
            Line::from(vec![
                Span::styled(
                    "Can Hold Stock: ",
                    Style::default().fg(theme::ACCENT).bold(),
                ),
                if loc_pres.location.can_hold_stock {
                    Span::styled("Yes", Style::default().fg(theme::ACCENT))
                } else {
                    Span::styled("No", Style::default().fg(theme::ERROR))
                },
            ]),
        ];

        if let Ok(record) = slim_db::repo::catalog::SqliteCatalogService::new(app.conn).get(
            slim_db::repo::catalog::CatalogKind::Storage,
            &loc_pres.location.id.to_string(),
        ) {
            let data = record.data;
            if let Some(label) = data["type_label"].as_str().filter(|s| !s.is_empty()) {
                info_lines.push(Line::from(format!("Type: {label}")));
            }
            let dimensions = ["width_mm", "depth_mm", "height_mm"]
                .iter()
                .map(|key| {
                    data[*key]
                        .as_f64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "—".into())
                })
                .collect::<Vec<_>>()
                .join(" × ");
            if ["width_mm", "depth_mm", "height_mm"]
                .iter()
                .any(|k| data[*k].is_number())
            {
                info_lines.push(Line::from(format!("Size: {dimensions} mm")));
            }
            if let Some(notes) = data["notes"].as_str().filter(|s| !s.is_empty()) {
                info_lines.push(Line::from(format!("Notes: {notes}")));
            }
        }
        let p_info = Paragraph::new(info_lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::default()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .borders(Borders::ALL)
                    .title(" Location Info ")
                    .border_style(Style::default().fg(theme::MUTED)),
            );
        frame.render_widget(p_info, right_chunks[0]);

        // Items in this location
        let item_rows: Vec<Row> = if loc_pres.items.is_empty() {
            vec![Row::new(vec![
                Cell::from("-").fg(theme::MUTED),
                Cell::from("No items stored in this location").fg(theme::MUTED),
                Cell::from("0"),
            ])]
        } else {
            loc_pres
                .items
                .iter()
                .map(|(sku, name, qty)| {
                    Row::new(vec![
                        Cell::from(sku.clone()).fg(theme::MUTED),
                        Cell::from(name.clone()),
                        Cell::from(qty.to_string()).bold().fg(theme::ACCENT),
                    ])
                })
                .collect()
        };

        let item_header = Row::new(vec![
            Cell::from("SKU").bold().fg(theme::ACCENT),
            Cell::from("Item Name").bold().fg(theme::ACCENT),
            Cell::from("Quantity").bold().fg(theme::ACCENT),
        ])
        .bottom_margin(1);

        let item_table = Table::new(
            item_rows,
            [
                Constraint::Length(14),
                Constraint::Min(20),
                Constraint::Length(10),
            ],
        )
        .header(item_header)
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(
                    " Items at this Location ({}) ",
                    loc_pres.items.len()
                ))
                .border_style(Style::default().fg(theme::MUTED)),
        );

        frame.render_widget(item_table, right_chunks[1]);
    } else {
        let p_empty = Paragraph::new("No location selected").block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(" Location Info ")
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(p_empty, chunks[1]);
    }
}
