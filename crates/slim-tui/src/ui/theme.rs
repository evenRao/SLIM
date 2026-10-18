//! Shared palette and surfaces for the terminal workspace.
use ratatui::{
    style::{Color, Style},
    widgets::{Block, BorderType, Borders},
};
pub const BACKGROUND: Color = Color::Rgb(11, 18, 32);
pub const SURFACE: Color = Color::Rgb(17, 28, 45);
pub const SIDEBAR: Color = Color::Rgb(14, 24, 39);
pub const BORDER: Color = Color::Rgb(48, 67, 88);
pub const TEXT: Color = Color::Rgb(229, 238, 248);
pub const MUTED: Color = Color::Rgb(159, 179, 201);
pub const ACCENT: Color = Color::Rgb(77, 219, 184);
pub const SELECTED: Color = Color::Rgb(24, 64, 69);
pub const WARNING: Color = Color::Rgb(245, 196, 110);
pub const ERROR: Color = Color::Rgb(255, 139, 148);
pub fn panel(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", title.into()))
        .title_style(Style::default().fg(MUTED))
        .style(Style::default().bg(SURFACE).fg(TEXT))
        .border_style(Style::default().fg(BORDER))
}
