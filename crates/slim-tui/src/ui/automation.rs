//! Automation and Hook tab rendering for SLIM TUI.
use super::theme;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use slim_core::model::automation::AutomationRunStatus;

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    // ── Upper section: Configured Automation Rules ──
    let rule_rows: Vec<Row> = app
        .automation_rules
        .iter()
        .map(|r| {
            let enabled_style = if r.enabled {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default().fg(theme::MUTED)
            };

            let cond_str = if let Some(ref c) = r.condition {
                if let Some(below) = c.quantity_below {
                    format!("qty < {}", below)
                } else if let Some(above) = c.quantity_above {
                    format!("qty > {}", above)
                } else {
                    "-".to_string()
                }
            } else {
                "-".to_string()
            };

            Row::new(vec![
                Cell::from(r.id.clone()).fg(theme::TEXT).bold(),
                Cell::from(r.event.clone()).fg(theme::ACCENT),
                Cell::from(r.item_sku.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(cond_str).fg(theme::MUTED),
                Cell::from(r.action.action_type_str()).fg(theme::WARNING),
                Cell::from(if r.enabled { "Active" } else { "Disabled" }).style(enabled_style),
            ])
        })
        .collect();

    let rule_header = Row::new(vec![
        Cell::from("Rule ID").bold().fg(theme::ACCENT),
        Cell::from("Event Trigger").bold().fg(theme::ACCENT),
        Cell::from("SKU Filter").bold().fg(theme::ACCENT),
        Cell::from("Condition").bold().fg(theme::ACCENT),
        Cell::from("Action").bold().fg(theme::ACCENT),
        Cell::from("Status").bold().fg(theme::ACCENT),
    ])
    .bottom_margin(1);

    if app.automation_rules.is_empty() {
        let empty_p = Paragraph::new(
            " No automation rules configured.\n\n \
             Add rules via CLI: slim automation rules add <ID> --event <EVENT> --action-type notify --message \"...\"\n \
             Or define [[rules]] in config.toml or rules.toml",
        )
        .style(Style::default().fg(theme::MUTED))
        .block(
            Block::default().border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(" Automation Rules ")
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(empty_p, chunks[0]);
    } else {
        let rules_table = Table::new(
            rule_rows,
            [
                Constraint::Length(18),
                Constraint::Length(24),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(12),
                Constraint::Min(10),
            ],
        )
        .header(rule_header)
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(
                    " Automation Rules ({} active) ",
                    app.automation_rules.iter().filter(|r| r.enabled).count()
                ))
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(rules_table, chunks[0]);
    }

    // ── Lower section: Execution Run History ──
    let run_rows: Vec<Row> = app
        .automation_runs
        .iter()
        .map(|run| {
            let status_style = match run.status {
                AutomationRunStatus::Success => Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
                AutomationRunStatus::Failed => Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
                AutomationRunStatus::Skipped => Style::default()
                    .fg(theme::WARNING)
                    .add_modifier(Modifier::BOLD),
            };

            let time_str =
                run.executed_at.date().to_string() + " " + &run.executed_at.time().to_string()[..8];

            let detail = run
                .output
                .as_deref()
                .or(run.error_message.as_deref())
                .unwrap_or("-");

            Row::new(vec![
                Cell::from(time_str).fg(theme::MUTED),
                Cell::from(run.rule_id.clone()).fg(theme::TEXT),
                Cell::from(run.event_type.clone()).fg(theme::ACCENT),
                Cell::from(run.action_type.clone()).fg(theme::WARNING),
                Cell::from(run.status.to_string()).style(status_style),
                Cell::from(detail).fg(theme::MUTED),
            ])
        })
        .collect();

    let run_header = Row::new(vec![
        Cell::from("Executed At").bold().fg(theme::ACCENT),
        Cell::from("Rule ID").bold().fg(theme::ACCENT),
        Cell::from("Event").bold().fg(theme::ACCENT),
        Cell::from("Action").bold().fg(theme::ACCENT),
        Cell::from("Outcome").bold().fg(theme::ACCENT),
        Cell::from("Output / Error").bold().fg(theme::ACCENT),
    ])
    .bottom_margin(1);

    if app.automation_runs.is_empty() {
        let empty_runs = Paragraph::new(
            " No execution history recorded.\n\n \
             Committed events are queued in event_outbox and executed by 'slim automation worker'.",
        )
        .style(Style::default().fg(theme::MUTED))
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(
                    " Automation Execution History ({} pending outbox events) ",
                    app.pending_events_count
                ))
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(empty_runs, chunks[1]);
    } else {
        let runs_table = Table::new(
            run_rows,
            [
                Constraint::Length(20),
                Constraint::Length(16),
                Constraint::Length(22),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Min(25),
            ],
        )
        .header(run_header)
        .block(
            Block::default()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .borders(Borders::ALL)
                .title(format!(
                    " Automation Execution History ({} recorded | {} pending in outbox) ",
                    app.automation_runs.len(),
                    app.pending_events_count
                ))
                .border_style(Style::default().fg(theme::MUTED)),
        );
        frame.render_widget(runs_table, chunks[1]);
    }
}
