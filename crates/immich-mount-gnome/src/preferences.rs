//! The preferences dialog, generated from the core's settings table, the same one behind `immich-mount configure`
//! (ARCHITECTURE.md, "The desktop app"). The two can never disagree about which settings exist or what is valid.
//!
//! Each change is written to `config.toml` at once, comments kept. The daemon reads its settings at start, so it is
//! restarted when the dialog closes, if anything changed and it is running.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use immich_mount_core::settings::{Kind, Setting};
use immich_mount_desktop::preferences::{
    caption, change, choice_label, crosslink_apps, current_value, forget_app, link, restart_daemon, settings_in, show_app, title, apps_heading, GROUPS,
};
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
            if let Some(action_row) = widget.downcast_ref::<adw::ActionRow>() {
                if let Some(page) = link(setting) {
                    let button = gtk::LinkButton::with_label(page, tr("Read more"));
                    // The link reads Read more and nothing else: GTK would show the address as its tooltip.
                    button.set_has_tooltip(false);
                    button.set_valign(gtk::Align::Center);
                    action_row.add_suffix(&button);
                }
                if let Some(caption) = caption(setting) {
                    action_row.set_subtitle(caption);
                }
            }
            group.add(&widget);
        }
        page.add(&group);
        if *prefix == "crosslink" {
            page.add(&apps_group(changed.clone(), report.clone()));
        }
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

/// The apps that take part in Immich Crosslink, by their names and icons, each with its switch; one not installed now
/// is marked so, with a generic icon and a button that forgets it (Immich Crosslink, "Participants that come and go").
fn apps_group(changed: Rc<Cell<bool>>, report: impl Fn(&str) + Clone + 'static) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(apps_heading()).build();
    let apps = crosslink_apps();
    if apps.is_empty() {
        group.add(&adw::ActionRow::builder().title(tr("No other app that takes part is installed.")).build());
    }
    for (listed, shown) in apps {
        let app = listed.app;
        let app_row = adw::SwitchRow::builder().title(app.name.as_str()).active(shown).build();
        // An app without an icon, or not installed, gets the generic one, so every name lines up.
        let icon = app.icon.as_deref().filter(|_| listed.installed).unwrap_or("application-x-executable");
        app_row.add_prefix(&gtk::Image::from_icon_name(icon));
        if !listed.installed {
            app_row.set_subtitle(tr("Not installed"));
            let forget = gtk::Button::with_label(tr("Forget"));
            forget.set_valign(gtk::Align::Center);
            let (id, group, row_to_remove, changed, report) = (app.id.clone(), group.clone(), app_row.clone(), changed.clone(), report.clone());
            forget.connect_clicked(move |_| match forget_app(&id) {
                Ok(restart) => {
                    changed.set(changed.get() || restart);
                    group.remove(&row_to_remove);
                }
                Err(message) => report(&message),
            });
            app_row.add_suffix(&forget);
        }
        let (changed, report) = (changed.clone(), report.clone());
        app_row.connect_active_notify(move |app_row| match show_app(&app.id, app_row.is_active()) {
            Ok(restart) => changed.set(changed.get() || restart),
            Err(message) => report(&message),
        });
        group.add(&app_row);
    }
    group
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
