//! The `omni` command line.

mod adb;
mod android;
mod app_store_connect;
mod bytes;
mod config;
mod icon;
mod ios;
mod key;
mod keystore;
mod modules;
mod payload;
mod report;
mod zip;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use adb::AdbKey;
use app_store_connect::{ApiKey, Client};
use config::App;
use ios::{Distribution, Provisioner};
use key::{SigningKey, debug_key_path, display_name};
use modules::Module;
use report::{Event, Format, KeyKind, Platform, Reporter};
use x509_cert::name::Name;

/// How the error for a file that's none of the shells OmniBuild knows
/// starts.
const NOT_A_SHELL: &str = "the shell isn't an APK, an AAB or an iOS app";
/// A keystore's password, from the environment rather than a flag, which
/// would show it to anyone listing the computer's processes.
const KEY_PASSWORD: &str = "OMNI_BUILD_KEY_PASSWORD";
/// Where App Store Connect's API is, if not at Apple: the CLI's tests point
/// it at one of their own.
const APP_STORE_CONNECT_URL: &str = "OMNI_BUILD_APP_STORE_CONNECT_URL";

/// Build, sign, install and ship iOS and Android apps with no native toolchain.
#[derive(Debug, Parser)]
#[command(name = "omni", version, about, arg_required_else_help = true)]
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
    Provision(Provision),
    Install(Install),
    Tcpip(Tcpip),
    Pair(Pair),
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
    /// A provisioning profile for an iOS build for devices
    /// (`.mobileprovision`), which allows the signing key's certificate and
    /// the app's ID, or a folder of them, as `provision` writes. Give one
    /// for each of the app's extensions too, which have IDs of their own,
    /// unless a wildcard App ID covers them: each takes the profile that
    /// fits its ID closest, as Xcode picks them.
    #[arg(long)]
    profile: Vec<PathBuf>,
}

/// Get what signs an iOS app for devices from App Store Connect, with an
/// App Store Connect API key, as Xcode's automatic signing does: a
/// certificate for the signing key, an App ID for the app and each bundle
/// nested in it, with the capabilities their entitlements need, and a
/// provisioning profile for each. What the account has already is reused,
/// and what it lacks is created.
///
/// Then sign the app with `repackage --signing-key` and the same keystore,
/// and `--profile` and the folder of profiles. Repackage the same shell
/// with the same config: the bundles' IDs and entitlements are what it
/// gives them.
#[derive(Debug, Args)]
struct Provision {
    /// The app shell, built for devices (an `.ipa`).
    shell: PathBuf,
    /// The folder to write the profiles to, one for the app and each bundle
    /// nested in it, named by its ID.
    #[arg(long, short)]
    out: PathBuf,
    /// The app config, as `repackage` takes it, which gives the app its ID
    /// and the entitlements it claims.
    #[arg(long, short)]
    config: Option<PathBuf>,
    /// The keystore with the key to sign with: a PKCS #12 keystore, with
    /// its password in `OMNI_BUILD_KEY_PASSWORD`, such as `key request`
    /// creates. Apple issues a certificate for the key if the account has
    /// none for it, and it's added to the keystore. OmniBuild creates the
    /// keystore if there's none.
    #[arg(long)]
    signing_key: PathBuf,
    /// What the app is provisioned for: the App Store needs a distribution
    /// certificate, and development and ad hoc profiles list the account's
    /// devices.
    #[arg(long, value_enum, default_value_t = Distribution::AppStore)]
    distribution: Distribution,
    #[command(flatten)]
    api_key: ApiKeyArgs,
}

/// An App Store Connect API key, which an account's Admin creates in App
/// Store Connect, under Users and Access > Integrations.
#[derive(Debug, Args)]
struct ApiKeyArgs {
    /// The key's file, as App Store Connect gives it: `AuthKey_<key ID>.p8`.
    #[arg(long = "api-key")]
    file: PathBuf,
    /// The key's ID, if its file isn't named by it.
    #[arg(long = "api-key-id")]
    id: Option<String>,
    /// The issuer ID, which App Store Connect shows above a team's keys,
    /// for a team key. An individual key has none.
    #[arg(long = "api-issuer")]
    issuer: Option<String>,
}

/// Install an APK on an Android device or emulator, over ADB, with no
/// Android SDK.
#[derive(Debug, Args)]
struct Install {
    /// The app: an APK. Google Play makes the APKs a device installs from
    /// an app bundle (AAB).
    app: PathBuf,
    /// The device to install on: a phone plugged in by USB, by its serial
    /// number, as `adb devices` shows it, an emulator, by its name
    /// (`emulator-5554`), or a device on the network, by its address and
    /// port (`192.168.1.20:5555`): with wireless debugging, the IP address
    /// and port it shows there, once paired with `omni pair`. Without it,
    /// the one phone plugged in or emulator running on this computer.
    #[arg(long, short)]
    device: Option<String>,
}

