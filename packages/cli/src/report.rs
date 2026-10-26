//! What OmniBuild says as it works. People read sentences: a command's
//! result on stdout, and the rest, such as a key it created or an error, on
//! stderr. With `--json`, a build tool reads each event as a line of JSON
//! on stdout instead: its name, its values, and the sentence a person would
//! read, so that a tool can show events it doesn't know yet.

use std::io::{self, Write};

use serde::Serialize;

use crate::ios::Distribution;
use crate::key::ALIAS;

/// Something OmniBuild did, or something it waits for. Its values are
/// named as JavaScript names them, for the build tools that read them.
#[derive(Debug, PartialEq, Serialize)]
#[serde(
    tag = "event",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// A key OmniBuild created: one it needed and the user had none of, or
    /// the upload key `key create` makes.
    KeyCreated { key: KeyKind, path: String },
    /// `key request` created a key for an Apple signing certificate, and
    /// the request Apple issues the certificate for.
    KeyRequested { path: String, request: String },
    /// `key import` added a certificate to the keystore with its key.
    CertificateImported {
        path: String,
        /// The certificate's name, as Keychain Access shows it.
        certificate: String,
        /// When it expires, as a date (`2027-10-22`).
        expires: String,
    },
    /// `provision` had Apple issue a certificate for the key in the
    /// keystore, and added it there.
    CertificateCreated {
        path: String,
        /// The certificate's name, as Keychain Access shows it.
        certificate: String,
        /// When it expires, as a date (`2027-10-22`).
        expires: String,
    },
    /// `provision` registered an App ID for one of the app's bundles.
    AppIdRegistered { bundle_id: String },
    /// `provision` turned on a capability for a bundle's App ID, which an
    /// entitlement the bundle claims needs.
    CapabilityEnabled {
        bundle_id: String,
        /// Its name, as developer.apple.com shows it (`Push Notifications`).
        capability: String,
    },
    /// `provision` had Apple make a provisioning profile for one of the
    /// app's bundles.
    ProfileCreated { bundle_id: String, name: String },
    /// `provision` wrote a provisioning profile for each of the app's
    /// bundles, which allows the certificate of the key in the keystore.
    Provisioned {
        keystore: String,
        /// The certificate's name, as Keychain Access shows it.
        certificate: String,
        /// When it expires, as a date (`2027-10-22`).
        expires: String,
        distribution: Distribution,
        /// The folder with the profiles, which `--profile` takes.
        out: String,
        /// Each profile's file, in the folder.
        profiles: Vec<String>,
    },
    /// `upload` uploaded the app to its store.
    Uploaded(UploadedApp),
    /// `upload` uploaded the app to App Store Connect, which checks it
    /// before it takes it, and OmniBuild waits for its answer.
    Processing { app: String },
    /// App Store Connect took the app, or refused it, with a warning: about
    /// something the app should fix before it's released, which TestFlight
    /// doesn't need.
    UploadWarning {
        /// The number of its kind (`90078`), as Transporter reports them.
        code: String,
        description: String,
    },
    /// The device asks its user whether to allow this computer, and
    /// OmniBuild waits for the answer.
    Authorizing { device: String },
    /// An iPhone or iPad asks its user whether to trust this computer, and
    /// OmniBuild waits for the answer.
    Trusting {
        /// The device's name and UDID.
        device: String,
    },
    /// `repackage` found a native module among the app's dependencies, or
    /// theirs, with something for the shell's platform, which it adds to
    /// the app.
    ModuleFound { name: String, version: String },
    /// `repackage` wrote the app.
    Repackaged {
        shell: String,
        out: String,
        platform: Platform,
        format: Format,
    },
    /// `install` installed the app.
    Installed {
        app: String,
        /// The device's name, as `--device` takes it.
        device: String,
        /// The device's model, as its user knows it, if it says.
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// `tcpip` had the device listen for ADB on the network, where this
    /// computer reached it.
    Listening {
        /// The device's name, as `--device` took it.
        device: String,
        /// The device's model, as its user knows it, if it says.
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// Where it listens, as `--device` takes it from now on
        /// (`192.168.1.20:5555`).
        address: String,
    },
    /// `pair` paired this computer with the device for wireless debugging.
    Paired {
        /// Where the device listened for pairing.
        address: String,
        /// The device's name for itself, which it advertises its wireless
        /// debugging under on the network (`adb-R5CT1234-a1B2c3`).
        guid: String,
    },
    /// Why the command failed. It's the last event, and OmniBuild exits
    /// with 1. The reason is the event's message, so it has no field of
    /// its own.
    Error {
        #[serde(skip)]
        message: String,
    },
}

