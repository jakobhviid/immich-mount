//! The settings pages of the main window, drawn from the layout every desktop app shares (`preferences::PAGES`;
//! ARCHITECTURE.md, "One layout for every settings window"), over the same settings table as `immich-mount configure`.
//!
//! Each change is written to `config.toml` at once, comments kept. The daemon reads its settings at start, so the
//! window restarts it when the user leaves a settings page, if anything changed and it is running.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use immich_mount_core::settings::Kind;
use immich_mount_desktop::preferences::{
    apps_description, apps_heading, cache_size_label, cache_sizes, caption, change, choice_label, crosslink_apps, current_value, forget_app,
    link, setting, show_app, title, Page, Row, PARTICIPANTS_PAGE,
};
use immich_mount_linux::i18n::tr;

/// Whether a change is waiting for the daemon to restart, and where a refusal is told.
#[derive(Clone)]
pub struct Context {
    pub changed: Rc<Cell<bool>>,
    pub report: Rc<dyn Fn(&str)>,
}

impl Context {
    /// Keep a change, or tell why it was refused.
    fn outcome(&self, outcome: Result<bool, String>) {
        match outcome {
            Ok(restart) => self.changed.set(self.changed.get() || restart),
            Err(message) => (self.report)(&message),
        }
    }
}

/// One page, as the settings are now.
pub fn page(page: &'static Page, context: &Context, window: &gtk::Window) -> adw::PreferencesPage {
    let preferences_page = adw::PreferencesPage::new();
    for group in page.groups {
        let preferences_group = adw::PreferencesGroup::new();
        if let Some(heading) = group.title {
            preferences_group.set_title(tr(heading));
        }
        if let Some(description) = group.description {
            preferences_group.set_description(Some(tr(description)));
        }
        for row in group.rows {
            preferences_group.add(&widget(row, context, window));
        }
        preferences_page.add(&preferences_group);
        if group.apps {
            preferences_page.add(&apps_group(context));
        }
    }
    preferences_page
}

/// A row's widget, by the kind of its setting.
fn widget(row: &'static Row, context: &Context, window: &gtk::Window) -> gtk::Widget {
    let shown = setting(row.key);
    let current = current_value(shown);
    if let Some((inner, _)) = row.inner {
        return section_row(row, inner, context);
    }
    match shown.kind {
        Kind::Bool if link(shown).is_some() => link_switch_row(row, &current, context),
        Kind::Bool => {
            let switch_row = adw::SwitchRow::builder().title(title(shown)).active(current == "true").build();
            if let Some(line) = caption(shown) {
                switch_row.set_subtitle(line);
            }
            let (key, context) = (row.key, context.clone());
            switch_row.connect_active_notify(move |switch_row| context.outcome(change(key, if switch_row.is_active() { "true" } else { "false" })));
            switch_row.upcast()
        }
        Kind::Choice(words) => choice_row(row.key, words, context).upcast(),
        Kind::Size => cache_row(row.key, context).upcast(),
        Kind::Path => folder_row(row.key, &current, context, window).upcast(),
        Kind::Text => {
            let entry_row = adw::EntryRow::builder().title(title(shown)).text(&current).show_apply_button(true).build();
            let (key, context) = (row.key, context.clone());
            entry_row.connect_apply(move |entry_row| context.outcome(change(key, entry_row.text().trim())));
            entry_row.upcast()
        }
    }
}

/// A section: its switch, and inside, its date folders (or, for Upload, what it does with folders), which mean
/// nothing while the section is hidden, so the row only opens while it is shown.
fn section_row(row: &'static Row, inner: &'static str, context: &Context) -> gtk::Widget {
    let shown = setting(row.key);
    let expander = adw::ExpanderRow::builder()
        .title(title(shown))
        .show_enable_switch(true)
        .enable_expansion(current_value(shown) == "true")
        .build();
    if let Some(line) = caption(shown) {
        expander.set_subtitle(line);
    }
    if let Kind::Choice(words) = setting(inner).kind {
        expander.add_row(&choice_row(inner, words, context));
    }
    let (key, context) = (row.key, context.clone());
    expander.connect_enable_expansion_notify(move |expander| {
        context.outcome(change(key, if expander.enables_expansion() { "true" } else { "false" }));
    });
    expander.upcast()
}

