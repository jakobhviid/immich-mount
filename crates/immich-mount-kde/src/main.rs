//! Immich Mount for KDE Plasma: sign-in, status and settings for the Immich drive in Dolphin (ROADMAP.md, M6).
//!
//! Kirigami pages in QML (`qml/Main.qml`) over one Rust object (`backend.rs`), which is the same
//! `immich-mount-desktop` code as the GNOME app. Qt has no single-instance service, so the browser's redirect to
//! `app.immich:///oauth-callback` starts a second process; it finishes the sign-in from the attempt the first one
//! left in the runtime directory, and exits.

mod backend;

use std::sync::OnceLock;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use immich_mount_desktop::session;

/// The desktop entry's name, without `.desktop`: the one app's, shared with the GNOME interface (`immich-mount app`
/// chooses between them).
const DESKTOP_ID: &str = "dev.hviid.ImmichMount";

/// The runtime the Immich client runs on, beside Qt's event loop.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("the async runtime starts")
    })
}

fn main() {
    // `immich-mount app` asks whether this interface loads here, before choosing it: reaching main is the answer.
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        return;
    }
    if let Some(redirect) = std::env::args().skip(1).find(|argument| argument.starts_with("app.immich:")) {
        std::process::exit(finish_redirect(&redirect));
    }
    // KDE's own look, unless the user chose another style.
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }
    let mut application = QGuiApplication::new();
    if let Some(mut application) = application.as_mut() {
        application.as_mut().set_application_name(&QString::from("immich-mount-kde"));
        application.as_mut().set_application_display_name(&QString::from("Immich Mount"));
        application.as_mut().set_organization_domain(&QString::from("hviid.dev"));
    }
    // So Plasma matches the window to its desktop entry, and shows its icon and name.
    QGuiApplication::set_desktop_file_name(&QString::from(DESKTOP_ID));
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/dev/hviid/ImmichMount/qml/Main.qml"));
    }
    if let Some(application) = application.as_mut() {
        application.exec();
    }
}

/// Make the app the handler of `app.immich`, so the browser's redirect comes back to it (`immich-mount app` hands it
/// to this interface on Plasma).
pub fn claim_redirect() {
    let desktop_id = format!("{DESKTOP_ID}.desktop");
    match std::process::Command::new("xdg-mime").args(["default", &desktop_id, "x-scheme-handler/app.immich"]).status() {
        Ok(status) if status.success() => {}
        outcome => eprintln!("could not make {desktop_id} the app.immich handler: {outcome:?}"),
    }
}

/// Finish a browser sign-in that another instance started, and keep the credential.
fn finish_redirect(redirect: &str) -> i32 {
    let outcome = match session::take_attempt_for(redirect) {
        Some(session::Redirect::SignIn { url, attempt }) => runtime().block_on(async {
            let signed_in = session::finish_browser_sign_in(url.clone(), redirect.to_owned(), attempt).await?;
            session::save(url, signed_in.credential).await.map(|_| ())
        }),
        Some(session::Redirect::Refused(message)) => Err(message),
        None => {
            eprintln!("no sign-in is waiting for this address; start it again in Immich Mount");
            return 1;
        }
    };
    session::report_attempt(&outcome);
    match outcome {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}
