//! The preferences dialog, generated from the core's settings table, the same one behind `immich-mount configure`
//! (ARCHITECTURE.md, "The desktop app"). The two can never disagree about which settings exist or what is valid.
//!
//! Each change is written to `config.toml` at once, comments kept. The daemon reads its settings at start, so it is
//! restarted when the dialog closes, if anything changed and it is running.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use immich_mount_core::settings::{Kind, Setting};
use immich_mount_desktop::preferences::{change, choice_label, crosslink_apps, current_value, link, restart_daemon, settings_in, show_app, title, GROUPS};
use immich_mount_linux::i18n::tr;

/// Build the dialog. `report` shows a refusal to the user.
pub fn dialog(report: impl Fn(&str) + Clone + 'static) -> adw::PreferencesDialog {
    let changed = Rc::new(Cell::new(false));
    let page = adw::PreferencesPage::new();
    for (prefix, heading) in GROUPS {
        let group = adw::PreferencesGroup::builder().title(tr(heading)).build();
        for setting in settings_in(prefix) {
            let current = current_value(setting);
            let widget = row(setting, &current, changed.clone(), report.clone());
            if let (Some(page), Some(action_row)) = (link(setting), widget.downcast_ref::<adw::ActionRow>()) {
                let button = gtk::LinkButton::with_label(page, tr("What is this?"));
                button.set_valign(gtk::Align::Center);
                action_row.add_suffix(&button);
            }
            group.add(&widget);
        }
        if *prefix == "crosslink" {
            for (app, shown) in crosslink_apps() {
                let app_row = adw::SwitchRow::builder().title(app.name.as_str()).active(shown).build();
                // An app without an icon gets the generic one, so every name lines up.
                app_row.add_prefix(&gtk::Image::from_icon_name(app.icon.as_deref().unwrap_or("application-x-executable")));
                let (changed, report) = (changed.clone(), report.clone());
                app_row.connect_active_notify(move |app_row| match show_app(&app.id, app_row.is_active()) {
                    Ok(restart) => changed.set(changed.get() || restart),
                    Err(message) => report(&message),
                });
                group.add(&app_row);
            }
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