/// A choice among words, each by its label.
fn choice_row(key: &'static str, words: &'static [&'static str], context: &Context) -> adw::ComboRow {
    let labels: Vec<&str> = words.iter().map(|word| choice_label(word)).collect();
    // A long choice ("Year in the Name (2023 - Trip)") reads whole under the title, rather than cut short beside it;
    // set before the model, so the row shows its value from the start.
    let long = labels.iter().any(|label| label.chars().count() > 20);
    let combo = adw::ComboRow::builder().title(title(setting(key))).use_subtitle(long).build();
    combo.set_list_factory(Some(&whole_labels()));
    combo.set_model(Some(&gtk::StringList::new(&labels)));
    let current = current_value(setting(key));
    combo.set_selected(words.iter().position(|word| *word == current).unwrap_or(0) as u32);
    let context = context.clone();
    combo.connect_selected_notify(move |combo| {
        let Some(word) = words.get(combo.selected() as usize) else { return };
        context.outcome(change(key, word));
    });
    combo
}

/// The list a choice drops down, its labels whole rather than cut to the row's width.
fn whole_labels() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        item.set_child(Some(&gtk::Label::builder().xalign(0.0).build()));
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let (Some(label), Some(text)) = (
            item.child().and_downcast::<gtk::Label>(),
            item.item().and_downcast::<gtk::StringObject>(),
        ) else {
            return;
        };
        label.set_label(&text.string());
    });
    factory
}

/// The cache's size: one of the sizes offered, in the units Files counts in, or one set by hand, as itself.
fn cache_row(key: &'static str, context: &Context) -> adw::ComboRow {
    let sizes = cache_sizes();
    let labels: Vec<String> = sizes.iter().map(|size| cache_size_label(size)).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let combo = adw::ComboRow::builder().title(title(setting(key))).model(&gtk::StringList::new(&label_refs)).build();
    if let Some(line) = caption(setting(key)) {
        combo.set_subtitle(line);
    }
    let current = current_value(setting(key));
    combo.set_selected(sizes.iter().position(|size| *size == current).unwrap_or(0) as u32);
    let context = context.clone();
    combo.connect_selected_notify(move |combo| {
        let Some(size) = sizes.get(combo.selected() as usize) else { return };
        context.outcome(change(key, size));
    });
    combo
}

/// A folder, chosen in the system's folder chooser rather than typed, and shown in full, as Status shows it.
fn folder_row(key: &'static str, current: &str, context: &Context, window: &gtk::Window) -> adw::ActionRow {
    let expand = |written: &str| immich_mount_linux::paths::Paths::from_environment().expand(written);
    let row = adw::ActionRow::builder().title(title(setting(key))).subtitle(expand(current).display().to_string()).build();
    row.add_css_class("property");
    let choose = gtk::Button::builder().icon_name("document-open-symbolic").valign(gtk::Align::Center).tooltip_text(tr("Choose a Folder")).build();
    choose.add_css_class("flat");
    let (shown_row, context, parent) = (row.clone(), context.clone(), window.clone());
    choose.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder().title(tr("Choose a Folder")).modal(true).build();
        // It opens where the drive is, beside it, rather than at Recent.
        if let Some(parent) = expand(&current_value(setting(key))).parent() {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(parent)));
        }
        let (shown_row, context) = (shown_row.clone(), context.clone());
        dialog.select_folder(Some(&parent), gtk::gio::Cancellable::NONE, move |chosen| {
            let Some(path) = chosen.ok().and_then(|folder| folder.path()) else { return };
            let text = immich_mount_desktop::preferences::home_relative(&path);
            match change(key, &text) {
                Ok(restart) => {
                    context.changed.set(context.changed.get() || restart);
                    shown_row.set_subtitle(&expand(&text).display().to_string());
                }
                Err(message) => (context.report)(&message),
            }
        });
    });
    row.add_suffix(&choose);
    row.set_activatable_widget(Some(&choose));
    row
}

