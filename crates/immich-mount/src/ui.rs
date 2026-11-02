//! Output discipline: the command's result goes to stdout; everything that narrates the run (progress, warnings,
//! errors) goes to stderr. Colour only on a terminal, and never when `NO_COLOR` is set.

use std::io::IsTerminal;
use std::sync::OnceLock;

fn color_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal())
}

fn paint(code: &str, text: &str) -> String {
    if color_enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn red(text: &str) -> String {
    paint("31", text)
}

fn yellow(text: &str) -> String {
    paint("33", text)
}

/// A warning line on stderr.
pub fn warn(message: &str) {
    eprintln!("{} {message}", yellow("warning:"));
}

/// An error line on stderr.
pub fn err(message: &str) {
    eprintln!("{} {message}", red("error:"));
}

/// The daemon's log: one line per message on stderr, which the user service sends to the journal.
/// `IMMICH_MOUNT_LOG` sets the level (`error`, `warn`, `info`, `debug` or `trace`, default `info`). Libraries only
/// get through with warnings and errors, so the journal is about the mount, not about HTTP.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let ours = metadata.target().starts_with("immich");
        metadata.level() <= log::max_level() && (ours || metadata.level() <= log::Level::Warn)
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {}", record.level().as_str().to_lowercase(), record.args());
        }
    }

    fn flush(&self) {}
}

pub fn init_log() {
    let level = std::env::var("IMMICH_MOUNT_LOG").ok().and_then(|value| value.parse().ok()).unwrap_or(log::LevelFilter::Info);
    if log::set_logger(&StderrLog).is_ok() {
        log::set_max_level(level);
    }
}
