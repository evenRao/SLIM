//! Consistent, keyboard-first editors with an actual cursor and inline feedback.
use super::theme::*;
use crate::app::{App, Modal, NotificationLevel, UserForm};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

struct Field<'a> {
    label: &'a str,
    value: &'a str,
    secret: bool,
}
impl<'a> Field<'a> {
    fn new(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value,
            secret: false,
        }
    }
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let Some(modal) = &app.active_modal else {
        return;
    };
    // Remove visual competition from the workspace while a form is active.
    for cell in &mut frame.buffer_mut().content {
        cell.set_fg(BORDER).set_bg(BACKGROUND);
    }
    let mut error = app
        .notification
        .as_ref()
        .filter(|n| n.level == NotificationLevel::Error)
        .map(|n| n.message.as_str());
    let (title, description, fields, focus) = match modal {
        Modal::Directory(form) => {
            render_directory(frame, form);
            return;
        }
        Modal::RecordEditor(f) => {
            error = f.error.as_deref();
            (
                if f.id.is_some() {
                    "Edit record"
                } else {
                    "Add record"
                },
                f.kind.title(),
                f.fields
                    .iter()
                    .map(|(_, label, value)| Field::new(label, value))
                    .collect::<Vec<_>>(),
                f.focused,
            )
        }
        Modal::RecordAction(f) => {
            error = f.error.as_deref();
            (
                match f.action {
                    "purge" => "Permanently delete record · Admin",
                    "restore" => "Restore record · Admin",
                    _ => "Delete record · Admin",
                },
                match f.action {
                    "purge" => "Cannot be undone. Type DELETE PERMANENTLY.",
                    "restore" => "Type RESTORE to return this record to active use.",
                    _ => "Type DELETE to move this record to Deleted.",
                },
                vec![Field::new(&f.name, &f.confirmation)],
                0,
            )
        }
        Modal::People(form) => {
            render_people(frame, form);
            return;
        }
        Modal::RemoveUser(f) => {
            error = f.error.as_deref();
            (
                if f.permanent {
                    "Permanently delete user · Admin"
                } else {
                    "Remove user · Admin"
                },
                if f.permanent {
                    "Type DELETE PERMANENTLY. Erases account; past activity is anonymized."
                } else {
                    "Type REMOVE to deactivate this account. Past activity is kept."
                },
                vec![Field::new(&f.name, &f.confirmation)],
                0,
            )
        }
        Modal::Delete(f) => {
            error = f.error.as_deref();
            (
                if f.permanent {
                    "Permanently delete item · Admin"
                } else if f.restore {
                    "Restore item · Admin"
                } else {
                    "Delete item · Admin"
                },
                if f.permanent {
                    "Cannot be undone. Erases item and history. Type DELETE PERMANENTLY."
                } else if f.restore {
                    "Type RESTORE to return this item to active inventory."
                } else {
                    "Type DELETE to remove this item. History stays; Admin can restore it."
                },
                vec![Field::new(&f.name, &f.confirmation)],
                0,
            )
        }
        Modal::Catalog(f) => {
            error = f.error.as_deref();
            (
                f.title,
                f.context
                    .as_deref()
                    .unwrap_or("Fill in the details below. Changes save together."),
                f.fields
                    .iter()
                    .map(|v| Field {
                        label: v.label,
                        value: &v.value,
                        secret: v.secret,
                    })
                    .collect::<Vec<_>>(),
                f.focused_field,
            )
        }
        Modal::Receive(f) => (
            "Receive Stock",
            "Add incoming stock to a storage location.",
            vec![
                Field::new("Item · SKU, name, or UUID", &f.item),
                Field::new("Destination location", &f.location),
                Field::new("Quantity", &f.quantity),
                Field::new("Reason · optional", &f.reason),
            ],
            f.focused_field,
        ),
        Modal::Issue(f) => (
            "Issue Stock",
            "Record items used or removed from storage.",
            vec![
                Field::new("Item · SKU, name, or UUID", &f.item),
                Field::new("Source location", &f.location),
                Field::new("Quantity", &f.quantity),
                Field::new("Reason · optional", &f.reason),
            ],
            f.focused_field,
        ),
        Modal::Transfer(f) => (
            "Transfer Stock",
            "Move items between locations. Total stock stays the same.",
            vec![
                Field::new("Item · SKU, name, or UUID", &f.item),
                Field::new("From location", &f.from_location),
                Field::new("To location", &f.to_location),
                Field::new("Quantity", &f.quantity),
                Field::new("Reason · optional", &f.reason),
            ],
            f.focused_field,
        ),
        Modal::Adjust(f) => (
            "Adjust Stock",
            "Set a physical count. A reason is required.",
            vec![
                Field::new("Item · SKU, name, or UUID", &f.item),
                Field::new("Location", &f.location),
                Field::new("New absolute quantity", &f.new_quantity),
                Field::new("Reason · required", &f.reason),
            ],
            f.focused_field,
        ),
        Modal::Search(f) => (
            "Find in your workspace",
            "Search by item name, SKU, or storage location.",
            vec![Field::new("Search", &f.query)],
            0,
        ),
        Modal::Scan(f) => (
            "Scan a label",
            "Use your USB scanner, paste a code, or type it below.",
            vec![Field::new("Barcode / QR / SKU", &f.input)],
            0,
        ),
        Modal::User(form) => {
            render_user(frame, form, app.edit_cursor);
            return;
        }
        Modal::Help => {
            render_help(frame);
            return;
        }
    };
    let suggestions = app.field_suggestions();
    let hint = if suggestions.is_empty() {
        "←→ Move cursor   Home/End   Ctrl+U Clear field".into()
    } else {
        format!(
            "F6 Complete: {}",
            suggestions
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("  ·  ")
        )
    };
    render_editor(
        frame,
        title,
        description,
        &fields,
        focus,
        app.edit_cursor,
        error,
        &hint,
    );
}

