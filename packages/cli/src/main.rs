//! The `omni-build` command line.

mod adb;
mod android;
mod bytes;
mod config;
mod icon;
mod ios;
mod key;
mod keystore;
mod payload;
mod report;
mod zip;

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use adb::AdbKey;
use config::App;
use key::{SigningKey, debug_key_path};
use report::{Event, Format, KeyKind, Platform, Reporter};
use x509_cert::name::Name;

/// How the error for a file that's none of the shells OmniBuild knows
/// starts.
const NOT_A_SHELL: &str = "the shell isn't an APK, an AAB or an iOS app";
/// A keystore's password, from the environment rather than a flag, which
/// would show it to anyone listing the computer's processes.
const KEY_PASSWORD: &str = "OMNI_BUILD_KEY_PASSWORD";

/// Build, sign, install and ship iOS and Android apps with no native toolchain.
#[derive(Debug, Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {
    /// Report each event as a line of JSON on stdout, for build tools to
    /// read: what OmniBuild created, what it waits for, the command's
    /// result, or the error it stopped at.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Repackage(Repackage),
    Install(Install),
    Key(Key),
}

/// Turn a prebuilt app shell into an app, and sign it.
#[derive(Debug, Args)]
struct Repackage {
    /// The app shell: an Android APK or App Bundle (AAB), or an iOS app
    /// built for devices (an `.ipa`) or for the Simulator (a zipped
    /// `.app`).
    shell: PathBuf,
    /// Where to write the app: an APK, an AAB, an `.ipa` or a zipped
    /// `.app`, as the shell is.
    #[arg(long, short)]
    out: PathBuf,
    /// The app config, a JSON file: the app's `id`, `name`, `version`,
    /// `build` number, `icon` (a square PNG) and `payload` (a folder of
    /// files for the app's assets). What it leaves out stays as the shell
    /// has it.
    #[arg(long, short)]
    config: Option<PathBuf>,
    /// The key to sign with: a PKCS #12 keystore (`.p12`, or a `.jks` from
    /// keytool since Java 9), with its password in `OMNI_BUILD_KEY_PASSWORD`,
    /// or a PEM file with the private key (PKCS #8). An RSA key, with its
    /// certificate and any that issued it. An iOS build for devices needs
    /// one Apple issued a certificate for. Without it, Android apps are
    /// signed with a debug key OmniBuild creates, and iOS Simulator builds
    /// ad hoc.
    #[arg(long)]
    signing_key: Option<PathBuf>,
    /// The provisioning profile for an iOS build for devices
    /// (`.mobileprovision`), which allows the signing key's certificate and
    /// the app's ID.
    #[arg(long)]
    profile: Option<PathBuf>,
}

/// Install an APK on an Android device or emulator, over ADB, with no
/// Android SDK.
#[derive(Debug, Args)]
struct Install {
    /// The app: an APK. Google Play makes the APKs a device installs from
    /// an app bundle (AAB).
    app: PathBuf,
    /// The device to install on: an emulator, by its name (`emulator-5554`),
    /// or a device on the network, by its address and port
    /// (`192.168.1.20:5555`). Without it, the one emulator running on this
    /// computer.
    #[arg(long, short)]
    device: Option<String>,
}

/// Manage signing keys.
#[derive(Debug, Args)]
struct Key {
    #[command(subcommand)]
    command: KeyCommand,
}

#[derive(Debug, Subcommand)]
enum KeyCommand {
    Create(CreateKey),
}

