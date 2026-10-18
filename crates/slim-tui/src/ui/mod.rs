//! Responsive sidebar, workspace, and contextual status bar.
pub mod automation;
pub mod dashboard;
pub mod devices;
pub mod gridfinity;
pub mod history;
pub mod items;
pub mod locations;
pub mod low_stock;
pub mod modals;
pub mod theme;

use crate::app::{App, NotificationLevel, Tab};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use theme::*;

pub fn render(app: &mut App, frame: &mut Frame) {
    let size = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(BACKGROUND).fg(TEXT)),
        size,
    );
    if size.width < 60 || size.height < 18 {
        frame.render_widget(
            Paragraph::new("SLIM\nPlease enlarge the terminal to at least 80 x 24.")
                .style(Style::default().fg(TEXT)),
            size,
        );
        return;
    }
    if !app.authenticated {
        frame.render_widget(
            Paragraph::new(format!(
                " SLIM {} / local inventory",
                env!("CARGO_PKG_VERSION")
            ))
            .style(Style::default().fg(ACCENT)),
            size,
        );
        modals::render(app, frame);
        return;
    }
    let horizontal = Layout::horizontal([
        Constraint::Length(if size.width >= 100 { 22 } else { 16 }),
        Constraint::Min(40),
    ])
    .split(size);
    render_sidebar(app, frame, horizontal[0]);
    let area = horizontal[1].inner(Margin::new(1, 0));
    let workspace = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(9),
        Constraint::Length(3),
    ])
    .split(area);
    let title = match app.current_tab {
        Tab::Dashboard => "Overview",
        Tab::Items => "Inventory",
        Tab::Locations => "Locations",
        Tab::Gridfinity => "Gridfinity",
        Tab::LowStock => "Low stock",
        Tab::History => "Activity",
        Tab::Automation => "Automation",
        Tab::Devices => "Devices",
    };
    let subtitle = match app.current_tab {
        Tab::Dashboard => "Your workspace at a glance",
        Tab::Items => "Select an item to inspect stock or make a change",
        Tab::Locations => "Organize storage and see what belongs where",
        Tab::Gridfinity => "Arrange bases and bins in your physical workspace",
        Tab::LowStock => "Items that need replenishment",
        Tab::History => "A record of inventory movements",
        Tab::Automation => "Rules, pending events, and recent results",
        Tab::Devices => "Connected devices and observations to review",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(title, Style::default().fg(TEXT).bold())),
            Line::from(Span::styled(subtitle, Style::default().fg(MUTED))),
        ]),
        workspace[0],
    );
    let content = workspace[1];
    match app.current_tab {
        Tab::Dashboard => dashboard::render(app, frame, content),
        Tab::Items => items::render(app, frame, content),
        Tab::Locations => locations::render(app, frame, content),
        Tab::Gridfinity => gridfinity::render(app, frame, content),
        Tab::LowStock => low_stock::render(app, frame, content),
        Tab::History => history::render(app, frame, content),
        Tab::Automation => automation::render(app, frame, content),
        Tab::Devices => devices::render(app, frame, content),
    }
    render_footer(app, frame, workspace[2]);
    modals::render(app, frame);
}

fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(Style::default().bg(SIDEBAR)), area);
    let mut lines = vec![
        Line::from(Span::styled(" SLIM", Style::default().fg(ACCENT).bold())),
        Line::from(Span::styled(
            format!(" {} · offline", env!("CARGO_PKG_VERSION")),
            Style::default().fg(MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled(" WORKSPACE", Style::default().fg(MUTED))),
        Line::from(""),
    ];
    for tab in Tab::ALL {
        let label = match tab {
            Tab::Dashboard => "1: Overview",
            Tab::Items => "2: Inventory",
            Tab::Locations => "3: Locations",
            Tab::Gridfinity => "4: Gridfinity",
            Tab::LowStock => "5: Low stock",
            Tab::History => "6: Activity",
            Tab::Automation => "7: Automation",
            Tab::Devices => "8: Devices",
        };
        let selected = app.current_tab == tab;
        lines.push(Line::from(Span::styled(
            format!(
                "{}{label:<width$}",
                if selected { "▌" } else { " " },
                width = area.width.saturating_sub(1) as usize
            ),
            if selected {
                Style::default().bg(SELECTED).fg(ACCENT).bold()
            } else {
                Style::default().fg(MUTED)
            },
        )));
    }
    frame.render_widget(Paragraph::new(lines), area.inner(Margin::new(0, 1)));
    if area.height >= 21 {
        let bottom = Rect::new(
            area.x + 1,
            area.bottom() - 6,
            area.width.saturating_sub(2),
            5,
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    &app.actor.display_name,
                    Style::default().fg(TEXT).bold(),
                )),
                Line::from(Span::styled(
                    format!("[{}]", app.actor.role),
                    Style::default().fg(ACCENT),
                )),
                Line::from(""),
                Line::from(Span::styled("u  Switch user", Style::default().fg(MUTED))),
                Line::from(Span::styled("? Help  q Quit", Style::default().fg(MUTED))),
            ]),
            bottom,
        );
    }
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    let context = match app.current_tab {
        Tab::Items if app.show_deleted => "d Delete forever  R Restore  v Active inventory",
        Tab::Items if app.actor.role.can_admin() => "n Add  e Edit  d Delete  v Deleted  / Search",
        Tab::Items => "n Add  e Edit  r Receive  i Issue  / Search",
        Tab::Locations => "n Storage  e Edit  d Delete  v Deleted  m Manage",
        Tab::Gridfinity => "n Base  b Bin  e Edit  d Delete  m Manage",
        _ => "n Add  r Receive  i Issue  m Manage records",
    };
    let context = if app.actor.role.can_admin() {
        format!("{context}  U People")
    } else {
        context.to_string()
    };
    let (status, color) = match app.notification.as_ref() {
        Some(n) => (
            n.message.as_str(),
            match n.level {
                NotificationLevel::Error => ERROR,
                NotificationLevel::Warning => WARNING,
                NotificationLevel::Success => ACCENT,
                NotificationLevel::Info => MUTED,
            },
        ),
        None => ("Ready · All changes save automatically", MUTED),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(context, Style::default().fg(ACCENT))),
            Line::from(Span::styled(status, Style::default().fg(color))),
        ]),
        area,
    );
}