/// Immich Crosslink's switch, with its link before it, so the switch stays last, as in every row.
fn link_switch_row(row: &'static Row, current: &str, context: &Context) -> gtk::Widget {
    let shown = setting(row.key);
    let action_row = adw::ActionRow::builder().title(title(shown)).build();
    if let Some(line) = caption(shown) {
        action_row.set_subtitle(line);
    }
    let switch = gtk::Switch::builder().active(current == "true").valign(gtk::Align::Center).build();
    action_row.add_suffix(&link_button(link(shown).unwrap_or_default(), tr("Learn More")));
    action_row.add_suffix(&switch);
    action_row.set_activatable_widget(Some(&switch));
    let (key, context) = (row.key, context.clone());
    switch.connect_active_notify(move |switch| context.outcome(change(key, if switch.is_active() { "true" } else { "false" })));
    action_row.upcast()
}

/// A link to `page` that reads `label` and nothing else: GTK would show the address as its tooltip.
fn link_button(page: &str, label: &str) -> gtk::LinkButton {
    let button = gtk::LinkButton::with_label(page, label);
    button.set_has_tooltip(false);
    button.set_valign(gtk::Align::Center);
    button
}

/// The apps that take part in Immich Crosslink, by their names and icons, each with its switch, last as in every row;
/// one not found now is marked so, with a generic icon and Forget before its switch (Immich Crosslink, "Participants
/// that come and go"). Forgetting the last one leaves the sentence that says none is installed.
fn apps_group(context: &Context) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(apps_heading()).description(apps_description()).build();
    let apps = crosslink_apps();
    if apps.is_empty() {
        group.add(&no_apps_row());
    }
    let remaining = Rc::new(Cell::new(apps.len()));
    for (listed, shown) in apps {
        let app = listed.app;
        let app_row = adw::ActionRow::builder().title(app.name.as_str()).build();
        // An app without an icon, or not found, gets the generic one, so every name lines up.
        let icon = app.icon.as_deref().filter(|_| listed.installed).unwrap_or("application-x-executable");
        app_row.add_prefix(&gtk::Image::from_icon_name(icon));
        let switch = gtk::Switch::builder().active(shown).valign(gtk::Align::Center).build();
        switch.update_property(&[gtk::accessible::Property::Label(app.name.as_str())]);
        if !listed.installed {
            app_row.set_subtitle(tr("Not found"));
            let forget = gtk::Button::with_label(tr("Forget"));
            forget.set_valign(gtk::Align::Center);
            let (id, group, row_to_remove, remaining, context) = (app.id.clone(), group.clone(), app_row.clone(), remaining.clone(), context.clone());
            forget.connect_clicked(move |_| match forget_app(&id) {
                Ok(restart) => {
                    context.changed.set(context.changed.get() || restart);
                    group.remove(&row_to_remove);
                    remaining.set(remaining.get() - 1);
                    if remaining.get() == 0 {
                        group.add(&no_apps_row());
                    }
                }
                Err(message) => (context.report)(&message),
            });
            app_row.add_suffix(&forget);
        }
        app_row.add_suffix(&switch);
        app_row.set_activatable_widget(Some(&switch));
        let context = context.clone();
        switch.connect_active_notify(move |switch| context.outcome(show_app(&app.id, switch.is_active())));
        group.add(&app_row);
    }
    group
}

/// No app takes part: a sentence saying so, and a link to the protocol's list of those that do.
fn no_apps_row() -> adw::ActionRow {
    let none = adw::ActionRow::builder().title(tr("No other app that takes part is installed.")).build();
    none.add_suffix(&link_button(PARTICIPANTS_PAGE, tr("Find Apps That Take Part")));
    none
}
