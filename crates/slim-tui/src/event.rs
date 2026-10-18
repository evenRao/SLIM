//! Event processing and keybinding dispatcher for SLIM TUI.

use crate::app::{App, Modal, Tab};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Process a single keyboard event and update application state accordingly.
pub fn handle_key_event(app: &mut App, key: KeyEvent) {
    if key.kind == KeyEventKind::Release {
        return;
    }
    if app.authenticated {
        if let Err(error) = app.revalidate_actor() {
            app.authenticated = false;
            app.open_user_modal();
            app.set_notification(crate::app::NotificationLevel::Error, error.to_string());
            return;
        }
        if !app.authenticated {
            return;
        }
    }
    if key.kind == KeyEventKind::Repeat
        && app.active_modal.is_none()
        && !matches!(
            key.code,
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k')
        )
    {
        return;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    if app.active_modal.is_some() {
        if key.code == KeyCode::F(6) {
            if let Some(suggestion) = app.field_suggestions().into_iter().next()
                && let Some(input) = app.active_modal.as_mut().and_then(modal_input)
            {
                *input = suggestion;
                app.edit_cursor = None;
            }
            return;
        }
        handle_modal_key_event(app, key);
    } else {
        handle_normal_key_event(app, key);
    }
}

impl App<'_> {
    /// Context-sensitive completion uses stable catalog values and full paths.
    pub fn field_suggestions(&self) -> Vec<String> {
        let (item_field, input) = match self.active_modal.as_ref() {
            Some(Modal::Receive(f)) if f.focused_field < 2 => (
                f.focused_field == 0,
                if f.focused_field == 0 {
                    &f.item
                } else {
                    &f.location
                },
            ),
            Some(Modal::Issue(f)) if f.focused_field < 2 => (
                f.focused_field == 0,
                if f.focused_field == 0 {
                    &f.item
                } else {
                    &f.location
                },
            ),
            Some(Modal::Adjust(f)) if f.focused_field < 2 => (
                f.focused_field == 0,
                if f.focused_field == 0 {
                    &f.item
                } else {
                    &f.location
                },
            ),
            Some(Modal::Transfer(f)) if f.focused_field < 3 => (
                f.focused_field == 0,
                match f.focused_field {
                    0 => &f.item,
                    1 => &f.from_location,
                    _ => &f.to_location,
                },
            ),
            _ => return Vec::new(),
        };
        let query = input.trim().to_lowercase();
        if item_field {
            self.items
                .iter()
                .filter(|p| {
                    p.item.name.to_lowercase().contains(&query)
                        || p.item
                            .sku
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&query)
                })
                .take(5)
                .map(|p| p.item.sku.clone().unwrap_or_else(|| p.item.name.clone()))
                .collect()
        } else {
            self.locations
                .iter()
                .filter(|p| p.location.can_hold_stock && p.path.to_lowercase().contains(&query))
                .take(5)
                .map(|p| p.path.clone())
                .collect()
        }
    }
}