/// What a created key is for.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyKind {
    /// Signs Android apps when the user gives no key.
    Debug,
    /// Proves this computer's identity to Android devices.
    Adb,
    /// Signs app bundles for Google Play.
    Upload,
    /// Signs iOS apps, with the certificate Apple issues for it.
    Apple,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Android,
    Ios,
}

/// An app `upload` uploaded.
#[derive(Debug, PartialEq, Serialize)]
pub struct UploadedApp {
    pub app: String,
    /// The app's ID.
    pub id: String,
    /// Its version, as people see it, if it has one as text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Its build number: an Android app's version code, or an iOS app's
    /// `CFBundleVersion`.
    pub build: String,
    /// Which store, and what became of the app there.
    #[serde(flatten)]
    pub store: Store,
}

impl UploadedApp {
    /// The sentence a person reads.
    fn message(&self) -> String {
        let Self {
            app,
            id,
            version,
            build,
            store,
        } = self;
        let release = match version {
            Some(version) => format!("{id} {version} ({build})"),
            None => format!("{id} ({build})"),
        };
        match store {
            Store::GooglePlay { track, draft: true } => format!(
                "Uploaded {app}, {release}, to Google Play, as a draft release on its {track} track: roll it out in the Play Console"
            ),
            Store::GooglePlay {
                track,
                draft: false,
            } => format!("Released {app}, {release}, to Google Play's {track} track"),
            Store::AppStoreConnect => format!(
                "Uploaded {app}, {release}, to App Store Connect, which took it: TestFlight has it for testers once Apple has processed it"
            ),
        }
    }
}

/// The store `upload` uploaded an app to, and what became of it there.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "store", rename_all = "kebab-case")]
pub enum Store {
    /// Released to one of the app's tracks on Google Play.
    GooglePlay {
        /// The track, as `--track` took it.
        track: String,
        /// Whether the release is a draft, to roll out in the Play Console.
        draft: bool,
    },
    /// Taken by App Store Connect, which processes it for TestFlight.
    AppStoreConnect,
}

/// The kind of file an app is in.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    Apk,
    Aab,
    Ipa,
    /// A zipped `.app`, for the iOS Simulator.
    App,
}

impl Event {
    /// The sentence a person reads.
    fn message(&self) -> String {
        match self {
            Self::KeyCreated {
                key: KeyKind::Debug,
                path,
            } => format!("Created a debug key for signing Android apps: {path}"),
            Self::KeyCreated {
                key: KeyKind::Adb,
                path,
            } => format!("Created a key for ADB, which devices ask you to allow: {path}"),
            Self::KeyCreated {
                key: KeyKind::Upload,
                path,
            } => format!(
                "Created an upload key in {path}, with the alias \"{ALIAS}\". Keep it, and its password, safe: Google Play only takes updates signed with it, unless you ask Google to reset it."
            ),
            Self::KeyCreated {
                key: KeyKind::Apple,
                path,
            } => format!(
                "Created a key for an Apple signing certificate in {path}. Keep it, and its password, safe: the certificate Apple issues for it signs nothing without it."
            ),
            Self::KeyRequested { path, request } => format!(
                "Created a key for an Apple signing certificate in {path}, and its certificate signing request in {request}. Upload the request at developer.apple.com, under Certificates, then add the certificate Apple issues: omni key import {path} <certificate>"
            ),
            Self::CertificateImported {
                path,
                certificate,
                expires,
            } => format!(
                "Added \"{certificate}\", which expires on {expires}, to {path}. Sign iOS apps with it with --signing-key, and a provisioning profile that allows it."
            ),
            Self::CertificateCreated {
                path,
                certificate,
                expires,
            } => format!(
                "Apple issued \"{certificate}\", which expires on {expires}, for the key in {path}, and OmniBuild added it there"
            ),
            Self::AppIdRegistered { bundle_id } => format!("Registered the App ID {bundle_id}"),
            Self::CapabilityEnabled {
                bundle_id,
                capability,
            } => format!("Turned on {capability} for {bundle_id}"),
            Self::ProfileCreated { bundle_id, name } => {
                format!("Created the provisioning profile \"{name}\" for {bundle_id}")
            }
            Self::Provisioned {
                keystore,
                certificate,
                expires,
                distribution,
                out,
                ..
            } => format!(
                "Wrote {} provisioning profiles for the app to {out}, which allow \"{certificate}\" in {keystore}, until {expires}. Sign the app with --signing-key {keystore} --profile {out}",
                distribution.name()
            ),
            Self::Uploaded(upload) => upload.message(),
            Self::Processing { app } => format!(
                "App Store Connect has {app}, and checks it before it takes it, which takes a few minutes"
            ),
            Self::UploadWarning { code, description } => {
                format!("App Store Connect warns: {description} ({code})")
            }
            Self::Authorizing { device } => format!(
                "{device} asks whether to allow debugging from this computer. Allow it on the device."
            ),
            Self::Trusting { device } => format!(
                "{device} asks whether to trust this computer. Tap Trust on it, and enter its passcode."
            ),
            Self::ModuleFound { name, version } => {
                format!("Found the native module {name} {version} among the app's dependencies")
            }
            Self::Repackaged { shell, out, .. } => format!("Repackaged {shell} into {out}"),
            Self::Installed {
                app,
                device,
                model: Some(model),
            } => format!("Installed {app} on {model} ({device})"),
            Self::Installed {
                app,
                device,
                model: None,
            } => format!("Installed {app} on {device}"),
            // Until it restarts: adbd forgets the port then, as it does
            // for adb.
            Self::Listening {
                device,
                model,
                address,
            } => format!(
                "{} listens for ADB at {address} until it restarts. Unplug it, and install with --device {address}",
                match model {
                    Some(model) => format!("{model} ({device})"),
                    None => device.clone(),
                }
            ),
            // The address and port to install at are on the device's
            // screen, where it shows wireless debugging is on.
            Self::Paired { address, guid } => format!(
                "Paired with the device at {address} ({guid}). Install on it with --device and the IP address and port it shows under Wireless debugging"
            ),
            Self::Error { message } => message.clone(),
        }
    }