fn dialog(frame: &mut Frame, width: u16, height: u16) -> Rect {
    let full = frame.area();
    let width = width.min(full.width.saturating_sub(4));
    let height = height.min(full.height.saturating_sub(2));
    let area = Rect::new(
        full.x + (full.width - width) / 2,
        full.y + (full.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(panel(""), area);
    area.inner(Margin::new(2, 1))
}

#[allow(clippy::too_many_arguments)]
fn render_editor(
    frame: &mut Frame,
    title: &str,
    description: &str,
    fields: &[Field],
    focus: usize,
    cursor: Option<usize>,
    error: Option<&str>,
    hint: &str,
) {
    let area = dialog(frame, 78, (fields.len() as u16 * 3 + 8).max(14));
    let regions = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(title, Style::default().fg(TEXT).bold())),
            Line::from(Span::styled(description, Style::default().fg(MUTED))),
        ]),
        regions[0],
    );
    let visible = usize::from(regions[1].height / 3).max(1);
    let start = focus.saturating_sub(visible - 1);
    for (row, (index, field)) in fields
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let y = regions[1].y + row as u16 * 3;
        let field_area = Rect::new(
            area.x,
            y,
            area.width,
            3.min(regions[1].bottom().saturating_sub(y)),
        );
        render_field(frame, field_area, field, index == focus, cursor);
    }
    let footer = vec![
        Line::from(Span::styled(
            format!(
                "Field {} of {}   ·   Tab Next   Enter Save   Esc Cancel",
                focus + 1,
                fields.len()
            ),
            Style::default().fg(ACCENT),
        )),
        Line::from(Span::styled(
            error.unwrap_or(hint),
            Style::default().fg(if error.is_some() { ERROR } else { MUTED }),
        )),
    ];
    frame.render_widget(
        Paragraph::new(footer).wrap(Wrap { trim: false }),
        regions[2],
    );
}

fn render_field(
    frame: &mut Frame,
    area: Rect,
    field: &Field,
    focused: bool,
    cursor: Option<usize>,
) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            field.label,
            Style::default().fg(if focused { ACCENT } else { MUTED }),
        ))),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let input = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    let chars: Vec<char> = if field.secret {
        "•".repeat(field.value.chars().count()).chars().collect()
    } else {
        field.value.chars().collect()
    };
    let at = cursor.unwrap_or(chars.len()).min(chars.len());
    let width = usize::from(area.width.saturating_sub(3));
    let start = if focused {
        at.saturating_sub(width.saturating_sub(1))
    } else {
        0
    };
    let before: String = chars
        .iter()
        .skip(start)
        .take(at.saturating_sub(start))
        .collect();
    let after: String = chars
        .iter()
        .skip(at + usize::from(at < chars.len()))
        .collect();
    let line = if focused {
        Line::from(vec![
            Span::raw(" "),
            Span::raw(before),
            Span::styled(
                chars.get(at).copied().unwrap_or(' ').to_string(),
                Style::default().bg(ACCENT).fg(BACKGROUND),
            ),
            Span::raw(after),
        ])
    } else {
        Line::from(format!(" {}", chars.iter().collect::<String>()))
    };
    frame.render_widget(
        Paragraph::new(line)
            .style(
                Style::default()
                    .bg(if focused { SELECTED } else { BACKGROUND })
                    .fg(TEXT),
            )
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(if focused { ACCENT } else { BORDER })),
            ),
        input,
    );
}

