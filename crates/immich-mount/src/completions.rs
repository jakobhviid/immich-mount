//! Shell completions and the man page, both generated from the one clap definition.

use clap::CommandFactory;

use crate::Cli;

/// Write completions for `shell` to stdout.
pub fn print_completions(shell: clap_complete::Shell) {
    let mut command = Cli::command();
    clap_complete::generate(shell, &mut command, "immich-mount", &mut std::io::stdout());
}

/// Write the man page (roff) to stdout.
pub fn print_man() -> anyhow::Result<()> {
    clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
    Ok(())
}
