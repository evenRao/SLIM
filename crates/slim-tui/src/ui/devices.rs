//! Hardware devices and sensor observations tab rendering for SLIM TUI.
use super::theme;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use slim_core::model::device::ObservationStatus;

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let compact = area.width < 110;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // ── Upper section: Registered Hardware Devices ──
    let dev_rows: Vec<Row> = app
        .devices
        .iter()
        .map(|d| {
            let active_style = if d.active {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default().fg(theme::MUTED)
            };

            let port_str = d.port_or_address.as_deref().unwrap_or("─");
            let loc_str = match d.location_id {
                Some(ref lid) => app
                    .locations
                    .iter()
                    .find(|l| l.location.id == *lid)
                    .map(|l| l.location.name.as_str())
                    .unwrap_or("Unknown"),
                None => "─",
            };

            let mut cells = vec![
                Cell::from(d.id.clone()).fg(theme::TEXT).bold(),
                Cell::from(d.name.clone()).fg(theme::ACCENT),
            ];
            if !compact {
                cells.extend([
                    Cell::from(d.kind.to_string()).fg(theme::WARNING),
                    Cell::from(port_str.to_string()).fg(theme::MUTED),
                    Cell::from(loc_str.to_string()).fg(theme::ACCENT),
                ]);
            }
            cells
                .push(Cell::from(if d.active { "Active" } else { "Inactive" }).style(active_style));
            Row::new(cells)
        })
        .collect();

    let dev_header = if compact {
        Row::new(vec!["Device ID", "Name", "Status"])
            .style(Style::default().fg(theme::ACCENT).bold())
            .bottom_margin(1)
    } else {
        Row::new(vec![
            Cell::from("Device ID").bold().fg(theme::ACCENT),
            Cell::from("Name").bold().fg(theme::ACCENT),
            Cell::from("Kind").bold().fg(theme::ACCENT),
            Cell::from("Port / Address").bold().fg(theme::ACCENT),
            Cell::from("Location").bold().fg(theme::ACCENT),
            Cell::from("Status").bold().fg(theme::ACCENT),
        ])
        .bottom_margin(1)
    };

    if app.devices.is_empty() {
        let empty_p = Paragraph::new(
            " No hardware devices registered.\n\n \
             Add devices via CLI: slim device add <ID> --name <NAME> --kind <KIND> [--port <PORT>]\n \
             Supported kinds: serial_scanner, ir_breakbeam, ir_presence, rfid_nfc, mock_sensor",
        )
        .style(Style::default().fg(theme::MUTED))
        .block(
            Block::default().border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(" Hardware Devices ")
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(empty_p, chunks[0]);
    } else {
        let devices_table = Table::new(
            dev_rows,
            if compact {
                vec![
                    Constraint::Length(18),
                    Constraint::Min(20),
                    Constraint::Length(9),
                ]
            } else {
                vec![
                    Constraint::Length(18),
                    Constraint::Length(22),
                    Constraint::Length(18),
                    Constraint::Length(16),
                    Constraint::Length(20),
                    Constraint::Min(10),
                ]
            },
        )
        .header(dev_header)
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(
                    " Hardware Devices ({} registered, {} active) ",
                    app.devices.len(),
                    app.devices.iter().filter(|d| d.active).count()
                ))
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(devices_table, chunks[0]);
    }

    // ── Lower section: Sensor Observations Log & Review Queue ──
    let pending_count = app
        .observations
        .iter()
        .filter(|o| o.status == ObservationStatus::PendingReview)
        .count();

    let obs_rows: Vec<Row> = app
        .observations
        .iter()
        .map(|o| {
            let status_cell = match o.status {
                ObservationStatus::PendingReview => {
                    Cell::from("Pending Review").fg(theme::WARNING).bold()
                }
                ObservationStatus::Confirmed => Cell::from("Confirmed").fg(theme::ACCENT),
                ObservationStatus::Rejected => Cell::from("Rejected").fg(theme::ERROR),
                ObservationStatus::Processed => Cell::from("Processed").fg(theme::ACCENT),
            };

            let ts_str = o.created_at.to_string();
            let short_ts = if ts_str.len() > 19 {
                &ts_str[..19]
            } else {
                &ts_str
            };

            let short_id = if o.id.to_string().len() > 8 {
                &o.id.to_string()[..8]
            } else {
                &o.id.to_string()
            };

            let mut cells = vec![
                Cell::from(short_id.to_string()).fg(theme::TEXT).bold(),
                Cell::from(o.device_id.clone()).fg(theme::ACCENT),
            ];
            if !compact {
                cells.push(Cell::from(short_ts.to_string()).fg(theme::MUTED));
            }
            let event_label = match o.observation_type.as_str() {
                "IdentifierScanned" => "Label scanned",
                "BeamInterrupted" => "Beam interrupted",
                "PresenceChanged" => "Presence changed",
                other => other,
            };
            cells.push(Cell::from(event_label.to_string()).fg(theme::ACCENT));
            if !compact {
                cells.push(Cell::from(o.payload.clone()).fg(theme::MUTED));
            }
            cells.push(status_cell);
            Row::new(cells)
        })
        .collect();

    let obs_header = if compact {
        Row::new(vec!["ID", "Device", "Event", "Status"])
            .style(Style::default().fg(theme::ACCENT).bold())
            .bottom_margin(1)
    } else {
        Row::new(vec![
            Cell::from("Obs ID").bold().fg(theme::ACCENT),
            Cell::from("Device").bold().fg(theme::ACCENT),
            Cell::from("Timestamp").bold().fg(theme::ACCENT),
            Cell::from("Event Type").bold().fg(theme::ACCENT),
            Cell::from("Payload").bold().fg(theme::ACCENT),
            Cell::from("Review Status").bold().fg(theme::ACCENT),
        ])
        .bottom_margin(1)
    };

    if app.observations.is_empty() {
        let empty_obs = Paragraph::new(
            " No sensor observations recorded.\n\n \
             Simulate observations via CLI: slim device simulate <DEV_ID> --event beam_interrupted --zone \"ZONE-1\"\n \
             Or stream live serial frames using: slim device listen --device <DEV_ID>",
        )
        .style(Style::default().fg(theme::MUTED))
        .block(
            Block::default().border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(" Observations Log & Review Queue ")
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(empty_obs, chunks[1]);
    } else {
        let obs_table = Table::new(
            obs_rows,
            if compact {
                vec![
                    Constraint::Length(8),
                    Constraint::Length(16),
                    Constraint::Min(20),
                    Constraint::Length(14),
                ]
            } else {
                vec![
                    Constraint::Length(10),
                    Constraint::Length(16),
                    Constraint::Length(21),
                    Constraint::Length(20),
                    Constraint::Percentage(40),
                    Constraint::Min(16),
                ]
            },
        )
        .header(obs_header)
        .row_highlight_style(
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(
                    " Observations Queue ({} pending review) [c: Confirm | x: Reject] ",
                    pending_count
                ))
                .border_style(Style::default().fg(theme::MUTED)),
        );

        frame.render_stateful_widget(obs_table, chunks[1], &mut app.observations_table_state);
    }
}