fn render_user(frame: &mut Frame, form: &UserForm, cursor: Option<usize>) {
    let area = dialog(frame, 66, 21);
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Sign in to your workspace",
                Style::default().fg(TEXT).bold(),
            )),
            Line::from(Span::styled(
                "Choose your account, then enter your PIN.",
                Style::default().fg(MUTED),
            )),
        ]),
        sections[0],
    );
    let items: Vec<_> = form
        .actors
        .iter()
        .map(|a| ListItem::new(format!(" {}  [{}]", a.display_name, a.role)))
        .collect();
    let mut state = ListState::default().with_selected(Some(form.selected_idx));
    frame.render_stateful_widget(
        List::new(items)
            .style(Style::default().fg(MUTED))
            .highlight_style(Style::default().bg(SELECTED).fg(ACCENT).bold())
            .highlight_symbol("▌"),
        sections[1],
        &mut state,
    );
    render_field(
        frame,
        sections[2],
        &Field {
            label: "PIN · leave blank for accounts without a PIN",
            value: &form.pin,
            secret: true,
        },
        form.focused_field == 1,
        cursor,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "↑↓ Choose account   Tab PIN   Enter Sign in   Esc Cancel",
                Style::default().fg(ACCENT),
            )),
            Line::from(Span::styled(
                form.error
                    .as_deref()
                    .unwrap_or("Your inventory stays private until you sign in."),
                Style::default().fg(if form.error.is_some() { ERROR } else { MUTED }),
            )),
        ])
        .wrap(Wrap { trim: false }),
        sections[3],
    );
}

fn render_people(frame: &mut Frame, form: &crate::people::PeopleForm) {
    let area = dialog(frame, 80, 23);
    let regions = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(4),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "People · Admin",
                Style::default().fg(TEXT).bold(),
            )),
            Line::from(Span::styled(
                "Manage accounts, permissions, and PINs.",
                Style::default().fg(MUTED),
            )),
        ]),
        regions[0],
    );
    let items: Vec<_> = form
        .users
        .iter()
        .map(|a| {
            ListItem::new(format!(
                " {}   [{}]  {}",
                a.display_name,
                a.role,
                if a.active { "Active" } else { "Removed" }
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(form.selected));
    frame.render_stateful_widget(
        List::new(items)
            .style(Style::default().fg(MUTED))
            .highlight_style(Style::default().bg(SELECTED).fg(ACCENT).bold())
            .highlight_symbol("▌"),
        regions[1],
        &mut state,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "n Add  e Edit  p PIN  d Remove/delete forever  r Reactivate",
                Style::default().fg(ACCENT),
            )),
            Line::from("↑↓ Select account   Enter Edit   Esc Close"),
            Line::from(Span::styled(
                form.error
                    .as_deref()
                    .unwrap_or("Removed users cannot sign in. History is preserved."),
                Style::default().fg(if form.error.is_some() { ERROR } else { MUTED }),
            )),
        ])
        .wrap(Wrap { trim: false }),
        regions[2],
    );
}

fn render_directory(frame: &mut Frame, form: &crate::management::Directory) {
    let area = dialog(frame, 84, 24);
    let sections = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(4),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    "{} · {}",
                    form.kind.title(),
                    if form.deleted { "Deleted" } else { "Active" }
                ),
                Style::default().fg(TEXT).bold(),
            )),
            Line::from("←→ Category: Storage / Bases / Bins / Rules / Devices / Tags"),
            Line::from("Custom storage works without Gridfinity. Dimensions are optional."),
        ])
        .wrap(Wrap { trim: false }),
        sections[0],
    );
    let items = form
        .records
        .iter()
        .map(|r| ListItem::new(r.name.clone()))
        .collect::<Vec<_>>();
    let mut selection =
        ListState::default().with_selected((!form.records.is_empty()).then_some(form.selected));
    frame.render_stateful_widget(
        List::new(items)
            .style(Style::default().fg(MUTED))
            .highlight_style(Style::default().bg(SELECTED).fg(ACCENT).bold())
            .highlight_symbol("▌ "),
        sections[1],
        &mut selection,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                if form.deleted {
                    "d Delete permanently  R Restore  v Active"
                } else {
                    "n Add  e Edit  d Delete  v Deleted"
                },
                Style::default().fg(ACCENT),
            )),
            Line::from("↑↓ Select   Enter Edit   Esc Close"),
            Line::from(Span::styled(
                form.error
                    .as_deref()
                    .unwrap_or("Deletion and restoration require Admin access."),
                Style::default().fg(MUTED),
            )),
        ])
        .wrap(Wrap { trim: false }),
        sections[2],
    );
}

fn render_help(frame: &mut Frame) {
    let area = dialog(frame, 86, 22);
    let text = [
        "Keyboard Shortcuts",
        "",
        "1–8 / Tab     Switch pages    ↑↓ / j,k Select",
        "n / e         Add / edit on the current page",
        "m             Manage storage, bases, bins, rules, devices, tags",
        "d / v / R     Delete / deleted items / restore (Admin)",
        "b / p         Add bin / place bin on Gridfinity",
        "r / i / t / a Receive / issue / transfer / adjust",
        "/ / s         Search / scan a label",
        "u / U         Switch user / manage people (Admin)",
        "F5            Create a backup beside the database",
        "F6            Complete suggested item or location",
        "l             Locate selected bin; ←→ selects bin",
        "c / x         Confirm / reject device observation",
        "",
        "Forms: Tab / Shift+Tab fields · Enter save · Esc cancel",
        "←→ / Home/End Move cursor · Ctrl+U clear field",
        "? closes help · q quits outside forms",
    ];
    frame.render_widget(
        Paragraph::new(text.join("\n"))
            .style(Style::default().fg(TEXT))
            .wrap(Wrap { trim: false }),
        area,
    );
}