/// Have an Android phone plugged in by USB listen for ADB on the network, as
/// `adb tcpip` does, so `install` reaches it there without its cable until
/// it restarts. The phone and this computer need to be on the same network.
#[derive(Debug, Args)]
struct Tcpip {
    /// The phone: one plugged in by USB, by its serial number, as `adb
    /// devices` shows it, or a device on the network already, by its address
    /// and port, to move it to another port. Without it, the one phone
    /// plugged in.
    #[arg(long, short)]
    device: Option<String>,
    /// The port it listens on.
    #[arg(long, short, default_value_t = 5555, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}

/// Pair this computer with an Android device for wireless debugging, as
/// `adb pair` does, so `install` reaches it over the network, encrypted,
/// with no cable.
///
/// On the device, under Developer options > Wireless debugging, tap "Pair
/// device with pairing code": it shows the address and port to pair with,
/// and the code. The device and this computer need to be on the same
/// network.
#[derive(Debug, Args)]
struct Pair {
    /// Where the device listens for pairing, as it shows it with the code
    /// (`192.168.1.20:37099`): not the address and port Wireless debugging
    /// shows, which `install` connects to.
    address: String,
    /// The code the device shows, six digits.
    code: String,
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
    Request(RequestKey),
    Import(ImportCertificate),
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

/// Create a key for an Apple signing certificate, as Keychain Access's
/// Certificate Assistant does: an RSA key in a PKCS #12 keystore, and a
/// certificate signing request for it beside the keystore
/// (`.certSigningRequest`), which developer.apple.com takes to issue the
/// certificate. Then add the certificate to the keystore with `key import`.
/// Its password comes from `OMNI_BUILD_KEY_PASSWORD`.
#[derive(Debug, Args)]
struct RequestKey {
    /// Where to write the keystore, which mustn't exist yet.
    keystore: PathBuf,
    /// The request's subject, as a distinguished name. Its common name names
    /// the key, as Keychain Access names it, and Apple issues the
    /// certificate in the team's name whatever it is.
    #[arg(long, default_value = "CN=App signing key")]
    subject: String,
}

/// Add a certificate to the keystore with its key: the one Apple issues for
/// the request `key request` made, which the keystore then signs iOS apps
/// with. Its password comes from `OMNI_BUILD_KEY_PASSWORD`.
#[derive(Debug, Args)]
struct ImportCertificate {
    /// The keystore with the key.
    keystore: PathBuf,
    /// The certificate: the `.cer` file developer.apple.com issued, or one
    /// in PEM.
    certificate: PathBuf,
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
        Command::Provision(args) => provision(&args, reporter),
        Command::Install(args) => install(&args, reporter),
        Command::Tcpip(args) => tcpip(&args, reporter),
        Command::Pair(args) => pair(&args, reporter),
        Command::Key(Key { command }) => match command {
            KeyCommand::Create(args) => create_key(&args, reporter),
            KeyCommand::Request(args) => request_key(&args, reporter),
            KeyCommand::Import(args) => import_certificate(&args, reporter),
        },
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
    let profiles = args
        .profile
        .iter()
        .map(|path| read_profiles(path))
        .collect::<Result<Vec<_>>>()?
        .concat();
    let modules = match &app.package_json {
        Some(path) => modules::find(path)?,
        None => Vec::new(),
    };
    let entries = zip::read(&shell).context(NOT_A_SHELL)?;
    let (repackaged, platform, format) = if let Some(bundle) = ios::bundle(&entries) {
        let format = if bundle.starts_with("Payload/") {
            Format::Ipa
        } else {
            Format::App
        };
        report_modules(&modules, &Platform::Ios, reporter);
        let repackaged = ios::repackage(entries, &bundle, &app, &modules, key.as_ref(), &profiles)?;
        (repackaged, Platform::Ios, format)
    } else if let Some(layout) = android::layout(&entries) {
        if !profiles.is_empty() {
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
        report_modules(&modules, &Platform::Android, reporter);
        let repackaged = android::repackage(entries, layout, &app, &modules, &key)?;
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

/// Tells which of the native modules go in an app for `platform`: those
/// with something for it.
fn report_modules(modules: &[Module], platform: &Platform, reporter: &Reporter) {
    for module in modules.iter().filter(|module| match platform {
        Platform::Ios => module.ios.is_some(),
        Platform::Android => module.android.is_some(),
    }) {
        reporter.report(&Event::ModuleFound {
            name: module.name.clone(),
            version: module.version.clone(),
        });
    }
}

/// The provisioning profiles at `path`: a profile, or a folder of them, as
/// `provision` writes, each a `.mobileprovision` in it.
fn read_profiles(path: &Path) -> Result<Vec<Vec<u8>>> {
    let read = |path: &Path| {
        fs::read(path)
            .with_context(|| format!("couldn't read the provisioning profile {}", path.display()))
    };
    if !path.is_dir() {
        return Ok(vec![read(path)?]);
    }
    let mut files: Vec<PathBuf> = fs::read_dir(path)
        .with_context(|| format!("couldn't read the folder {}", path.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|file| {
            file.extension()
                .is_some_and(|extension| extension == "mobileprovision")
        })
        .collect();
    files.sort();
    files.iter().map(|file| read(file)).collect()
}

fn provision(args: &Provision, reporter: &Reporter) -> Result<()> {
    let shell = fs::read(&args.shell)
        .with_context(|| format!("couldn't read the shell {}", args.shell.display()))?;
    let app = match &args.config {
        Some(path) => App::read(path)?,
        None => App::default(),
    };
    let entries = zip::read(&shell).context(NOT_A_SHELL)?;
    let bundle = ios::bundle(&entries).with_context(|| {
        format!(
            "{} isn't an iOS app, which is what provisioning profiles sign",
            args.shell.display()
        )
    })?;
    let bundles = ios::signed_bundles(&entries, &bundle, &app)?;
    let ApiKeyArgs { file, id, issuer } = &args.api_key;
    let key = ApiKey::read(file, id.as_deref(), issuer.as_deref())?;
    let url = std::env::var(APP_STORE_CONNECT_URL);
    let url = url.as_deref().unwrap_or(app_store_connect::URL);
    let provisioned = Provisioner {
        client: &Client::new(key, url),
        distribution: args.distribution,
        report: &|event| reporter.report(&event),
    }
    .provision(&bundles, &args.signing_key, &key_password())?;
    fs::create_dir_all(&args.out)
        .with_context(|| format!("couldn't create the folder {}", args.out.display()))?;
    let mut profiles = Vec::new();
    for (id, profile) in &provisioned.profiles {
        let path = args.out.join(format!("{id}.mobileprovision"));
        fs::write(&path, profile).with_context(|| {
            format!(
                "couldn't write the provisioning profile to {}",
                path.display()
            )
        })?;
        profiles.push(path.display().to_string());
    }
    reporter.report(&Event::Provisioned {
        keystore: args.signing_key.display().to_string(),
        certificate: display_name(&provisioned.key.x509().tbs_certificate.subject),
        expires: provisioned.key.expires(),
        distribution: args.distribution,
        out: args.out.display().to_string(),
        profiles,
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
        &adb::Usb,
        &key,
        &authorizing(reporter),
    )?;
    device.install(&app)?;
    reporter.report(&Event::Installed {
        app: path.to_string(),
        device: device.serial().to_owned(),
        model: device.model().map(str::to_owned),
    });
    Ok(())
}

fn tcpip(args: &Tcpip, reporter: &Reporter) -> Result<()> {
    let key = adb_key(reporter)?;
    let authorizing = authorizing(reporter);
    let mut device = adb::Device::connect(
        args.device.as_deref(),
        &adb::emulator_ports(),
        &adb::Usb,
        &key,
        &authorizing,
    )?;
    let address = device.listen(args.port, &key, &authorizing, adb::LISTEN_TIMEOUT)?;
    reporter.report(&Event::Listening {
        device: device.serial().to_owned(),
        model: device.model().map(str::to_owned),
        address: address.to_string(),
    });
    Ok(())
}

fn pair(args: &Pair, reporter: &Reporter) -> Result<()> {
    let key = adb_key(reporter)?;
    let guid = adb::pair(&args.address, &args.code, &key)?;
    reporter.report(&Event::Paired {
        address: args.address.clone(),
        guid,
    });
    Ok(())
}

/// Says a device asks its user whether to allow this computer, given the
/// device's name.
fn authorizing(reporter: &Reporter) -> impl Fn(&str) + '_ {
    |device| {
        reporter.report(&Event::Authorizing {
            device: device.to_owned(),
        });
    }
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
    let subject = subject(&args.subject, "CN=Upload key, O=Example")?;
    SigningKey::create(&args.keystore, &subject, &key_password())?;
    reporter.report(&Event::KeyCreated {
        key: KeyKind::Upload,
        path: args.keystore.display().to_string(),
    });
    Ok(())
}

fn request_key(args: &RequestKey, reporter: &Reporter) -> Result<()> {
    let subject = subject(
        &args.subject,
        "CN=Jane Doe key, emailAddress=jane@example.com",
    )?;
    // Beside the keystore, with the extension Keychain Access gives a
    // request, which developer.apple.com asks for.
    let request = args.keystore.with_extension("certSigningRequest");
    key::request(&args.keystore, &request, &subject, &key_password())?;
    reporter.report(&Event::KeyRequested {
        path: args.keystore.display().to_string(),
        request: request.display().to_string(),
    });
    Ok(())
}

fn import_certificate(args: &ImportCertificate, reporter: &Reporter) -> Result<()> {
    let key = SigningKey::import(&args.keystore, &args.certificate, &key_password())?;
    reporter.report(&Event::CertificateImported {
        path: args.keystore.display().to_string(),
        certificate: display_name(&key.x509().tbs_certificate.subject),
        expires: key.expires(),
    });
    Ok(())
}

/// A certificate's subject, or a request's, from `--subject`.
fn subject(text: &str, example: &str) -> Result<Name> {
    distinguished_name(text)
        .with_context(|| format!("\"{text}\" isn't a distinguished name, such as \"{example}\""))
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
