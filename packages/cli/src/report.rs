//! What OmniBuild says as it works. People read sentences: a command's
//! result on stdout, and the rest, such as a key it created or an error, on
//! stderr. With `--json`, a build tool reads each event as a line of JSON
//! on stdout instead: its name, its values, and the sentence a person would
//! read, so that a tool can show events it doesn't know yet.

use std::io::{self, Write};

use serde::Serialize;

use crate::key::ALIAS;

/// Something OmniBuild did, or something it waits for.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
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
    /// The device asks its user whether to allow this computer, and
    /// OmniBuild waits for the answer.
    Authorizing { device: String },
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
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Android,
    Ios,
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
            Self::KeyRequested { path, request } => format!(
                "Created a key for an Apple signing certificate in {path}, and its certificate signing request in {request}. Upload the request at developer.apple.com, under Certificates, then add the certificate Apple issues: omni-build key import {path} <certificate>"
            ),
            Self::CertificateImported {
                path,
                certificate,
                expires,
            } => format!(
                "Added \"{certificate}\", which expires on {expires}, to {path}. Sign iOS apps with it with --signing-key, and a provisioning profile that allows it."
            ),
            Self::Authorizing { device } => format!(
                "{device} asks whether to allow debugging from this computer. Allow it on the device."
            ),
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
                | Self::Repackaged { .. }
                | Self::Installed { .. }
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
    use super::{Event, Format, KeyKind, Line, Platform, Reporter};

    fn repackaged() -> Event {
        Event::Repackaged {
            shell: "shell.ipa".to_owned(),
            out: "app.ipa".to_owned(),
            platform: Platform::Ios,
            format: Format::Ipa,
        }
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
                "Created a key for an Apple signing certificate in apple.p12, and its certificate signing request in apple.certSigningRequest. Upload the request at developer.apple.com, under Certificates, then add the certificate Apple issues: omni-build key import apple.p12 <certificate>"
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
        assert_eq!(
            tools.line(&Event::Error {
                message: "couldn't read \"shell\"".to_owned()
            }),
            Line::Stdout(r#"{"event":"error","message":"couldn't read \"shell\""}"#.to_owned())
        );
    }
}
