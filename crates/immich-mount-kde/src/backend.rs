//! The object the QML pages talk to: one `Backend` with the state each page shows, and the actions behind its
//! buttons. Everything it does is `immich-mount-desktop`, the same code as the GNOME app; network work runs on the
//! async runtime and comes back to the Qt thread through `qt_thread().queue`.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, page)]
        #[qproperty(bool, busy)]
        #[qproperty(QString, message)]
        #[qproperty(QString, server)]
        #[qproperty(QString, sso)]
        #[qproperty(bool, passwords)]
        #[qproperty(QString, address)]
        #[qproperty(QString, connection)]
        #[qproperty(QString, cache)]
        #[qproperty(QString, location)]
        #[qproperty(QString, credential)]
        #[qproperty(bool, signedout)]
        #[qproperty(bool, mounted)]
        #[qproperty(bool, photos)]
        #[qproperty(bool, albums)]
        #[qproperty(bool, people)]
        #[qproperty(bool, places)]
        #[qproperty(bool, previews)]
        #[qproperty(QString, version)]
        #[qproperty(QString, uploads)]
        #[qproperty(QString, serverline)]
        #[qproperty(QString, account)]
        type Backend = super::BackendRust;

        /// Check a server and offer its sign-in methods.
        #[qinvokable]
        #[cxx_name = "checkServer"]
        fn check_server(self: Pin<&mut Backend>, typed: &QString);

        /// Start single sign-on; `address` then holds the provider's page to open in the browser.
        #[qinvokable]
        #[cxx_name = "signInBrowser"]
        fn sign_in_browser(self: Pin<&mut Backend>);

        #[qinvokable]
        #[cxx_name = "signInPassword"]
        fn sign_in_password(self: Pin<&mut Backend>, email: &QString, password: &QString);

        #[qinvokable]
        #[cxx_name = "signInKey"]
        fn sign_in_key(self: Pin<&mut Backend>, key: &QString);

        /// Store the first-run choices and start the mount.
        #[qinvokable]
        #[cxx_name = "finishSetup"]
        fn finish_setup(self: Pin<&mut Backend>);

        /// Read the status again (the page refreshes it every few seconds).
        #[qinvokable]
        fn refresh(self: Pin<&mut Backend>);

        /// Mount, or unmount as an eject does.
        #[qinvokable]
        #[cxx_name = "switchMount"]
        fn switch_mount(self: Pin<&mut Backend>, mounted: bool);

        #[qinvokable]
        fn disconnect(self: Pin<&mut Backend>);

        /// Let Dolphin preview remote files, this mount included, or go back to KDE's default of none.
        #[qinvokable]
        #[cxx_name = "allowPreviews"]
        fn allow_previews(self: Pin<&mut Backend>, allow: bool);

        /// The preferences page's rows as JSON, in the settings table's order: a section header for each group, then
        /// its settings with their kind, description, current value and choices.
        #[qinvokable]
        #[cxx_name = "preferenceRows"]
        fn preference_rows(self: &Backend) -> QString;

        /// Store one setting from the preferences page: the refusal's message, or nothing.
        #[qinvokable]
        #[cxx_name = "storeSetting"]
        fn store_setting(self: Pin<&mut Backend>, key: &QString, value: &QString) -> QString;

        /// The preferences page closed: the daemon restarts if a setting it reads changed.
        #[qinvokable]
        #[cxx_name = "preferencesClosed"]
        fn preferences_closed(self: Pin<&mut Backend>);

        /// The mount came up after the first-run setup: QML opens it in the file manager.
        #[qsignal]
        #[cxx_name = "mountReady"]
        fn mount_ready(self: Pin<&mut Backend>);

        /// Back from sign-in to the server's address, or from waiting for the browser to sign-in (the attempt is
        /// dropped, so a late redirect finds nothing).
        #[qinvokable]
        #[cxx_name = "goBack"]
        fn go_back(self: Pin<&mut Backend>);

        /// Sign in to the same server again, caches kept.
        #[qinvokable]
        #[cxx_name = "signInAgain"]
        fn sign_in_again(self: Pin<&mut Backend>);
    }

    impl cxx_qt::Threading for Backend {}
    impl cxx_qt::Initialize for Backend {}
}

use core::pin::Pin;
use std::future::Future;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use immich_mount_core::settings::Kind;
use immich_mount_desktop::preferences;
use immich_mount_desktop::session::{self, ServerOffer, SignedIn, Status};
use immich_mount_linux::kde;
use immich_mount_linux::paths::Paths;