    /// Whether it's what the command was run for, which goes to stdout for
    /// people, rather than news along the way.
    fn is_result(&self) -> bool {
        matches!(
            self,
            Self::KeyCreated {
                key: KeyKind::Upload,
                ..
            } | Self::KeyRequested { .. }
                | Self::CertificateImported { .. }
                | Self::Provisioned { .. }
                | Self::Uploaded(_)
                | Self::Repackaged { .. }
                | Self::Installed { .. }
                | Self::Listening { .. }
                | Self::Paired { .. }
        )
    }
}

/// Says what happens, to people or, with `json`, to build tools.
pub struct Reporter {
    json: bool,
}

/// An event as a line for one of the standard streams.
#[derive(Debug, PartialEq)]
enum Line {
    Stdout(String),
    Stderr(String),
}

/// An event as JSON: its name and values, then its sentence.
#[derive(Serialize)]
struct Json<'a> {
    #[serde(flatten)]
    event: &'a Event,
    message: String,
}

impl Reporter {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    pub fn report(&self, event: &Event) {
        // A build tool that stops reading closes the pipe. OmniBuild then
        // finishes its work unheard, where println! would panic.
        let _ = match self.line(event) {
            Line::Stdout(line) => writeln!(io::stdout().lock(), "{line}"),
            Line::Stderr(line) => writeln!(io::stderr().lock(), "{line}"),
        };
    }

