//! Immich Mount for GNOME: sign-in and status for the Immich drive in Files (ARCHITECTURE.md, "The desktop app").
//!
//! One instance per session (`dev.hviid.ImmichMount`), so the browser's redirect to `app.immich:///oauth-callback`
//! reaches the process that holds the sign-in's PKCE verifier.

mod preferences;
mod window;

use std::sync::OnceLock;

use adw::prelude::*;
use gtk::{gio, glib};
use immich_mount_linux::i18n::tr;

const APP_ID: &str = "dev.hviid.ImmichMount";

/// The non-affiliation notice (SPEC.md), in English here and translated where it is shown.
const NOTICE: &str = "Immich Mount is an independent, unofficial project. It is not affiliated with, endorsed by, or \
                      sponsored by the Immich project or FUTO. “Immich” is used only to describe the server this \
                      software works with, and remains the mark of its owner.";

/// The runtime the Immich client runs on. GTK owns the main thread; network work runs here, awaited from the main
/// loop.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("the async runtime starts")
    })
}

fn main() -> glib::ExitCode {
    // `immich-mount app` asks whether this interface loads here, before choosing it: reaching main is the answer.
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        return glib::ExitCode::SUCCESS;
    }
    bind_translations();
    let application = adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    application.connect_activate(|application| window_of(application).present());
    application.connect_open(|application, files, _hint| {
        let window = window_of(application);
        for file in files {
            window.handle_redirect(&file.uri());
        }
        window.present();
    });
    let about = gio::ActionEntry::builder("about").activate(|application: &adw::Application, _, _| show_about(application)).build();
    let quit = gio::ActionEntry::builder("quit").activate(|application: &adw::Application, _, _| application.quit()).build();
    let preferences = gio::ActionEntry::builder("preferences")
        .activate(|application: &adw::Application, _, _| window_of(application).show_preferences())
        .build();
    let shortcuts = gio::ActionEntry::builder("shortcuts").activate(|application: &adw::Application, _, _| show_shortcuts(application)).build();
    application.add_action_entries([about, quit, preferences, shortcuts]);
    for (action, keys) in SHORTCUTS.iter().map(|(action, _, keys)| (action, keys)) {
        application.set_accels_for_action(action, &[*keys]);
    }
    application.run()
}

/// Point the C library's gettext, which GTK uses for the Blueprint's texts, at the catalogs `tr` reads (ARCHITECTURE.md,
/// "Translations"). GTK sets the locale itself when it starts.
fn bind_translations() {
    use std::ffi::{c_char, CString};
    extern "C" {
        fn bindtextdomain(domain: *const c_char, directory: *const c_char) -> *mut c_char;
        fn bind_textdomain_codeset(domain: *const c_char, codeset: *const c_char) -> *mut c_char;
    }
    let Some(directory) = immich_mount_linux::i18n::locale_directory() else { return };
    let (Ok(domain), Ok(directory)) = (CString::new(immich_mount_linux::i18n::DOMAIN), CString::new(directory.as_os_str().as_encoded_bytes()))
    else {
        return;
    };
    // SAFETY: both are NUL-terminated strings that live across the calls, which copy them.
    unsafe {
        bindtextdomain(domain.as_ptr(), directory.as_ptr());
        bind_textdomain_codeset(domain.as_ptr(), c"UTF-8".as_ptr());
    }
}

/// The app's keyboard shortcuts: its action, what it does, and its keys. The dialog lists exactly these.
const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("app.preferences", "Preferences", "<Control>comma"),
    ("app.shortcuts", "Keyboard Shortcuts", "<Control>question"),
    ("app.quit", "Quit", "<Control>q"),
];

fn show_shortcuts(application: &adw::Application) {
    let section = adw::ShortcutsSection::new(None);
    for (_, title, keys) in SHORTCUTS {
        section.add(adw::ShortcutsItem::new(tr(title), keys));
    }
    let dialog = adw::ShortcutsDialog::new();
    dialog.add(section);
    dialog.present(application.active_window().as_ref());
}

fn window_of(application: &adw::Application) -> window::Window {
    application.active_window().and_downcast::<window::Window>().unwrap_or_else(|| window::Window::new(application))
}

fn show_about(application: &adw::Application) {
    let about = adw::AboutDialog::builder()
        .application_name("Immich Mount")
        .application_icon("dev.hviid.ImmichMount")
        .developer_name("Jakob Hviid")
        .version(env!("CARGO_PKG_VERSION"))
        .comments(format!("{}\n\n{}", tr("Your Immich photos and videos as a drive in Files."), tr(NOTICE)))
        .website("https://jakob.hviid.phd/immich-mount/")
        .license_type(gtk::License::MitX11)
        .build();
    about.add_link(tr("Source Code"), "https://github.com/jakobhviid/immich-mount");
    about.add_credit_section(Some(tr("Inspired By")), &["Kartax/immich-desktop-app https://github.com/Kartax/immich-desktop-app"]);
    about.add_credit_section(Some(tr("The Immich API Layer")), &["immich-rs https://github.com/boul2gom/immich-rs"]);
    about.add_legal_section(
        tr("AI Disclosure"),
        None,
        // Custom, since libadwaita shows a legal section's text only with it; Unknown left the heading alone.
        gtk::License::Custom,
        Some(tr("Parts of this project were written with the assistance of AI coding agents. All changes are reviewed by the maintainer.")),
    );
    about.present(application.active_window().as_ref());
}
