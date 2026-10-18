//! The `omni-build` command line.

mod android;
mod bytes;
mod zip;

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use android::key::{SigningKey, debug_key_path};

/// Build, sign, install and ship iOS and Android apps with no native toolchain.
#[derive(Debug, Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Repackage(Repackage),
}

/// Turn a prebuilt app shell into an app, and sign it.
#[derive(Debug, Args)]
struct Repackage {
    /// The app shell: an Android APK.
    shell: PathBuf,
    /// Where to write the app.
    #[arg(long, short)]
    out: PathBuf,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Repackage(args) => repackage(&args),
    }
}

fn repackage(args: &Repackage) -> Result<()> {
    let shell = fs::read(&args.shell)
        .with_context(|| format!("couldn't read the shell {}", args.shell.display()))?;
    let key_path = debug_key_path();
    let (key, created) = SigningKey::debug(&key_path)?;
    if created {
        eprintln!(
            "Created a debug key for signing Android apps: {}",
            key_path.display()
        );
    }
    let app = android::repackage(&shell, &key)?;
    fs::write(&args.out, app)
        .with_context(|| format!("couldn't write the app to {}", args.out.display()))
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        // clap checks a command's definition (conflicting flags, duplicate
        // names) only when it's built, so a mistake would otherwise surface
        // as a panic the first time a user runs that command.
        Cli::command().debug_assert();
    }
}
