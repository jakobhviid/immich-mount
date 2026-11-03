//! `immich-mount`: the daemon and command line of Immich Mount.
//!
//! A thin layer. Every command resolves its inputs, calls one function, and renders the typed result: for people on
//! stdout, or as one JSON document with `--json`. Progress and errors go to stderr. `--llm` and `--man` are answered
//! before argument parsing, from the leading arguments only, so they never steal a subcommand's own arguments.

mod app;
mod commands;
mod completions;
mod online;
mod ui;

use clap::{CommandFactory, Parser, Subcommand};
use immich_mount_linux::doctor::Status;
use immich_mount_linux::paths::Paths;
use serde::Serialize;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "immich-mount",
    version,
    about = "Your Immich photos and videos as a drive in your file manager",
    after_help = "Docs: https://github.com/jakobhviid/immich-mount\nThe full guide, for people and LLMs: immich-mount --llm"
)]
pub(crate) struct Cli {
    /// Print the command reference and all of the project's docs, for an LLM or a curious human
    #[arg(long)]
    llm: bool,

    /// Write one JSON document to stdout instead of human output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the app, in the interface native to this desktop (KDE's on Plasma, GNOME's elsewhere). Running
    /// `immich-mount` with no command does the same where there is a display
    App {
        /// Handed to the app, such as the sign-in redirect the browser returns
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        arguments: Vec<String>,
    },
    /// Sign in to an Immich server and remember it. Without a flag, sign in through the server's OIDC provider in
    /// your browser: it prints an address to open, then reads the address the browser ends on
    Login {
        /// Sign in with email and password (prompted on a terminal, else read from stdin, one per line)
        #[arg(long, conflicts_with = "api_key")]
        password: bool,
        /// Sign in with an API key (prompted on a terminal, else read from stdin)
        #[arg(long)]
        api_key: bool,
        /// The server's address, such as https://photos.example.org (default: the configured one)
        #[arg(long, value_name = "URL")]
        server: Option<String>,
    },
    /// Show the connection, the server, the mount and the cache
    Status,
    /// Run the daemon in the foreground (what the user service starts)
    Daemon,
    /// Read and change settings
    Configure {
        #[command(subcommand)]
        action: ConfigureAction,
    },
    /// Check that everything the mount needs is in place (changes nothing)
    Doctor,
    /// Mount the drive: starts the user service and waits for the mount
    Mount,
    /// Unmount the drive (a dead mount is detached)
    Unmount,
    /// Remove the credential and every cache (settings are kept)
    Logout,
    /// Print shell completions
    Completions {
        /// The shell to generate completions for
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum ConfigureAction {
    /// Every setting with its effective value
    List,
    /// The names of all settings
    Keys,
    /// One setting's value
    Get { key: String },
    /// Change a setting
    Set { key: String, value: String },
    /// Return a setting to its default
    Unset { key: String },
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().collect();
    if is_leading_flag(&arguments, "--man") {
        return finish(completions::print_man());
    }
    if is_leading_flag(&arguments, "--llm") {
        print!("{}", llm_guide());
        return ExitCode::SUCCESS;
    }

    let cli = Cli::parse();
    let paths = Paths::from_environment();
    let json = cli.json;
    match cli.command {
        Some(Command::Login { password, api_key, server }) => finish(run_login(&paths, password, api_key, server, json)),
        Some(Command::Status) => finish(online::status(&paths).map(|report| {
            if json {
                print_json(&report);
            } else {
                let server = match &report.server {
                    Some(server) => format!("{}{}", server.url, server.version.as_deref().map(|version| format!(" (Immich {version})")).unwrap_or_default()),
                    None => "(none)".to_string(),
                };
                println!("state:      {}", report.state);
                if let Some(detail) = &report.detail {
                    println!("            {detail}");
                }
                println!("server:     {server}");
                if let Some(email) = &report.account.email {
                    println!("account:    {email}");
                }
                println!("credential: {}", report.account.credential.unwrap_or("(none)"));
                let mounted = if report.mount.mounted { "mounted" } else if report.mount.stale { "stale; run immich-mount unmount" } else { "not mounted" };
                println!("mount:      {} ({mounted})", report.mount.path.display());
                let mebibytes = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
                println!("cache:      {:.1} MiB of {:.0} MiB", mebibytes(report.cache.used_bytes), mebibytes(report.cache.limit_bytes));
            }
        })),
        Some(Command::Daemon) => {
            ui::init_log();
            finish(online::daemon(&paths).map(|_| ()))
        }
        Some(Command::Configure { action }) => finish(run_configure(&paths, action, json)),
        Some(Command::Doctor) => finish(commands::doctor(&paths).map(|report| {
            if json {
                print_json(&report);
            } else {
                for check in &report.checks {
                    let glyph = match check.status {
                        Status::Ok => "✓",
                        Status::Warning => "⚠",
                        Status::Problem => "✗",
                        Status::Skipped => "·",
                    };
                    println!("{glyph} {}: {}", check.name, check.detail);
                    if let Some(fix) = &check.fix {
                        println!("    {fix}");
                    }
                }
                println!("{}", headline(report.problems, report.warnings));
            }
        })),
        Some(Command::Mount) => finish(commands::mount(&paths).map(|report| {
            if json {
                print_json(&report);
            } else if report.was_mounted {
                println!("{} was already mounted", report.mount_point.display());
            } else {
                println!("mounted at {}", report.mount_point.display());
            }
        })),
        Some(Command::Unmount) => finish(commands::unmount(&paths).map(|report| {
            if json {
                print_json(&report);
            } else {
                println!("{} ({:?} before)", report.mount_point.display(), report.was);
            }
        })),
        Some(Command::Logout) => finish(commands::logout(&paths, online::revoke_session(&paths)).map(|report| {
            if json {
                print_json(&report);
            } else if report.removed.is_empty() && report.seeded_thumbnails_removed == 0 && !report.unmounted && !report.session_ended {
                println!("Nothing to remove");
            } else {
                if report.session_ended {
                    println!("ended the session on the server");
                }
                if report.unmounted {
                    println!("unmounted");
                }
                if report.seeded_thumbnails_removed > 0 {
                    println!("removed {} seeded thumbnail(s)", report.seeded_thumbnails_removed);
                }
                for path in &report.removed {
                    println!("removed {}", path.display());
                }
            }
        })),
        Some(Command::Completions { shell }) => {
            completions::print_completions(shell);
            ExitCode::SUCCESS
        }
        Some(Command::App { arguments }) => finish(Err(app::open(&arguments))),
        // The app first; the command line where there is no display to show it on.
        None if app::has_display() => finish(Err(app::open(&[]))),
        None => finish(Cli::command().print_help().map_err(anyhow::Error::from)),
    }
}

fn run_login(paths: &Paths, password: bool, api_key: bool, server: Option<String>, json: bool) -> anyhow::Result<()> {
    let method = match (password, api_key) {
        (true, _) => online::LoginMethod::Password,
        (_, true) => online::LoginMethod::ApiKey,
        _ => online::LoginMethod::Browser,
    };
    let report = online::login(paths, method, server)?;
    if json {
        print_json(&report);
    } else {
        let user = report.user.as_deref().map(|user| format!(" as {user}")).unwrap_or_default();
        println!("✓ signed in to {} (Immich {}){user} with {}, kept in {}", report.server, report.server_version, report.method, report.stored_in);
        // A logout turns the user service off; say how to turn it on, unless it already is.
        let unit = immich_mount_linux::service::user_service();
        let enabled = std::process::Command::new("systemctl")
            .args(["--user", "is-enabled", "--quiet", unit])
            .status()
            .is_ok_and(|status| status.success());
        if !enabled {
            println!("next: `immich-mount mount` mounts it now and at every login");
        }
    }
    Ok(())
}

fn run_configure(paths: &Paths, action: ConfigureAction, json: bool) -> anyhow::Result<()> {
    match action {
        ConfigureAction::List => {
            let rows = commands::configure_list(paths)?;
            if json {
                print_json(&rows);
            } else {
                for row in rows {
                    let marker = if row.is_set { "" } else { "  (default)" };
                    println!("{} = {}{marker}", row.key, if row.value.is_empty() { "(not set)" } else { &row.value });
                }
            }
        }
        ConfigureAction::Keys => {
            let keys: Vec<&str> = immich_mount_core::settings::SETTINGS.iter().map(|setting| setting.key).collect();
            if json {
                print_json(&keys);
            } else {
                for key in keys {
                    println!("{key}");
                }
            }
        }
        ConfigureAction::Get { key } => print_row(&commands::configure_get(paths, &key)?, json),
        ConfigureAction::Set { key, value } => print_row(&commands::configure_set(paths, &key, &value)?, json),
        ConfigureAction::Unset { key } => print_row(&commands::configure_unset(paths, &key)?, json),
    }
    Ok(())
}

fn print_row(row: &commands::SettingRow, json: bool) {
    if json {
        print_json(row);
    } else {
        println!("{}", row.value);
    }
}

/// A result headline that carries failures in its text, not only in colour.
fn headline(problems: usize, warnings: usize) -> String {
    match (problems, warnings) {
        (0, 0) => "✓ all checks passed".to_string(),
        (0, warnings) => format!("⚠ {warnings} warning(s)"),
        (problems, warnings) => format!("✗ {problems} problem(s), {warnings} warning(s)"),
    }
}

fn print_json<Report: Serialize + ?Sized>(report: &Report) {
    match serde_json::to_string_pretty(report) {
        Ok(text) => println!("{text}"),
        Err(error) => ui::err(&format!("could not write JSON: {error}")),
    }
}

/// Whether `flag` appears among the leading flags, before the first subcommand or value.
fn is_leading_flag(arguments: &[String], flag: &str) -> bool {
    arguments
        .iter()
        .skip(1)
        .take_while(|argument| argument.starts_with('-'))
        .any(|argument| argument == flag)
}

fn finish(outcome: anyhow::Result<()>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            ui::err(&format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

/// The `--llm` guide: the generated command reference, then the repository's docs, embedded at compile time so
/// they can never drift from the binary.
fn llm_guide() -> String {
    let mut guide = String::from("===== COMMAND REFERENCE =====\n\n");
    let mut root = Cli::command();
    guide.push_str(&root.render_long_help().to_string());
    for subcommand in root.get_subcommands_mut().filter(|subcommand| !subcommand.is_hide_set()) {
        guide.push_str(&format!("\n\n----- immich-mount {} -----\n\n", subcommand.get_name()));
        guide.push_str(&subcommand.render_long_help().to_string());
    }
    let documents: &[(&str, &str)] = &[
        ("README", include_str!("../../../README.md")),
        ("WORKFLOWS", include_str!("../../../WORKFLOWS.md")),
        ("SPEC", include_str!("../../../SPEC.md")),
        ("ARCHITECTURE", include_str!("../../../ARCHITECTURE.md")),
        ("PRINCIPLES", include_str!("../../../PRINCIPLES.md")),
        ("ROADMAP", include_str!("../../../ROADMAP.md")),
    ];
    for (name, body) in documents {
        guide.push_str(&format!("\n\n===== {name} =====\n\n"));
        guide.push_str(body);
    }
    guide
}