#[derive(Default)]
pub struct BackendRust {
    page: QString,
    busy: bool,
    message: QString,
    server: QString,
    sso: QString,
    passwords: bool,
    address: QString,
    connection: QString,
    cache: QString,
    location: QString,
    credential: QString,
    signedout: bool,
    mounted: bool,
    photos: bool,
    albums: bool,
    people: bool,
    places: bool,
    previews: bool,
    version: QString,
    uploads: QString,
    serverline: QString,
    account: QString,
    /// The server being signed in to.
    url: String,
    /// A preference the daemon reads at start changed since the page opened.
    settings_changed: bool,
    /// The sign-in under way follows "Sign In Again", so its setup is not a first run.
    signing_in_again: bool,
}

/// Run `work` on the async runtime, then `then` on the Qt thread with its result.
fn run<Output, Work>(backend: Pin<&mut qobject::Backend>, work: Work, then: impl FnOnce(Pin<&mut qobject::Backend>, Output) + Send + 'static)
where
    Work: Future<Output = Output> + Send + 'static,
    Output: Send + 'static,
{
    let qt_thread = backend.qt_thread();
    crate::runtime().spawn(async move {
        let output = work.await;
        let _ = qt_thread.queue(move |backend| then(backend, output));
    });
}

fn format_bytes(bytes: u64) -> String {
    let gigabytes = bytes as f64 / 1e9;
    if gigabytes >= 1.0 { format!("{gigabytes:.1} GB") } else { format!("{:.1} MB", bytes as f64 / 1e6) }
}

impl cxx_qt::Initialize for qobject::Backend {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_page(QString::from("connect"));
        self.as_mut().set_version(QString::from(env!("CARGO_PKG_VERSION")));
        if let Some(server) = session::configured_server() {
            self.as_mut().set_server(QString::from(&server));
        }
        self.as_mut().refresh();
    }
}

impl qobject::Backend {
    fn fail(mut self: Pin<&mut Self>, message: &str) {
        self.as_mut().set_busy(false);
        self.as_mut().set_message(QString::from(message));
    }

    pub fn check_server(mut self: Pin<&mut Self>, typed: &QString) {
        // Pressing Enter in a field while the last request is still out asks again; one is enough.
        if *self.busy() {
            return;
        }
        let address = session::normalised_address(&typed.to_string());
        self.as_mut().set_busy(true);
        self.as_mut().set_message(QString::default());
        run(self, session::probe(address), |mut backend, outcome: Result<ServerOffer, String>| match outcome {
            Ok(offer) => {
                backend.as_mut().set_busy(false);
                backend.as_mut().set_sso(QString::from(offer.browser_button.as_deref().unwrap_or("")));
                backend.as_mut().set_passwords(offer.password);
                backend.as_mut().set_server(QString::from(&offer.url));
                backend.as_mut().rust_mut().url = offer.url;
                backend.as_mut().set_page(QString::from("signin"));
            }
            Err(message) => backend.fail(&message),
        });
    }

    pub fn sign_in_browser(mut self: Pin<&mut Self>) {
        if *self.busy() {
            return;
        }
        let url = self.rust().url.clone();
        let attempt = match immich_mount_core::sign_in::BrowserSignIn::new() {
            Ok(attempt) => attempt,
            Err(error) => return self.fail(&error.to_string()),
        };
        if let Err(message) = session::remember_attempt(&url, &attempt) {
            return self.fail(&message);
        }
        self.as_mut().set_busy(true);
        let (state, challenge) = (attempt.state.clone(), attempt.code_challenge.clone());
        let work = async move {
            crate::claim_redirect();
            session::begin_browser_sign_in(url, state, challenge).await
        };
        run(self, work, |mut backend, outcome: Result<String, String>| match outcome {
            Ok(address) => {
                // QML opens it, and the redirect comes back to a second instance of the app (main.rs), which
                // finishes the sign-in; this one hears how it went on its next refresh.
                backend.as_mut().set_busy(false);
                backend.as_mut().set_address(QString::from(&address));
                backend.as_mut().set_page(QString::from("waiting"));
            }
            Err(message) => backend.fail(&message),
        });
    }

