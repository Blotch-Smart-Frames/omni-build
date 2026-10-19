//! An app's `Info.plist`, where iOS reads its ID, name, version and build
//! number, among everything else it needs to know before running it.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

use crate::config::App;

/// The start of a binary property list, the format Xcode writes an app's
/// Info.plist in.
const BINARY: &[u8] = b"bplist00";

#[derive(Clone, Debug, PartialEq)]
pub struct InfoPlist {
    /// The keys in the order the shell has them, which `plist` keeps.
    dictionary: Dictionary,
    /// Whether it was read from a binary property list rather than XML.
    /// It's written back as it came.
    binary: bool,
}

impl InfoPlist {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value = Value::from_reader(Cursor::new(bytes)).context("it isn't a property list")?;
        let dictionary = value
            .into_dictionary()
            .context("it isn't a dictionary of keys and values")?;
        Ok(Self {
            dictionary,
            binary: bytes.starts_with(BINARY),
        })
    }

    /// The platforms the app is built for, which Xcode lists in
    /// `CFBundleSupportedPlatforms`: `iPhoneSimulator` for the iOS
    /// Simulator, `iPhoneOS` for devices.
    pub fn platforms(&self) -> Vec<&str> {
        self.dictionary
            .get("CFBundleSupportedPlatforms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_string)
            .collect()
    }

    /// The bundle ID, `CFBundleIdentifier`.
    pub fn id(&self) -> Option<&str> {
        self.string("CFBundleIdentifier")
    }

    /// The executable's name in the bundle, `CFBundleExecutable`.
    pub fn executable(&self) -> Option<&str> {
        self.string("CFBundleExecutable")
    }

    fn string(&self, key: &str) -> Option<&str> {
        self.dictionary.get(key).and_then(Value::as_string)
    }

    /// The name of the app's icon in its asset catalog, which Xcode gives
    /// in `CFBundleIcons` → `CFBundlePrimaryIcon` → `CFBundleIconName`.
    pub fn icon_name(&self) -> Option<&str> {
        self.primary_icon("CFBundleIcons")
            .and_then(|icon| icon.get("CFBundleIconName"))
            .and_then(Value::as_string)
    }

    /// The PNGs of the icon that Xcode puts beside the asset catalog for
    /// earlier versions of iOS, named as `CFBundleIconFiles` lists them,
    /// without their scale and device: `AppIcon60x60` for
    /// `AppIcon60x60@2x.png`. The iPad's (`CFBundleIcons~ipad`) list the
    /// iPhone's again, with its own.
    pub fn icon_files(&self) -> Vec<&str> {
        let mut files: Vec<&str> = ["CFBundleIcons", "CFBundleIcons~ipad"]
            .into_iter()
            .filter_map(|key| self.primary_icon(key))
            .filter_map(|icon| icon.get("CFBundleIconFiles"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_string)
            .collect();
        files.sort_unstable();
        files.dedup();
        files
    }

    fn primary_icon(&self, key: &str) -> Option<&Dictionary> {
        self.dictionary
            .get(key)
            .and_then(Value::as_dictionary)
            .and_then(|icons| icons.get("CFBundlePrimaryIcon"))
            .and_then(Value::as_dictionary)
    }

    /// Gives the app its ID, name, version and build number. Every other
    /// key stays as Xcode wrote it, the `DT*` keys above all: they say
    /// which Xcode and SDK built the shell, which App Store validation
    /// checks, and that's still true of the app.
    pub fn rewrite(&mut self, app: &App) -> Result<()> {
        if let Some(id) = &app.id {
            if !is_bundle_id(id) {
                bail!(
                    "\"{id}\" isn't a valid iOS bundle ID: it can hold only letters, digits, hyphens and dots"
                );
            }
            self.set("CFBundleIdentifier", id);
        }
        // The name under the icon. `CFBundleName`, the bundle's short name,
        // is what iOS shows only when there's no display name, so it can
        // stay the shell's.
        if let Some(name) = &app.name {
            self.set("CFBundleDisplayName", name);
        }
        if let Some(version) = &app.version {
            if !is_version(version) {
                bail!(
                    "\"{version}\" isn't a version the App Store takes: it needs one to three numbers separated by dots, such as 2.5.0"
                );
            }
            self.set("CFBundleShortVersionString", version);
        }
        if let Some(build) = app.build {
            self.set("CFBundleVersion", &build.to_string());
        }
        Ok(())
    }

    fn set(&mut self, key: &str, value: &str) {
        // A key the shell has keeps its place; a new one goes at the end.
        self.dictionary
            .insert(key.to_owned(), Value::String(value.to_owned()));
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let value = Value::Dictionary(self.dictionary.clone());
        let mut bytes = Vec::new();
        if self.binary {
            value.to_writer_binary(&mut bytes)
        } else {
            value.to_writer_xml(&mut bytes)
        }
        .expect("writing into memory can't fail");
        bytes
    }
}

/// Whether `id` is a valid bundle ID: letters, digits, hyphens and dots,
/// as Apple documents for `CFBundleIdentifier`.
fn is_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'))
}