/// Create an upload key for Google Play: an RSA key with a certificate of
/// its own, valid for 30 years, in a PKCS #12 keystore that keytool,
/// Android Studio and Gradle read too, under the alias "upload". Its
/// password comes from `OMNI_BUILD_KEY_PASSWORD`.
#[derive(Debug, Args)]
struct CreateKey {
    /// Where to write the keystore, which mustn't exist yet.
    keystore: PathBuf,
    /// The certificate's subject, as a distinguished name.
    #[arg(long, default_value = "CN=Upload key")]
    subject: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let reporter = Reporter::new(cli.json);
    match run(cli.command, &reporter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            reporter.report(&Event::Error {
                message: format!("{error:#}"),
            });
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command, reporter: &Reporter) -> Result<()> {
    match command {
        Command::Repackage(args) => repackage(&args, reporter),
        Command::Install(args) => install(&args, reporter),
        Command::Key(Key {
            command: KeyCommand::Create(args),
        }) => create_key(&args, reporter),
    }
}

/// The keystore password, or none.
fn key_password() -> String {
    std::env::var(KEY_PASSWORD).unwrap_or_default()
}

fn repackage(args: &Repackage, reporter: &Reporter) -> Result<()> {
    let shell = fs::read(&args.shell)
        .with_context(|| format!("couldn't read the shell {}", args.shell.display()))?;
    let app = match &args.config {
        Some(path) => App::read(path)?,
        None => App::default(),
    };
    let key = args
        .signing_key
        .as_deref()
        .map(|path| SigningKey::read(path, &key_password()))
        .transpose()?;
    let profile = args
        .profile
        .as_ref()
        .map(|path| {
            fs::read(path).with_context(|| {
                format!("couldn't read the provisioning profile {}", path.display())
            })
        })
        .transpose()?;
    let entries = zip::read(&shell).context(NOT_A_SHELL)?;
    let (repackaged, platform, format) = if let Some(bundle) = ios::bundle(&entries) {
        let format = if bundle.starts_with("Payload/") {
            Format::Ipa
        } else {
            Format::App
        };
        let repackaged = ios::repackage(entries, &bundle, &app, key.as_ref(), profile.as_deref())?;
        (repackaged, Platform::Ios, format)
    } else if let Some(layout) = android::layout(&entries) {
        if profile.is_some() {
            bail!("a provisioning profile is for iOS apps, and the shell is an Android app");
        }
        let key = match key {
            Some(key) => key,
            None => android_debug_key(reporter)?,
        };
        let format = if layout.bundle {
            Format::Aab
        } else {
            Format::Apk
        };
        let repackaged = android::repackage(entries, layout, &app, &key)?;
        (repackaged, Platform::Android, format)
    } else {
        bail!(
            "{NOT_A_SHELL}: it has no AndroidManifest.xml or BundleConfig.pb, and no .app with an Info.plist"
        );
    };
    fs::write(&args.out, repackaged)
        .with_context(|| format!("couldn't write the app to {}", args.out.display()))?;
    reporter.report(&Event::Repackaged {
        shell: args.shell.display().to_string(),
        out: args.out.display().to_string(),
        platform,
        format,
    });
    Ok(())
}

fn install(args: &Install, reporter: &Reporter) -> Result<()> {
    let path = args.app.display();
    let app = fs::read(&args.app).with_context(|| format!("couldn't read the app {path}"))?;
    match zip::read(&app).ok().as_deref().and_then(android::layout) {
        Some(layout) if !layout.bundle => {}
        Some(_) => bail!(
            "{path} is an app bundle (AAB), which is for Google Play: it makes the APKs a device installs from it. Repackage an APK shell to install the app on a device"
        ),
        None => bail!("{path} isn't an APK"),
    }
    let key = adb_key(reporter)?;
    let mut device = adb::Device::connect(
        args.device.as_deref(),
        &adb::emulator_ports(),
        &key,
        &|device| {
            reporter.report(&Event::Authorizing {
                device: device.to_owned(),
            });
        },
    )?;
    device.install(&app)?;
    reporter.report(&Event::Installed {
        app: path.to_string(),
        device: device.serial().to_owned(),
        model: device.model().map(str::to_owned),
    });
    Ok(())
}

/// The key OmniBuild proves itself to Android devices with: adb's, which
/// OmniBuild creates where adb keeps it if adb hasn't yet.
fn adb_key(reporter: &Reporter) -> Result<AdbKey> {
    let path = adb::key_path();
    let (key, created) = AdbKey::at(&path, &adb::owner(|name| std::env::var(name).ok()))?;
    if created {
        reporter.report(&Event::KeyCreated {
            key: KeyKind::Adb,
            path: path.display().to_string(),
        });
    }
    Ok(key)
}

fn create_key(args: &CreateKey, reporter: &Reporter) -> Result<()> {
    let subject = distinguished_name(&args.subject).with_context(|| {
        format!(
            "\"{}\" isn't a distinguished name, such as \"CN=Upload key, O=Example\"",
            args.subject
        )
    })?;
    SigningKey::create(&args.keystore, &subject, &key_password())?;
    reporter.report(&Event::KeyCreated {
        key: KeyKind::Upload,
        path: args.keystore.display().to_string(),
    });
    Ok(())
}

/// A distinguished name as keytool takes one, with spaces after its
/// commas, which RFC 4514, and so x509-cert, doesn't allow.
fn distinguished_name(text: &str) -> Option<Name> {
    let mut name = String::new();
    let (mut escaped, mut after_comma) = (false, false);
    for character in text.chars() {
        if after_comma && character == ' ' {
            continue;
        }
        after_comma = character == ',' && !escaped;
        escaped = character == '\\' && !escaped;
        name.push(character);
    }
    name.parse().ok()
}

/// The key OmniBuild signs Android apps with when it's given none, which
/// it creates the first time.
fn android_debug_key(reporter: &Reporter) -> Result<SigningKey> {
    let path = debug_key_path();
    let (key, created) = SigningKey::debug(&path)?;
    if created {
        reporter.report(&Event::KeyCreated {
            key: KeyKind::Debug,
            path: path.display().to_string(),
        });
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::{Cli, distinguished_name};

    #[test]
    fn reads_a_distinguished_name_with_spaces_after_its_commas() {
        let name = |text| distinguished_name(text).map(|name| name.to_string());

        assert_eq!(
            name("CN=Jane Doe,  O=Example"),
            Some("CN=Jane Doe,O=Example".to_owned())
        );
        // An escaped comma is part of a value, with the space after it.
        assert_eq!(
            name("CN=Jane Doe, O=Doe\\, Inc."),
            Some("CN=Jane Doe,O=Doe\\, Inc.".to_owned())
        );
        assert_eq!(name("not a name"), None);
    }

    #[test]
    fn cli_definition_is_valid() {
        // clap checks a command's definition (conflicting flags, duplicate
        // names) only when it's built, so a mistake would otherwise surface
        // as a panic the first time a user runs that command.
        Cli::command().debug_assert();
    }
}
