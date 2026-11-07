//! The preferences dialog, generated from the core's settings table, the same one behind `immich-mount configure`
//! (ARCHITECTURE.md, "The desktop app"). The two can never disagree about which settings exist or what is valid.
//!
//! Each change is written to `config.toml` at once, comments kept. The daemon reads its settings at start, so it is
//! restarted when the dialog closes, if anything changed and it is running.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use immich_mount_core::settings::{Kind, Setting};
use immich_mount_desktop::preferences::{change, choice_label, current_value, restart_daemon, settings_in, title, GROUPS};
use immich_mount_linux::i18n::tr;

/// Build the dialog. `report` shows a refusal to the user.
pub fn dialog(report: impl Fn(&str) + Clone + 'static) -> adw::PreferencesDialog {
    let changed = Rc::new(Cell::new(false));
    let page = adw::PreferencesPage::new();
    for (prefix, heading) in GROUPS {
        let group = adw::PreferencesGroup::builder().title(tr(heading)).build();
        for setting in settings_in(prefix) {
            let current = current_value(setting);
            group.add(&row(setting, &current, changed.clone(), report.clone()));
        }
        page.add(&group);
    }
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);
    dialog.connect_closed(move |_| {
        if changed.get() {
            restart_daemon();
        }
    });
    dialog
}

fn row(setting: &'static Setting, current: &str, changed: Rc<Cell<bool>>, report: impl Fn(&str) + 'static) -> gtk::Widget {
    let key = setting.key;
    match setting.kind {
        Kind::Bool => {
            let row = adw::SwitchRow::builder().title(title(setting)).active(current == "true").build();
            row.connect_active_notify(move |row| match change(key, if row.is_active() { "true" } else { "false" }) {
                Ok(restart) => changed.set(changed.get() || restart),
                Err(message) => report(&message),
            });
            row.upcast()
        }
        Kind::Choice(words) => {
            let labels: Vec<&str> = words.iter().map(|word| choice_label(word)).collect();
            let row = adw::ComboRow::builder().title(title(setting)).model(&gtk::StringList::new(&labels)).build();
            row.set_selected(words.iter().position(|word| *word == current).unwrap_or(0) as u32);
            row.connect_selected_notify(move |row| {
                let Some(word) = words.get(row.selected() as usize) else { return };
                match change(key, word) {
                    Ok(restart) => changed.set(changed.get() || restart),
                    Err(message) => report(&message),
                }
            });
            row.upcast()
        }
        Kind::Text | Kind::Path | Kind::Size => {
            let row = adw::EntryRow::builder().title(title(setting)).text(current).show_apply_button(true).build();
            row.connect_apply(move |row| match change(key, row.text().trim()) {
                Ok(restart) => changed.set(changed.get() || restart),
                Err(message) => report(&message),
            });
            row.upcast()
        }
    }
}