/// Whether `version` is one App Store validation takes for
/// `CFBundleShortVersionString`: one to three whole numbers, separated by
/// dots.
fn is_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() <= 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use plist::Value;

    use super::InfoPlist;
    use crate::config::App;
    use crate::ios::tests::shell_info_plist;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>dev.omnibuild.shell</string>
	<key>CFBundleName</key>
	<string>OmniBuildShell</string>
</dict>
</plist>"#;

    fn app() -> App {
        App {
            id: Some("dev.example.app".to_owned()),
            name: Some("Repackaged".to_owned()),
            version: Some("2.5.0".to_owned()),
            build: Some(42),
            ..App::default()
        }
    }

    /// The keys and values of a written Info.plist, in order.
    fn entries(bytes: &[u8]) -> Vec<(String, Value)> {
        Value::from_reader(std::io::Cursor::new(bytes))
            .unwrap()
            .into_dictionary()
            .unwrap()
            .into_iter()
            .collect()
    }

    #[test]
    fn reads_the_shells_info_plist() {
        let info = InfoPlist::parse(&shell_info_plist()).unwrap();

        assert!(info.binary);
        assert_eq!(info.platforms(), ["iPhoneSimulator"]);
        assert_eq!(info.id(), Some("dev.omnibuild.shell"));
        assert_eq!(info.executable(), Some("OmniBuildShell"));
        assert_eq!(info.icon_name(), Some("AppIcon"));
        assert_eq!(info.icon_files(), ["AppIcon60x60", "AppIcon76x76"]);
        assert_eq!(
            info.dictionary.get("CFBundleIdentifier"),
            Some(&Value::String("dev.omnibuild.shell".to_owned()))
        );
    }

    #[test]
    fn gives_the_app_its_values_and_keeps_the_rest() {
        let shell = shell_info_plist();
        let mut info = InfoPlist::parse(&shell).unwrap();

        info.rewrite(&app()).unwrap();

        let bytes = info.to_bytes();
        assert!(bytes.starts_with(b"bplist00"));
        let (before, after) = (entries(&shell), entries(&bytes));
        let changed: Vec<_> = before
            .iter()
            .zip(&after)
            .filter(|(old, new)| old != new)
            .map(|(_, (key, value))| (key.as_str(), value.as_string().unwrap()))
            .collect();
        assert_eq!(
            changed,
            [
                ("CFBundleDisplayName", "Repackaged"),
                ("CFBundleIdentifier", "dev.example.app"),
                ("CFBundleShortVersionString", "2.5.0"),
                ("CFBundleVersion", "42"),
            ]
        );
        // Same keys, in the same order: the DT* keys among them.
        assert_eq!(before.len(), after.len());
        assert!(after.iter().any(|(key, _)| key == "DTXcode"));
    }

    #[test]
    fn adds_a_display_name_to_xml() {
        let mut info = InfoPlist::parse(XML.as_bytes()).unwrap();

        info.rewrite(&app()).unwrap();

        let bytes = info.to_bytes();
        assert!(bytes.starts_with(b"<?xml"));
        let keys: Vec<String> = entries(&bytes).into_iter().map(|(key, _)| key).collect();
        assert_eq!(
            keys,
            [
                "CFBundleIdentifier",
                "CFBundleName",
                "CFBundleDisplayName",
                "CFBundleShortVersionString",
                "CFBundleVersion",
            ]
        );
        assert_eq!(info.platforms(), Vec::<&str>::new());
        assert_eq!(info.icon_name(), None);
        assert_eq!(info.icon_files(), Vec::<&str>::new());
    }

    #[test]
    fn rejects_values_ios_wont_take() {
        let cases = [
            (
                App {
                    id: Some("dev.my_app".to_owned()),
                    ..App::default()
                },
                "\"dev.my_app\" isn't a valid iOS bundle ID",
            ),
            (
                App {
                    id: Some(String::new()),
                    ..App::default()
                },
                "\"\" isn't a valid iOS bundle ID",
            ),
            (
                App {
                    version: Some("2.5.0-beta".to_owned()),
                    ..App::default()
                },
                "\"2.5.0-beta\" isn't a version the App Store takes",
            ),
            (
                App {
                    version: Some("1.2.3.4".to_owned()),
                    ..App::default()
                },
                "\"1.2.3.4\" isn't a version the App Store takes",
            ),
            (
                App {
                    version: Some("2..0".to_owned()),
                    ..App::default()
                },
                "\"2..0\" isn't a version the App Store takes",
            ),
        ];

        for (app, message) in cases {
            let mut info = InfoPlist::parse(XML.as_bytes()).unwrap();

            let error = info.rewrite(&app).unwrap_err();

            assert!(error.to_string().starts_with(message), "{error}");
        }
        let mut info = InfoPlist::parse(XML.as_bytes()).unwrap();
        let versions = App {
            id: Some("dev.example.my-app".to_owned()),
            version: Some("3".to_owned()),
            ..App::default()
        };
        assert!(info.rewrite(&versions).is_ok());
    }

    #[test]
    fn rejects_what_isnt_an_info_plist() {
        let not_plist = InfoPlist::parse(b"not a plist").unwrap_err();
        let not_dictionary = InfoPlist::parse(b"<plist><array/></plist>").unwrap_err();

        assert_eq!(not_plist.to_string(), "it isn't a property list");
        assert_eq!(
            not_dictionary.to_string(),
            "it isn't a dictionary of keys and values"
        );
    }
}