fn handle_normal_key_event(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.should_quit = true;
        }
        KeyCode::Char('1') => app.switch_tab(Tab::Dashboard),
        KeyCode::Char('2') => app.switch_tab(Tab::Items),
        KeyCode::Char('3') => app.switch_tab(Tab::Locations),
        KeyCode::Char('4') => app.switch_tab(Tab::Gridfinity),
        KeyCode::Char('5') => app.switch_tab(Tab::LowStock),
        KeyCode::Char('6') => app.switch_tab(Tab::History),
        KeyCode::Char('7') => app.switch_tab(Tab::Automation),
        KeyCode::Char('8') => app.switch_tab(Tab::Devices),
        KeyCode::Tab => app.next_tab(),

        KeyCode::BackTab => app.prev_tab(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection_down(),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection_up(),
        KeyCode::Left if app.current_tab == Tab::Gridfinity => {
            app.selected_grid_bin = app.selected_grid_bin.saturating_sub(1);
        }
        KeyCode::Right if app.current_tab == Tab::Gridfinity => {
            let count = app
                .active_base_grid_map
                .as_ref()
                .map(|g| g.placed_bins.len())
                .unwrap_or(0);
            app.selected_grid_bin = (app.selected_grid_bin + 1).min(count.saturating_sub(1));
        }
        KeyCode::Char('c') => {
            if app.current_tab == Tab::Devices {
                let _ = app.confirm_selected_observation();
            }
        }
        KeyCode::Char('x') => {
            if app.current_tab == Tab::Devices {
                let _ = app.reject_selected_observation();
            }
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            if app.current_tab == Tab::Gridfinity {
                let _ = app.locate_selected_grid_bin();
            }
        }
        KeyCode::Char('r') | KeyCode::F(1) => app.open_receive_modal(),
        KeyCode::Char('i') | KeyCode::F(2) => app.open_issue_modal(),
        KeyCode::Char('t') | KeyCode::F(3) => app.open_transfer_modal(),
        KeyCode::Char('a') | KeyCode::F(4) => app.open_adjust_modal(),
        KeyCode::Char('n') => app.open_new_catalog(),
        KeyCode::Char('m') => app.open_management(),
        KeyCode::Char('e') => app.open_edit_catalog(),
        KeyCode::Char('d') | KeyCode::Delete if app.current_tab == Tab::Items => {
            if app.show_deleted {
                app.open_permanent_delete_modal()
            } else {
                app.open_delete_modal(false)
            }
        }
        KeyCode::Char('R') if app.current_tab == Tab::Items => app.open_delete_modal(true),
        KeyCode::Char('v') if app.current_tab == Tab::Items => app.toggle_deleted(),
        KeyCode::Char('d') | KeyCode::Delete
            if matches!(
                app.current_tab,
                Tab::Locations | Tab::Gridfinity | Tab::Automation | Tab::Devices
            ) =>
        {
            app.context_record_action(false)
        }
        KeyCode::Char('v')
            if matches!(
                app.current_tab,
                Tab::Locations | Tab::Gridfinity | Tab::Automation | Tab::Devices
            ) =>
        {
            app.context_record_action(true)
        }
        KeyCode::Char('b') if app.current_tab == Tab::Gridfinity => {
            app.open_catalog(crate::catalog::CatalogKind::Bin)
        }
        KeyCode::Char('p') if app.current_tab == Tab::Gridfinity => {
            app.open_catalog(crate::catalog::CatalogKind::Placement)
        }
        KeyCode::Char('U') => app.open_people_modal(),
        KeyCode::F(5) => app.create_backup(),
        KeyCode::Char('/') => app.open_search_modal(),
        KeyCode::Char('s') => app.open_scan_modal(),
        KeyCode::Char('u') => app.open_user_modal(),
        KeyCode::Char('?') => app.open_help_modal(),
        _ => {}
    }
}