    pub fn sign_in_password(mut self: Pin<&mut Self>, email: &QString, password: &QString) {
        if *self.busy() {
            return;
        }
        let (url, email, password) = (self.rust().url.clone(), email.to_string(), password.to_string());
        self.as_mut().set_busy(true);
        run(self, async move { finish(url.clone(), session::sign_in_with_password(url, email, password).await).await }, after_sign_in);
    }

    pub fn sign_in_key(mut self: Pin<&mut Self>, key: &QString) {
        if *self.busy() {
            return;
        }
        let (url, key) = (self.rust().url.clone(), key.to_string());
        self.as_mut().set_busy(true);
        run(self, async move { finish(url.clone(), session::sign_in_with_api_key(url, key).await).await }, after_sign_in);
    }

    pub fn finish_setup(mut self: Pin<&mut Self>) {
        if *self.busy() {
            return;
        }
        // Dolphin's place is added on a first run, not after signing in again: a place the user removed stays removed.
        let first_run = !std::mem::take(&mut self.as_mut().rust_mut().signing_in_again);
        let setup = session::Setup {
            photos: *self.photos(),
            albums: *self.albums(),
            people: *self.people(),
            places: *self.places(),
            location: self.location().to_string(),
        };
        self.as_mut().set_busy(true);
        let work = async move {
            session::apply_setup(&setup)?;
            // The daemon first asks the server a few things: look once a second, for up to 20 seconds.
            let mut status = None;
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                status = session::current_status().await;
                if status.as_ref().is_some_and(|status| status.mounted) {
                    break;
                }
            }
            // Dolphin lists the mount nowhere by itself: give it a place, once, as "Add to Places" would.
            if let Some(status) = status.as_ref().filter(|_| first_run) {
                if let Err(error) = kde::add_place(&Paths::from_environment(), &status.mount_point, preferences::drive_name()) {
                    eprintln!("could not add the mount to Dolphin's Places: {error}");
                }
            }
            Ok(status)
        };
        run(self, work, |mut backend, outcome: Result<Option<Status>, String>| match outcome {
            Ok(status) => {
                backend.as_mut().set_busy(false);
                backend.as_mut().set_page(QString::from("status"));
                if let Some(status) = status {
                    backend.as_mut().show_status(&status);
                    if status.mounted {
                        backend.as_mut().mount_ready();
                    }
                }
            }
            Err(message) => backend.fail(&message),
        });
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        if self.page().to_string() == "waiting" {
            // The browser's redirect starts a second instance (main.rs), which finishes the sign-in and reports here.
            match session::attempt_outcome() {
                Some(Ok(())) => self.show_setup(),
                Some(Err(message)) => {
                    self.as_mut().set_page(QString::from("signin"));
                    self.fail(&message);
                }
                None => {}
            }
            return;
        }
        run(self, session::current_status(), |mut backend, status: Option<Status>| {
            let Some(status) = status else {
                return;
            };
            backend.as_mut().show_status(&status);
            if backend.page().to_string() == "connect" {
                backend.as_mut().set_page(QString::from("status"));
            }
        });
    }

    fn show_setup(mut self: Pin<&mut Self>) {
        let setup = session::current_setup();
        self.as_mut().set_photos(setup.photos);
        self.as_mut().set_albums(setup.albums);
        self.as_mut().set_people(setup.people);
        self.as_mut().set_places(setup.places);
        self.as_mut().set_location(QString::from(&setup.location));
        self.as_mut().read_previews();
        self.as_mut().set_busy(false);
        self.as_mut().set_message(QString::default());
        self.as_mut().set_page(QString::from("setup"));
    }

    fn show_status(mut self: Pin<&mut Self>, status: &Status) {
        // The address alone, as sign-in again uses it; the page shows `server_text` beside it.
        self.as_mut().set_server(QString::from(&status.server));
        self.as_mut().set_serverline(QString::from(&status.server_text()));
        self.as_mut().set_uploads(QString::from(&status.uploads_text().unwrap_or_default()));
        self.as_mut().set_account(QString::from(&status.account_text().unwrap_or_default()));
        self.as_mut().set_credential(QString::from(&status.credential_place));
        self.as_mut().set_location(QString::from(&status.mount_point.display().to_string()));
        self.as_mut().set_mounted(status.mounted);
        self.as_mut().read_previews();
        let (connection, cache, signed_out) = match &status.daemon {
            Some(report) => (
                match report.state.as_str() {
                    "online" => "Online".to_owned(),
                    "offline" => "Server unreachable; files you opened before stay available".to_owned(),
                    "signed-out" => "Signed out; sign in again".to_owned(),
                    other => other.to_owned(),
                },
                format!("{} of {}", format_bytes(report.cache_used), format_bytes(report.cache_limit)),
                report.state == "signed-out",
            ),
            None => ("Not running".to_owned(), "Not running".to_owned(), false),
        };
        self.as_mut().set_connection(QString::from(&connection));
        self.as_mut().set_cache(QString::from(&cache));
        self.as_mut().set_signedout(signed_out);
    }

    fn read_previews(self: Pin<&mut Self>) {
        let limit = kde::remote_preview_limit(&Paths::from_environment()).unwrap_or(0);
        self.set_previews(limit > 0);
    }

    pub fn allow_previews(mut self: Pin<&mut Self>, allow: bool) {
        match kde::allow_remote_previews(allow) {
            Ok(()) => self.as_mut().set_previews(allow),
            Err(message) => {
                self.as_mut().read_previews();
                self.fail(&message);
            }
        }
    }

    pub fn preference_rows(&self) -> QString {
        let mut rows = Vec::new();
        for (prefix, title) in preferences::GROUPS {
            rows.push(serde_json::json!({ "kind": "section", "title": title }));
            for setting in preferences::settings_in(prefix) {
                let (kind, choices) = match setting.kind {
                    Kind::Bool => ("bool", Vec::new()),
                    Kind::Choice(words) => (
                        "choice",
                        words.iter().map(|word| serde_json::json!({ "word": word, "label": preferences::choice_label(word) })).collect(),
                    ),
                    Kind::Text | Kind::Path | Kind::Size => ("text", Vec::new()),
                };
                rows.push(serde_json::json!({
                    "kind": kind,
                    "key": setting.key,
                    "title": setting.description,
                    "value": preferences::current_value(setting),
                    "choices": choices,
                }));
            }
        }
        QString::from(&serde_json::Value::Array(rows).to_string())
    }

    pub fn store_setting(mut self: Pin<&mut Self>, key: &QString, value: &QString) -> QString {
        match preferences::change(&key.to_string(), &value.to_string()) {
            Ok(restart) => {
                self.as_mut().rust_mut().settings_changed |= restart;
                QString::default()
            }
            Err(message) => QString::from(&message),
        }
    }

    pub fn preferences_closed(mut self: Pin<&mut Self>) {
        if std::mem::take(&mut self.as_mut().rust_mut().settings_changed) {
            preferences::restart_daemon();
        }
    }

    pub fn switch_mount(mut self: Pin<&mut Self>, mounted: bool) {
        self.as_mut().set_busy(true);
        run(self, async move { session::set_mounted(mounted) }, |mut backend, outcome: Result<(), String>| {
            match outcome {
                Ok(()) => {
                    backend.as_mut().set_busy(false);
                    backend.as_mut().set_message(QString::default());
                }
                Err(message) => backend.as_mut().fail(&message),
            }
            backend.refresh();
        });
    }

    pub fn disconnect(mut self: Pin<&mut Self>) {
        self.as_mut().set_busy(true);
        run(self, async { session::disconnect() }, |mut backend, outcome: Result<(), String>| match outcome {
            Ok(()) => {
                backend.as_mut().set_busy(false);
                backend.as_mut().set_page(QString::from("connect"));
            }
            Err(message) => backend.fail(&message),
        });
    }

    pub fn go_back(mut self: Pin<&mut Self>) {
        if self.page().to_string() == "waiting" {
            session::forget_attempt();
            self.as_mut().set_page(QString::from("signin"));
        } else {
            self.as_mut().rust_mut().signing_in_again = false;
            self.as_mut().set_page(QString::from("connect"));
        }
    }

    pub fn sign_in_again(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().signing_in_again = true;
        let server = self.server().clone();
        self.as_mut().check_server(&server);
    }
}

/// Keep the credential (keyring, or the private file), then report.
async fn finish(url: String, signed_in: Result<SignedIn, String>) -> Result<(), String> {
    let signed_in = signed_in?;
    session::save(url, signed_in.credential).await.map(|_| ())
}

fn after_sign_in(backend: Pin<&mut qobject::Backend>, outcome: Result<(), String>) {
    match outcome {
        Ok(()) => {
            // A browser sign-in started earlier must not replace this one when its redirect arrives late.
            session::forget_attempt();
            backend.show_setup()
        }
        Err(message) => backend.fail(&message),
    }
}
