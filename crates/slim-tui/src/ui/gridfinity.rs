//! Gridfinity tab rendering for SLIM TUI.
use super::theme;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};
use slim_core::model::gridfinity::GridCell;
use slim_core::model::led::LedMode;

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(40)])
        .split(area);

    // ── Baseplates List (Left) ──
    let items: Vec<ListItem> = app
        .grid_bases
        .iter()
        .map(|base| {
            let line = Line::from(vec![
                Span::styled(&base.code, Style::default().fg(theme::ACCENT).bold()),
                Span::raw(format!(" ({}x{} cells)", base.cols, base.rows)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(" Baseplates ({}) ", app.grid_bases.len()))
                .border_style(Style::default().fg(theme::MUTED)),
        )
        .highlight_style(
            Style::default()
                .bg(theme::SELECTED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, chunks[0], &mut app.grid_base_list_state);

    // ── 2D Grid Visualizer & Placed Bins (Right) ──
    if let Some(ref grid_map) = app.active_base_grid_map {
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(10)])
            .split(chunks[1]);

        // 2D Grid Matrix
        let mut map_lines = Vec::new();
        map_lines.push(Line::from(vec![
            Span::styled(
                format!("Baseplate: {}", grid_map.base.code),
                Style::default().bold().fg(theme::ACCENT),
            ),
            Span::raw(format!(
                "  Dimensions: {} x {} units ({:.1} x {:.1} mm)  Pitch: {:.1} mm",
                grid_map.base.cols,
                grid_map.base.rows,
                grid_map.base.width_mm(),
                grid_map.base.depth_mm(),
                grid_map.base.pitch_mm
            )),
        ]));
        map_lines.push(Line::from(""));

        // Column headers
        let mut col_spans = vec![Span::raw("    ")];
        for x in 0..grid_map.base.cols {
            col_spans.push(Span::styled(
                format!("  X{:<4}", x),
                Style::default().fg(theme::MUTED),
            ));
        }
        map_lines.push(Line::from(col_spans));

        // Rows
        for (y, row) in grid_map.cells.iter().enumerate() {
            let mut row_spans = vec![Span::styled(
                format!("Y{:<2} ", y),
                Style::default().fg(theme::MUTED),
            )];
            for (x, cell) in row.iter().enumerate() {
                let dock_id = format!("{}:{}:{}", grid_map.base.code, x, y);
                let led_state = app.dock_leds.iter().find(|l| {
                    (l.base_id == grid_map.base.id && l.x == x as u32 && l.y == y as u32)
                        || l.dock_id == dock_id
                });
                let active_mode = led_state.and_then(|l| {
                    if l.mode == LedMode::Off {
                        None
                    } else if let Some(exp) = l.expires_at {
                        if exp > time::OffsetDateTime::now_utc() {
                            Some(l.mode)
                        } else {
                            None
                        }
                    } else {
                        Some(l.mode)
                    }
                });

                match cell {
                    GridCell::Empty => match active_mode {
                        Some(LedMode::Missing) => {
                            row_spans.push(Span::styled(
                                "[!MIS!]",
                                Style::default().fg(theme::ERROR).bold(),
                            ));
                        }
                        Some(LedMode::Locate) => {
                            row_spans.push(Span::styled(
                                "[*LOC*]",
                                Style::default().fg(theme::ACCENT).bold().reversed(),
                            ));
                        }
                        Some(LedMode::LowStock) => {
                            row_spans.push(Span::styled(
                                "[?LOW?]",
                                Style::default().fg(theme::WARNING).bold(),
                            ));
                        }
                        _ => {
                            row_spans
                                .push(Span::styled(" [ . ] ", Style::default().fg(theme::MUTED)));
                        }
                    },
                    GridCell::Occupied {
                        bin_code,
                        is_origin,
                        ..
                    } => {
                        let text = if *is_origin {
                            format!("{:^3}", bin_code.chars().take(3).collect::<String>())
                        } else {
                            " # ".to_string()
                        };
                        match active_mode {
                            Some(LedMode::Missing) => {
                                row_spans.push(Span::styled(
                                    format!("[!{text}!]"),
                                    Style::default().fg(theme::ERROR).bold(),
                                ));
                            }
                            Some(LedMode::Locate) => {
                                row_spans.push(Span::styled(
                                    format!("[*{text}*]"),
                                    Style::default().fg(theme::ACCENT).bold().reversed(),
                                ));
                            }
                            Some(LedMode::LowStock) => {
                                row_spans.push(Span::styled(
                                    format!("[?{text}?]"),
                                    Style::default().fg(theme::WARNING).bold(),
                                ));
                            }
                            _ => {
                                row_spans.push(Span::styled(
                                    format!(" [{text}] "),
                                    Style::default().fg(theme::ACCENT).bold(),
                                ));
                            }
                        }
                    }
                }
            }
            map_lines.push(Line::from(row_spans));
        }

        let p_grid = Paragraph::new(map_lines).block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(" Storage Map [Left/Right: Select bin | L: Locate] ")
                .border_style(Style::default().fg(theme::ACCENT)),
        );
        frame.render_widget(p_grid, right_chunks[0]);

        // Placed Bins Table
        let bin_rows: Vec<Row> = if grid_map.placed_bins.is_empty() {
            vec![Row::new(vec![
                Cell::from("No bins currently placed on this baseplate").fg(theme::MUTED),
                Cell::from("-"),
                Cell::from("-"),
                Cell::from("-"),
                Cell::from("-"),
            ])]
        } else {
            grid_map
                .placed_bins
                .iter()
                .map(|p| {
                    let (ew, ed) = p.effective_dimensions();
                    let covered = slim_core::model::led::calculate_covered_docks(
                        &grid_map.base.code,
                        p.placement.x,
                        p.placement.y,
                        p.width_units,
                        p.depth_units,
                        p.placement.rotation,
                    );
                    let bin_led = app.dock_leds.iter().find(|l| {
                        covered.iter().any(|(d, _, _)| d == &l.dock_id) && l.mode != LedMode::Off
                    });
                    let (led_text, led_color) = match bin_led.map(|l| l.mode) {
                        Some(LedMode::Missing) => ("[MISSING]", theme::ERROR),
                        Some(LedMode::Locate) => ("[LOCATE]", theme::ACCENT),
                        Some(LedMode::LowStock) => ("[LOW STOCK]", theme::WARNING),
                        Some(LedMode::Custom) => ("[CUSTOM]", theme::ACCENT),
                        _ => ("OFF", theme::MUTED),
                    };

                    Row::new(vec![
                        Cell::from(p.bin_code.clone())
                            .bold()
                            .fg(match bin_led.map(|l| l.mode) {
                                Some(LedMode::Missing) => theme::ERROR,
                                Some(LedMode::Locate) => theme::ACCENT,
                                Some(LedMode::LowStock) => theme::WARNING,
                                _ => theme::ACCENT,
                            }),
                        Cell::from(format!("{}x{}", ew, ed)),
                        Cell::from(format!("({}, {})", p.placement.x, p.placement.y)),
                        Cell::from(format!("{}°", p.placement.rotation)),
                        Cell::from(led_text).bold().fg(led_color),
                    ])
                })
                .collect()
        };

        let bin_header = Row::new(vec![
            Cell::from("Bin Code").bold().fg(theme::ACCENT),
            Cell::from("Footprint (WxD)").bold().fg(theme::ACCENT),
            Cell::from("Origin (X, Y)").bold().fg(theme::ACCENT),
            Cell::from("Rotation").bold().fg(theme::ACCENT),
            Cell::from("LED State").bold().fg(theme::ACCENT),
        ])
        .bottom_margin(1);

        let bin_table = Table::new(
            bin_rows,
            [
                Constraint::Length(16),
                Constraint::Length(18),
                Constraint::Length(16),
                Constraint::Length(12),
                Constraint::Length(14),
            ],
        )
        .header(bin_header)
        .row_highlight_style(Style::default().bg(theme::SELECTED).bold())
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(" Placed Bins ({}) ", grid_map.placed_bins.len()))
                .border_style(Style::default().fg(theme::MUTED)),
        );

        let mut selection =
            ratatui::widgets::TableState::default().with_selected(Some(app.selected_grid_bin));
        frame.render_stateful_widget(bin_table, right_chunks[1], &mut selection);
    } else {
        let p_empty = Paragraph::new("No baseplate selected or none registered").block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(" 2D Storage Layout Map ")
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(p_empty, chunks[1]);
    }
}