fn handle_modal_key_event(app: &mut App, key: KeyEvent) {
    if key.code == KeyCode::Esc {
        if let Some(Modal::RecordAction(f)) = &app.active_modal {
            app.open_directory(f.kind, f.action != "delete");
            return;
        }
        if let Some(Modal::RecordEditor(f)) = &app.active_modal {
            app.open_directory(f.kind, false);
            return;
        }
        app.close_modal();
        return;
    }
    if matches!(app.active_modal, Some(Modal::People(_))) {
        match key.code {
            KeyCode::Char('n') => app.open_catalog(crate::catalog::CatalogKind::Person),
            KeyCode::Char('e') | KeyCode::Enter => app.open_selected_person(false),
            KeyCode::Char('p') => app.open_selected_person(true),
            KeyCode::Char('d') | KeyCode::Delete => app.open_remove_user(),
            KeyCode::Char('r') => app.restore_selected_user(),
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(Modal::People(form)) = app.active_modal.as_mut() {
                    form.selected = (form.selected + 1).min(form.users.len().saturating_sub(1));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(Modal::People(form)) = app.active_modal.as_mut() {
                    form.selected = form.selected.saturating_sub(1);
                }
            }
            _ => {}
        }
        return;
    }

    if matches!(app.active_modal, Some(Modal::Directory(_))) {
        match key.code {
            KeyCode::Left | KeyCode::BackTab => app.directory_tab(false),
            KeyCode::Right | KeyCode::Tab => app.directory_tab(true),
            KeyCode::Char('n') => app.directory_edit(true),
            KeyCode::Char('e') | KeyCode::Enter => app.directory_edit(false),
            KeyCode::Char('d') | KeyCode::Delete => app.directory_action(false),
            KeyCode::Char('R') => app.directory_action(true),
            KeyCode::Char('v') => {
                if let Some(Modal::Directory(f)) = &app.active_modal {
                    app.open_directory(f.kind, !f.deleted);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(Modal::Directory(f)) = app.active_modal.as_mut() {
                    f.selected = (f.selected + 1).min(f.records.len().saturating_sub(1));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(Modal::Directory(f)) = app.active_modal.as_mut() {
                    f.selected = f.selected.saturating_sub(1);
                }
            }
            _ => {}
        }
        return;
    }
    if key.code == KeyCode::Enter {
        app.submit_active_modal();
        return;
    }

    let modal = match app.active_modal.as_mut() {
        Some(m) => m,
        None => return,
    };
    if matches!(modal, Modal::Help) {
        if matches!(key.code, KeyCode::Char('?' | 'q')) {
            app.close_modal();
        }
        return;
    }
    if let Modal::User(form) = modal {
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            form.focused_field = (form.focused_field + 1) % 2;
            app.edit_cursor = None;
            return;
        }
        if form.focused_field == 0 {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => form.select_next(),
                KeyCode::Up | KeyCode::Char('k') => form.select_prev(),
                _ => {}
            }
            return;
        }
    }
    if matches!(
        key.code,
        KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up
    ) {
        let next = matches!(key.code, KeyCode::Tab | KeyCode::Down);
        match modal {
            Modal::Catalog(f) => f.move_field(next),
            Modal::RecordEditor(f) => {
                f.focused =
                    (f.focused + if next { 1 } else { f.fields.len() - 1 }) % f.fields.len();
            }
            Modal::Receive(f) => {
                if next {
                    f.next_field()
                } else {
                    f.prev_field()
                }
            }
            Modal::Issue(f) => {
                if next {
                    f.next_field()
                } else {
                    f.prev_field()
                }
            }
            Modal::Transfer(f) => {
                if next {
                    f.next_field()
                } else {
                    f.prev_field()
                }
            }
            Modal::Adjust(f) => {
                if next {
                    f.next_field()
                } else {
                    f.prev_field()
                }
            }
            _ => {}
        }
        app.edit_cursor = None;
        return;
    }
    if let Some(input) = modal_input(modal) {
        edit_text(input, key, &mut app.edit_cursor);
    }
}

fn modal_input(modal: &mut Modal) -> Option<&mut String> {
    match modal {
        Modal::RecordEditor(f) => Some(&mut f.fields[f.focused].2),
        Modal::RecordAction(f) => Some(&mut f.confirmation),
        Modal::RemoveUser(f) => Some(&mut f.confirmation),
        Modal::Delete(f) => Some(&mut f.confirmation),
        Modal::Catalog(f) => Some(&mut f.fields[f.focused_field].value),
        Modal::Receive(f) => Some(f.active_field_mut()),
        Modal::Issue(f) => Some(f.active_field_mut()),
        Modal::Transfer(f) => Some(f.active_field_mut()),
        Modal::Adjust(f) => Some(f.active_field_mut()),
        Modal::Search(f) => Some(&mut f.query),
        Modal::Scan(f) => Some(&mut f.input),
        Modal::User(f) if f.focused_field == 1 => Some(&mut f.pin),
        _ => None,
    }
}

fn edit_text(input: &mut String, key: KeyEvent, cursor: &mut Option<usize>) {
    let len = input.chars().count();
    let mut at = cursor.unwrap_or(len).min(len);
    let byte = |s: &str, n: usize| s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => at = 0,
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => at = len,
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.clear();
                at = 0;
            }
            _ => {}
        }
    } else {
        match key.code {
            KeyCode::Left => at = at.saturating_sub(1),
            KeyCode::Right => at = (at + 1).min(len),
            KeyCode::Home => at = 0,
            KeyCode::End => at = len,
            KeyCode::Backspace if at > 0 => {
                input.remove(byte(input, at - 1));
                at -= 1;
            }
            KeyCode::Delete if at < len => {
                input.remove(byte(input, at));
            }
            KeyCode::Char(c) if !c.is_control() => {
                input.insert(byte(input, at), c);
                at += 1;
            }
            _ => {}
        }
    }
    *cursor = Some(at);
}

pub fn handle_paste(app: &mut App, text: &str) {
    if let Some(modal) = app.active_modal.as_mut()
        && let Some(input) = modal_input(modal)
    {
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            edit_text(
                input,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                &mut app.edit_cursor,
            );
        }
    }
}
