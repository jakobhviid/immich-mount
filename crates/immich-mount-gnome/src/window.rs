//! The main window: connect to a server and sign in; then the app's pages in a sidebar, Status first, then the settings
//! and the account (ARCHITECTURE.md, "The desktop app", "One layout for every settings window").

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::future::Future;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use immich_mount_core::sign_in::BrowserSignIn;
use immich_mount_linux::i18n::tr;

use immich_mount_desktop::session::{self, ServerOffer, SignedIn, Status};

/// Cargo rebuilds when the interface changes only if the file is included somewhere it can see.
const _INTERFACE: &str = include_str!("window.blp");

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "src/window.blp")]
    pub struct Window {
        #[template_child]
        pub toasts: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub navigation: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub server_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub connect_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub browser_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub browser_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub password_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub email_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub password_row: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub api_key_row: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub split_view: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub content_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub pages: TemplateChild<gtk::Stack>,
        #[template_child]
        pub status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub server_status_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub account_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub mount_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub connection_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub sign_in_again_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub open_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub unmount_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub mount_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub photos_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub albums_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub people_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub places_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub favorites_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub archive_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub location_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub cache_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub uploads_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub email_status_row: TemplateChild<adw::ActionRow>,
        /// The server being signed in to.
        pub offer: RefCell<Option<ServerOffer>>,
        /// The browser sign-in in progress, holding its PKCE verifier until the redirect comes back.
        pub attempt: RefCell<Option<BrowserSignIn>>,
        /// Where the mount is, for "Open in Files".
        pub mount_point: RefCell<Option<std::path::PathBuf>>,
        /// A setting changed since the daemon last started: it restarts once the user leaves the page.
        pub settings_changed: Rc<Cell<bool>>,
        /// The page shown, by its sidebar id.
        pub shown_page: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "ImmichMountWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(class: &mut Self::Class) {
            class.bind_template();
            class.bind_template_callbacks();
        }

        fn instance_init(object: &glib::subclass::InitializingObject<Self>) {
            object.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl Window {
        #[template_callback]
        fn on_connect(&self, #[rest] _values: &[glib::Value]) {
            self.obj().connect_to_server();
        }

        #[template_callback]
        fn on_browser_sign_in(&self, #[rest] _values: &[glib::Value]) {
            self.obj().begin_browser_sign_in();
        }

        #[template_callback]
        fn on_password_sign_in(&self, #[rest] _values: &[glib::Value]) {
            self.obj().sign_in_with_password();
        }

        #[template_callback]
        fn on_api_key_sign_in(&self, #[rest] _values: &[glib::Value]) {
            self.obj().sign_in_with_api_key();
        }

        #[template_callback]
        fn on_sign_in_again(&self, #[rest] _values: &[glib::Value]) {
            // Sign in to the same server again; nothing is removed.
            let obj = self.obj();
            if let Some(server) = session::configured_server() {
                self.server_row.set_text(&server);
            }
            self.navigation.replace_with_tags(&["connect"]);
            obj.connect_to_server();
        }

        #[template_callback]
        fn on_setup_done(&self, #[rest] _values: &[glib::Value]) {
            self.obj().finish_setup();
        }

        #[template_callback]
        fn on_open_in_files(&self, #[rest] _values: &[glib::Value]) {
            self.obj().open_in_files();
        }

        #[template_callback]
        fn on_mount(&self, #[rest] _values: &[glib::Value]) {
            self.obj().set_mounted(true);
        }

        #[template_callback]
        fn on_unmount(&self, #[rest] _values: &[glib::Value]) {
            self.obj().set_mounted(false);
        }

        #[template_callback]
        fn on_disconnect(&self, #[rest] _values: &[glib::Value]) {
            self.obj().disconnect();
        }

        #[template_callback]
        fn on_page_selected(&self, #[rest] _values: &[glib::Value]) {
            let Some(row) = self.sidebar.selected_row() else { return };
            self.obj().open_page(&row.widget_name());
        }
    }

    impl ObjectImpl for Window {}
    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

/// Run `work` on the app's async runtime, then `then` with its result on the main loop.
fn run<Output, Work>(work: Work, then: impl FnOnce(Output) + 'static)
where
    Work: Future<Output = Output> + Send + 'static,
    Output: Send + 'static,
{
    let handle = crate::runtime().spawn(work);
    glib::spawn_future_local(async move {
        if let Ok(output) = handle.await {
            then(output);
        }
    });
}

/// The sidebar's pages: Status, the settings pages of the shared layout, and Account, each by its id, title and icon.
fn sidebar_pages() -> Vec<(&'static str, &'static str, &'static str)> {
    let settings = immich_mount_desktop::preferences::PAGES.iter().map(|page| (page.id, tr(page.title), page.icon));
    std::iter::once(("status", tr("Status"), "dev.hviid.ImmichMount-symbolic"))
        .chain(settings)
        .chain(std::iter::once(("account", tr("Account"), "avatar-default-symbolic")))
        .collect()
}

impl Window {
    pub fn new(application: &adw::Application) -> Self {
        let window: Window = glib::Object::builder().property("application", application).build();
        for (id, title, icon) in sidebar_pages() {
            let content = gtk::Box::builder().spacing(12).margin_top(6).margin_bottom(6).margin_start(6).margin_end(6).build();
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&gtk::Label::builder().label(title).xalign(0.0).build());
            let row = gtk::ListBoxRow::builder().child(&content).name(id).build();
            window.imp().sidebar.append(&row);
        }
        window.imp().sidebar.select_row(window.imp().sidebar.row_at_index(0).as_ref());
        // A change of settings reaches the daemon when the window closes, as when the user leaves its page.
        window.connect_close_request(|window| {
            window.apply_settings();
            glib::Propagation::Proceed
        });
        if let Some(server) = session::configured_server() {
            window.imp().server_row.set_text(&server);
        }
        let shown = window.clone();
        run(session::current_status(), move |status| {
            if let Some(status) = status {
                shown.show_status(&status);
            }
        });
        // Keep the status current while it is on screen: connection, cache and mount change underneath.
        let watched = window.downgrade();
        glib::timeout_add_seconds_local(5, move || {
            let Some(window) = watched.upgrade() else { return glib::ControlFlow::Break };
            let on_status = window.imp().navigation.visible_page().and_then(|page| page.tag()).is_some_and(|tag| tag == "status");
            if on_status && window.is_visible() {
                let refreshed = window.clone();
                run(session::current_status(), move |status| {
                    if let Some(status) = status {
                        refreshed.update_status(&status);
                    }
                });
            }
            glib::ControlFlow::Continue
        });
        window
    }

    fn toast(&self, message: &str) {
        self.imp().toasts.add_toast(adw::Toast::new(message));
    }

    fn connect_to_server(&self) {
        let imp = self.imp();
        let address = session::normalised_address(&imp.server_row.text());
        imp.connect_button.set_sensitive(false);
        let window = self.clone();
        run(session::probe(address), move |outcome| {
            let imp = window.imp();
            imp.connect_button.set_sensitive(true);
            match outcome {
                Ok(offer) => {
                    match &offer.browser_button {
                        Some(label) => imp.browser_button.set_label(label),
                        None => imp.browser_group.set_visible(false),
                    }
                    imp.browser_group.set_visible(offer.browser_button.is_some());
                    imp.password_group.set_visible(offer.password);
                    imp.offer.replace(Some(offer));
                    imp.navigation.push_by_tag("sign-in");
                }
                Err(message) => window.toast(&message),
            }
        });
    }

    fn server_url(&self) -> Option<String> {
        self.imp().offer.borrow().as_ref().map(|offer| offer.url.clone())
    }

    fn begin_browser_sign_in(&self) {
        let Some(url) = self.server_url() else { return };
        let attempt = match BrowserSignIn::new() {
            Ok(attempt) => attempt,
            Err(error) => return self.toast(&error.to_string()),
        };
        let (state, challenge) = (attempt.state.clone(), attempt.code_challenge.clone());
        self.imp().attempt.replace(Some(attempt));
        let window = self.clone();
        run(session::begin_browser_sign_in(url, state, challenge), move |outcome| match outcome {
            Ok(address) => {
                claim_redirect();
                gtk::UriLauncher::new(&address).launch(Some(&window), gio::Cancellable::NONE, |_| {});
                window.toast(tr("Continue in your browser"));
            }
            Err(message) => window.toast(&message),
        });
    }

    /// The browser came back with `app.immich:///oauth-callback?…`, through the scheme handler.
    pub fn handle_redirect(&self, address: &str) {
        // A redirect for an older attempt (a double click) must not use up the live one.
        let belongs = self.imp().attempt.borrow().as_ref().is_some_and(|attempt| immich_mount_core::sign_in::is_redirect_for(address, &attempt.state));
        let (Some(url), true, Some(attempt)) = (self.server_url(), belongs, if belongs { self.imp().attempt.take() } else { None }) else {
            return self.toast(tr("This sign-in belongs to an earlier attempt. Finish the latest one, or sign in again."));
        };
        let window = self.clone();
        self.set_busy(true);
        run(session::finish_browser_sign_in(url, address.to_owned(), attempt), move |outcome| window.finish_sign_in(outcome));
    }

    fn sign_in_with_password(&self) {
        let Some(url) = self.server_url() else { return };
        let imp = self.imp();
        let (email, password) = (imp.email_row.text().to_string(), imp.password_row.text().to_string());
        if email.trim().is_empty() || password.is_empty() {
            return self.toast(tr("Enter your email and password"));
        }
        self.set_busy(true);
        let window = self.clone();
        run(session::sign_in_with_password(url, email, password), move |outcome| window.finish_sign_in(outcome));
    }

    fn sign_in_with_api_key(&self) {
        let Some(url) = self.server_url() else { return };
        let key = self.imp().api_key_row.text().to_string();
        if key.trim().is_empty() {
            return self.toast(tr("Enter an API key"));
        }
        self.set_busy(true);
        let window = self.clone();
        run(session::sign_in_with_api_key(url, key), move |outcome| window.finish_sign_in(outcome));
    }

    /// While a sign-in runs, its buttons cannot be pressed again (a second press would open a second server session).
    fn set_busy(&self, busy: bool) {
        self.imp().navigation.set_sensitive(!busy);
    }

    /// Keep the credential, then offer the first-run choices.
    fn finish_sign_in(&self, outcome: Result<SignedIn, String>) {
        self.set_busy(false);
        let signed_in = match outcome {
            Ok(signed_in) => signed_in,
            Err(message) => return self.toast(&message),
        };
        let Some(url) = self.server_url() else { return };
        self.imp().password_row.set_text("");
        self.imp().api_key_row.set_text("");
        let account = signed_in.account.clone();
        let window = self.clone();
        run(session::save(url, signed_in.credential), move |outcome| match outcome {
            Ok(_) => {
                if let Some(account) = account {
                    window.toast(&tr("Signed in as {account}").replace("{account}", &account));
                }
                window.show_setup();
            }
            Err(message) => window.toast(&message),
        });
    }

    /// The first-run choices, filled in from the configuration.
    fn show_setup(&self) {
        let imp = self.imp();
        let setup = session::current_setup();
        imp.photos_switch.set_active(setup.photos);
        imp.albums_switch.set_active(setup.albums);
        imp.people_switch.set_active(setup.people);
        imp.places_switch.set_active(setup.places);
        imp.favorites_switch.set_active(setup.favorites);
        imp.archive_switch.set_active(setup.archive);
        imp.location_row.set_text(&setup.location);
        imp.navigation.push_by_tag("setup");
    }

    /// Store the choices, start the mount, and open it in Files once it is up. `systemctl` runs off the main loop.
    fn finish_setup(&self) {
        let imp = self.imp();
        let setup = session::Setup {
            photos: imp.photos_switch.is_active(),
            albums: imp.albums_switch.is_active(),
            people: imp.people_switch.is_active(),
            places: imp.places_switch.is_active(),
            favorites: imp.favorites_switch.is_active(),
            archive: imp.archive_switch.is_active(),
            location: imp.location_row.text().to_string(),
        };
        self.set_busy(true);
        let window = self.clone();
        run(async move { session::apply_setup(&setup) }, move |outcome| match outcome {
            Ok(()) => window.wait_for_mount(20),
            Err(message) => {
                window.set_busy(false);
                window.toast(&message);
            }
        });
    }

    /// Look once a second until the mount is up (the daemon first asks the server a few things), then show the status
    /// and open it in Files; after `seconds_left`, show the status as it is.
    fn wait_for_mount(&self, seconds_left: u32) {
        let window = self.clone();
        glib::timeout_add_seconds_local_once(1, move || {
            run(session::current_status(), move |status| match status {
                Some(status) if status.mounted => {
                    // GVfs lists no mount in a home reached through a symlink (an atomic desktop's /home): give the
                    // sidebar a bookmark instead, once.
                    let paths = immich_mount_linux::paths::Paths::from_environment();
                    if !immich_mount_linux::gnome::listed_in_sidebar(&paths, &status.mount_point) {
                        if let Err(error) = immich_mount_linux::gnome::add_bookmark(&paths, &status.mount_point, immich_mount_desktop::preferences::drive_name()) {
                            eprintln!("could not bookmark the mount in Files' sidebar: {error}");
                        }
                    }
                    window.set_busy(false);
                    window.show_status(&status);
                    window.open_in_files();
                }
                Some(status) if seconds_left == 0 => {
                    window.set_busy(false);
                    window.show_status(&status);
                }
                None if seconds_left == 0 => {
                    window.set_busy(false);
                    window.toast(tr("Signed in, but the status could not be read"));
                }
                _ => window.wait_for_mount(seconds_left - 1),
            });
        });
    }

    fn show_status(&self, status: &Status) {
        self.update_status(status);
        self.imp().navigation.replace_with_tags(&["status"]);
    }

    /// Fill in the status page without changing which page is shown.
    fn update_status(&self, status: &Status) {
        let imp = self.imp();
        imp.server_status_row.set_subtitle(&status.server_text());
        imp.email_status_row.set_visible(status.account_text().is_some());
        imp.email_status_row.set_subtitle(&status.account_text().unwrap_or_default());
        imp.uploads_row.set_visible(status.uploads_text().is_some());
        imp.uploads_row.set_subtitle(&status.uploads_text().unwrap_or_default());
        imp.account_row.set_subtitle(&status.credential_place);
        imp.mount_row.set_subtitle(&status.mount_point.display().to_string());
        imp.connection_row.set_subtitle(&status.connection_text());
        imp.sign_in_again_group.set_visible(status.daemon.as_ref().is_some_and(|report| report.state == "signed-out"));
        imp.cache_row.set_subtitle(&status.cache_text(immich_mount_desktop::preferences::format_size));
        let signed_out = status.daemon.as_ref().is_some_and(|report| report.state == "signed-out");
        let (title, description) = match (signed_out, status.mounted) {
            (true, _) => (tr("Signed Out of Immich"), tr("Sign in again to keep browsing. Your caches are kept.")),
            (false, true) => (tr("Your Library Is in Files"), tr("Browse it like any folder. Originals download only when you open them.")),
            (false, false) => (tr("Not Mounted"), tr("Mount it to browse your library in Files.")),
        };
        imp.open_button.set_visible(status.mounted);
        imp.unmount_button.set_visible(status.mounted);
        imp.mount_button.set_visible(!status.mounted);
        imp.status_page.set_title(title);
        imp.status_page.set_description(Some(description));
        imp.mount_point.replace(Some(status.mount_point.clone()));
    }

    /// Show the first settings page, General, as Preferences in the menus and Ctrl+, ask.
    pub fn show_preferences(&self) {
        self.select_page("general");
    }

    /// Select a page in the sidebar, which opens it.
    pub fn select_page(&self, id: &str) {
        let sidebar = &self.imp().sidebar;
        let mut index = 0;
        while let Some(row) = sidebar.row_at_index(index) {
            if row.widget_name() == id {
                sidebar.select_row(Some(&row));
                self.imp().split_view.set_show_content(true);
                return;
            }
            index += 1;
        }
    }

    /// Open a page: a settings page is drawn anew, so it shows the settings as they are now. Leaving one where
    /// something changed restarts the daemon, which reads its settings at start.
    fn open_page(&self, id: &str) {
        let imp = self.imp();
        if *imp.shown_page.borrow() != id {
            self.apply_settings();
        }
        imp.shown_page.replace(id.to_owned());
        if let Some(page) = immich_mount_desktop::preferences::PAGES.iter().find(|page| page.id == id) {
            if let Some(old) = imp.pages.child_by_name(id) {
                imp.pages.remove(&old);
            }
            let window = self.clone();
            let context = crate::preferences::Context {
                changed: imp.settings_changed.clone(),
                report: Rc::new(move |message: &str| window.toast(message)),
            };
            let drawn = crate::preferences::page(page, &context, self.upcast_ref());
            imp.pages.add_named(&drawn, Some(id));
        }
        imp.pages.set_visible_child_name(id);
        let title = sidebar_pages().into_iter().find(|(page, _, _)| *page == id).map_or("", |(_, title, _)| title);
        imp.content_page.set_title(title);
        imp.split_view.set_show_content(true);
    }

    /// Restart the daemon if a setting changed, so it reads them again.
    fn apply_settings(&self) {
        if self.imp().settings_changed.replace(false) {
            immich_mount_desktop::preferences::restart_daemon();
        }
    }

    fn open_in_files(&self) {
        let Some(mount_point) = self.imp().mount_point.borrow().clone() else { return };
        gtk::FileLauncher::new(Some(&gio::File::for_path(mount_point))).launch(Some(self), gio::Cancellable::NONE, |_| {});
    }

    /// Mount or unmount, off the main loop, then show the status as it is.
    fn set_mounted(&self, mounted: bool) {
        self.set_busy(true);
        let window = self.clone();
        run(async move { session::set_mounted(mounted) }, move |outcome| {
            if let Err(message) = outcome {
                window.toast(&message);
            }
            let refreshed = window.clone();
            run(session::current_status(), move |status| {
                refreshed.set_busy(false);
                if let Some(status) = status {
                    refreshed.update_status(&status);
                }
            });
        });
    }

    fn disconnect(&self) {
        let window = self.clone();
        run(async { session::disconnect() }, move |outcome| match outcome {
            Ok(()) => {
                window.imp().navigation.replace_with_tags(&["connect"]);
                window.toast(tr("Disconnected"));
            }
            Err(message) => window.toast(&message),
        });
    }
}

/// Make the app the handler of `app.immich`, so the browser's redirect comes back to it (`immich-mount app` hands it
/// to this interface outside Plasma).
fn claim_redirect() {
    let desktop_id = format!("{}.desktop", crate::APP_ID);
    if let Some(info) = gio::AppInfo::all().into_iter().find(|info| info.id().as_deref() == Some(desktop_id.as_str())) {
        if let Err(error) = info.set_as_default_for_type("x-scheme-handler/app.immich") {
            eprintln!("could not make {desktop_id} the app.immich handler: {error}");
        }
    }
}