    fn line(&self, event: &Event) -> Line {
        let message = event.message();
        if self.json {
            Line::Stdout(
                serde_json::to_string(&Json { event, message }).expect("an event serializes"),
            )
        } else if let Event::Error { .. } = event {
            Line::Stderr(format!("error: {message}"))
        } else if event.is_result() {
            Line::Stdout(message)
        } else {
            Line::Stderr(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Format, KeyKind, Line, Platform, Reporter, Store, UploadedApp};
    use crate::ios::Distribution;

    fn repackaged() -> Event {
        Event::Repackaged {
            shell: "shell.ipa".to_owned(),
            out: "app.ipa".to_owned(),
            platform: Platform::Ios,
            format: Format::Ipa,
        }
    }

    fn paired() -> Event {
        Event::Paired {
            address: "192.168.1.20:37099".to_owned(),
            guid: "adb-R5CT1234-a1B2c3".to_owned(),
        }
    }

    fn provisioned() -> Event {
        Event::Provisioned {
            keystore: "apple.p12".to_owned(),
            certificate: "Apple Distribution: Jane Doe (ABCDE12345)".to_owned(),
            expires: "2027-10-22".to_owned(),
            distribution: Distribution::AppStore,
            out: "profiles".to_owned(),
            profiles: vec!["profiles/dev.example.app.mobileprovision".to_owned()],
        }
    }

    #[test]
    fn tells_people_what_provisioning_created() {
        let people = Reporter::new(false);

        assert_eq!(
            people.line(&provisioned()),
            Line::Stdout(
                "Wrote App Store provisioning profiles for the app to profiles, which allow \"Apple Distribution: Jane Doe (ABCDE12345)\" in apple.p12, until 2027-10-22. Sign the app with --signing-key apple.p12 --profile profiles"
                    .to_owned()
            )
        );
        let news = [
            (
                Event::KeyCreated {
                    key: KeyKind::Apple,
                    path: "apple.p12".to_owned(),
                },
                "Created a key for an Apple signing certificate in apple.p12. Keep it, and its password, safe: the certificate Apple issues for it signs nothing without it.",
            ),
            (
                Event::CertificateCreated {
                    path: "apple.p12".to_owned(),
                    certificate: "Apple Distribution: Jane Doe (ABCDE12345)".to_owned(),
                    expires: "2027-10-22".to_owned(),
                },
                "Apple issued \"Apple Distribution: Jane Doe (ABCDE12345)\", which expires on 2027-10-22, for the key in apple.p12, and OmniBuild added it there",
            ),
            (
                Event::AppIdRegistered {
                    bundle_id: "dev.example.app".to_owned(),
                },
                "Registered the App ID dev.example.app",
            ),
            (
                Event::CapabilityEnabled {
                    bundle_id: "dev.example.app".to_owned(),
                    capability: "Push Notifications".to_owned(),
                },
                "Turned on Push Notifications for dev.example.app",
            ),
            (
                Event::ProfileCreated {
                    bundle_id: "dev.example.app".to_owned(),
                    name: "OmniBuild App Store dev.example.app 1761400000".to_owned(),
                },
                "Created the provisioning profile \"OmniBuild App Store dev.example.app 1761400000\" for dev.example.app",
            ),
        ];
        for (event, message) in news {
            assert_eq!(people.line(&event), Line::Stderr(message.to_owned()));
        }
    }

    fn uploaded(version: Option<&str>, draft: bool) -> Event {
        Event::Uploaded(UploadedApp {
            app: "app.aab".to_owned(),
            id: "dev.example.app".to_owned(),
            version: version.map(str::to_owned),
            build: "42".to_owned(),
            store: Store::GooglePlay {
                track: "internal".to_owned(),
                draft,
            },
        })
    }

    #[test]
    fn tells_people_what_it_uploaded() {
        let people = Reporter::new(false);

        assert_eq!(
            people.line(&uploaded(Some("1.2.0"), false)),
            Line::Stdout(
                "Released app.aab, dev.example.app 1.2.0 (42), to Google Play's internal track"
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&uploaded(None, true)),
            Line::Stdout(
                "Uploaded app.aab, dev.example.app (42), to Google Play, as a draft release on its internal track: roll it out in the Play Console"
                    .to_owned()
            )
        );
    }

    #[test]
    fn tells_people_what_app_store_connect_does_with_an_app() {
        let people = Reporter::new(false);

        assert_eq!(
            people.line(&Event::Uploaded(UploadedApp {
                app: "app.ipa".to_owned(),
                id: "dev.example.app".to_owned(),
                version: Some("1.2.0".to_owned()),
                build: "42".to_owned(),
                store: Store::AppStoreConnect,
            })),
            Line::Stdout(
                "Uploaded app.ipa, dev.example.app 1.2.0 (42), to App Store Connect, which took it: TestFlight has it for testers once Apple has processed it"
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&Event::Processing {
                app: "app.ipa".to_owned()
            }),
            Line::Stderr(
                "App Store Connect has app.ipa, and checks it before it takes it, which takes a few minutes"
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&Event::UploadWarning {
                code: "90078".to_owned(),
                description: "Missing Push Notification Entitlement.".to_owned(),
            }),
            Line::Stderr(
                "App Store Connect warns: Missing Push Notification Entitlement. (90078)"
                    .to_owned()
            )
        );
    }

    #[test]
    fn tells_build_tools_what_it_uploaded() {
        let tools = Reporter::new(true);

        assert!(matches!(
            tools.line(&uploaded(Some("1.2.0"), false)),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"uploaded","app":"app.aab","id":"dev.example.app","version":"1.2.0","build":"42","store":"google-play","track":"internal","draft":false,"message":"Released "#
            )
        ));
        // A version the app doesn't have as text is left out.
        assert!(matches!(
            tools.line(&uploaded(None, true)),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"uploaded","app":"app.aab","id":"dev.example.app","build":"42","store":"google-play","track":"internal","draft":true,"message":"Uploaded "#
            )
        ));
        assert!(matches!(
            tools.line(&Event::Uploaded(UploadedApp {
                app: "app.ipa".to_owned(),
                id: "dev.example.app".to_owned(),
                version: Some("1.2.0".to_owned()),
                build: "42".to_owned(),
                store: Store::AppStoreConnect,
            })),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"uploaded","app":"app.ipa","id":"dev.example.app","version":"1.2.0","build":"42","store":"app-store-connect","message":"Uploaded "#
            )
        ));
        assert!(matches!(
            tools.line(&Event::Processing { app: "app.ipa".to_owned() }),
            Line::Stdout(line) if line.starts_with(r#"{"event":"processing","app":"app.ipa","message":"#)
        ));
        assert!(matches!(
            tools.line(&Event::UploadWarning {
                code: "90078".to_owned(),
                description: "Missing Push Notification Entitlement.".to_owned(),
            }),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"upload-warning","code":"90078","description":"Missing Push Notification Entitlement.","message":"#
            )
        ));
    }

    #[test]
    fn tells_people_the_native_modules_it_found() {
        assert_eq!(
            Reporter::new(false).line(&Event::ModuleFound {
                name: "@example/haptics".to_owned(),
                version: "1.0.0".to_owned(),
            }),
            Line::Stderr(
                "Found the native module @example/haptics 1.0.0 among the app's dependencies"
                    .to_owned()
            )
        );
    }

    #[test]
    fn tells_people_results_on_stdout_and_the_rest_on_stderr() {
        let people = Reporter::new(false);
        let key = |key| Event::KeyCreated {
            key,
            path: "key.p12".to_owned(),
        };
        let installed = |model: Option<&str>| Event::Installed {
            app: "app.apk".to_owned(),
            device: "emulator-5554".to_owned(),
            model: model.map(str::to_owned),
        };

        assert_eq!(
            people.line(&repackaged()),
            Line::Stdout("Repackaged shell.ipa into app.ipa".to_owned())
        );
        assert_eq!(
            people.line(&installed(Some("Pixel 8"))),
            Line::Stdout("Installed app.apk on Pixel 8 (emulator-5554)".to_owned())
        );
        assert_eq!(
            people.line(&installed(None)),
            Line::Stdout("Installed app.apk on emulator-5554".to_owned())
        );
        for (model, device) in [(Some("Pixel 8"), "Pixel 8 (R5CT1234)"), (None, "R5CT1234")] {
            assert_eq!(
                people.line(&Event::Listening {
                    device: "R5CT1234".to_owned(),
                    model: model.map(str::to_owned),
                    address: "192.168.1.20:5555".to_owned(),
                }),
                Line::Stdout(format!(
                    "{device} listens for ADB at 192.168.1.20:5555 until it restarts. Unplug it, and install with --device 192.168.1.20:5555"
                ))
            );
        }
        assert_eq!(
            people.line(&paired()),
            Line::Stdout(
                "Paired with the device at 192.168.1.20:37099 (adb-R5CT1234-a1B2c3). Install on it with --device and the IP address and port it shows under Wireless debugging"
                    .to_owned()
            )
        );
        assert!(matches!(
            people.line(&key(KeyKind::Upload)),
            Line::Stdout(line) if line.starts_with("Created an upload key in key.p12, with the alias \"upload\".")
        ));
        assert_eq!(
            people.line(&Event::KeyRequested {
                path: "apple.p12".to_owned(),
                request: "apple.certSigningRequest".to_owned(),
            }),
            Line::Stdout(
                "Created a key for an Apple signing certificate in apple.p12, and its certificate signing request in apple.certSigningRequest. Upload the request at developer.apple.com, under Certificates, then add the certificate Apple issues: omni key import apple.p12 <certificate>"
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&Event::CertificateImported {
                path: "apple.p12".to_owned(),
                certificate: "Apple Distribution: Jane Doe (ABCDE12345)".to_owned(),
                expires: "2027-10-22".to_owned(),
            }),
            Line::Stdout(
                "Added \"Apple Distribution: Jane Doe (ABCDE12345)\", which expires on 2027-10-22, to apple.p12. Sign iOS apps with it with --signing-key, and a provisioning profile that allows it."
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&key(KeyKind::Debug)),
            Line::Stderr("Created a debug key for signing Android apps: key.p12".to_owned())
        );
        assert_eq!(
            people.line(&key(KeyKind::Adb)),
            Line::Stderr(
                "Created a key for ADB, which devices ask you to allow: key.p12".to_owned()
            )
        );
        assert_eq!(
            people.line(&Event::Authorizing {
                device: "192.168.1.20:5555".to_owned()
            }),
            Line::Stderr(
                "192.168.1.20:5555 asks whether to allow debugging from this computer. Allow it on the device."
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&Event::Trusting {
                device: "Jane's iPhone (00008030-001A2B3C4D5E6F7A)".to_owned()
            }),
            Line::Stderr(
                "Jane's iPhone (00008030-001A2B3C4D5E6F7A) asks whether to trust this computer. Tap Trust on it, and enter its passcode."
                    .to_owned()
            )
        );
        assert_eq!(
            people.line(&Event::Error {
                message: "couldn't read the shell".to_owned()
            }),
            Line::Stderr("error: couldn't read the shell".to_owned())
        );
    }

    #[test]
    fn tells_build_tools_each_event_as_a_line_of_json_on_stdout() {
        let tools = Reporter::new(true);

        assert_eq!(
            tools.line(&repackaged()),
            Line::Stdout(
                r#"{"event":"repackaged","shell":"shell.ipa","out":"app.ipa","platform":"ios","format":"ipa","message":"Repackaged shell.ipa into app.ipa"}"#
                    .to_owned()
            )
        );
        assert!(matches!(
            tools.line(&Event::CertificateImported {
                path: "apple.p12".to_owned(),
                certificate: "Apple Distribution: Jane Doe (ABCDE12345)".to_owned(),
                expires: "2027-10-22".to_owned(),
            }),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"certificate-imported","path":"apple.p12","certificate":"Apple Distribution: Jane Doe (ABCDE12345)","expires":"2027-10-22","message":"Added "#
            )
        ));
        // A model the device doesn't say is left out.
        assert_eq!(
            tools.line(&Event::Installed {
                app: "app.apk".to_owned(),
                device: "emulator-5554".to_owned(),
                model: None,
            }),
            Line::Stdout(
                r#"{"event":"installed","app":"app.apk","device":"emulator-5554","message":"Installed app.apk on emulator-5554"}"#
                    .to_owned()
            )
        );
        assert!(matches!(
            tools.line(&Event::Listening {
                device: "R5CT1234".to_owned(),
                model: Some("Pixel 8".to_owned()),
                address: "192.168.1.20:5555".to_owned(),
            }),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"listening","device":"R5CT1234","model":"Pixel 8","address":"192.168.1.20:5555","message":"Pixel 8 (R5CT1234) listens for ADB at "#
            )
        ));
        assert!(matches!(
            tools.line(&paired()),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"paired","address":"192.168.1.20:37099","guid":"adb-R5CT1234-a1B2c3","message":"Paired with "#
            )
        ));
        assert!(matches!(
            tools.line(&provisioned()),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"provisioned","keystore":"apple.p12","certificate":"Apple Distribution: Jane Doe (ABCDE12345)","expires":"2027-10-22","distribution":"app-store","out":"profiles","profiles":["profiles/dev.example.app.mobileprovision"],"message":"Wrote "#
            )
        ));
        assert!(matches!(
            tools.line(&Event::CapabilityEnabled {
                bundle_id: "dev.example.app".to_owned(),
                capability: "Push Notifications".to_owned(),
            }),
            Line::Stdout(line) if line.starts_with(
                r#"{"event":"capability-enabled","bundleId":"dev.example.app","capability":"Push Notifications","message":"#
            )
        ));
        assert_eq!(
            tools.line(&Event::ModuleFound {
                name: "@example/haptics".to_owned(),
                version: "1.0.0".to_owned(),
            }),
            Line::Stdout(
                r#"{"event":"module-found","name":"@example/haptics","version":"1.0.0","message":"Found the native module @example/haptics 1.0.0 among the app's dependencies"}"#
                    .to_owned()
            )
        );
        assert_eq!(
            tools.line(&Event::Error {
                message: "couldn't read \"shell\"".to_owned()
            }),
            Line::Stdout(r#"{"event":"error","message":"couldn't read \"shell\""}"#.to_owned())
        );
    }
}
