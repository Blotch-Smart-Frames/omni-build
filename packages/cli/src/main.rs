//! The `omni-build` command line.

mod android;
mod bytes;
mod config;
mod icon;
mod ios;
mod payload;
mod zip;

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use android::key::{SigningKey, debug_key_path};
use config::App;

/// How the error for a file that's none of the shells OmniBuild knows
/// starts.
const NOT_A_SHELL: &str = "the shell isn't an APK, an AAB or an iOS app";

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
    /// The app shell: an Android APK or App Bundle (AAB), or a zipped iOS
    /// app (`.app`) built for the iOS Simulator.
    shell: PathBuf,
    /// Where to write the app: an APK, an AAB or a zipped `.app`, as the
    /// shell is.
    #[arg(long, short)]
    out: PathBuf,
    /// The app config, a JSON file: the app's `id`, `name`, `version`,
    /// `build` number, `icon` (a square PNG) and `payload` (a folder of
    /// files for the app's assets). What it leaves out stays as the shell
    /// has it.
    #[arg(long, short)]
    config: Option<PathBuf>,
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
    let app = match &args.config {
        Some(path) => App::read(path)?,
        None => App::default(),
    };
    let entries = zip::read(&shell).context(NOT_A_SHELL)?;
    let repackaged = if let Some(bundle) = ios::bundle(&entries) {
        ios::repackage(entries, &bundle, &app)?
    } else if let Some(layout) = android::layout(&entries) {
        android::repackage(entries, layout, &app, &android_debug_key()?)?
    } else {
        bail!(
            "{NOT_A_SHELL}: it has no AndroidManifest.xml or BundleConfig.pb, and no .app with an Info.plist"
        );
    };
    fs::write(&args.out, repackaged)
        .with_context(|| format!("couldn't write the app to {}", args.out.display()))
}

/// The key OmniBuild signs Android apps with when it's given none, which
/// it creates the first time.
fn android_debug_key() -> Result<SigningKey> {
    let path = debug_key_path();
    let (key, created) = SigningKey::debug(&path)?;
    if created {
        eprintln!(
            "Created a debug key for signing Android apps: {}",
            path.display()
        );
    }
    Ok(key)
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
