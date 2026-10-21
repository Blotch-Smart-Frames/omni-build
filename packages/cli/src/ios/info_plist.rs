//! An app's `Info.plist`, where iOS reads its ID, name, version and build
//! number, among everything else it needs to know before running it, and
//! its translations, `InfoPlist.strings` in each language's folder, which
//! Xcode writes in the same format.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

use crate::config::{App, Orientation, check_url_scheme};

/// The start of a binary property list, the format Xcode writes an app's
/// Info.plist in.
const BINARY: &[u8] = b"bplist00";
/// The ID of the iPhone app a watch app pairs with, which watchOS installs
/// it for.
const WATCH_COMPANION: &str = "WKCompanionAppBundleIdentifier";
/// The orientations an iPhone turns the app to, which it reads over those
/// an iPad reads too (`UISupportedInterfaceOrientations`), as iOS reads a
/// key for one kind of device over the key for all. Xcode's templates
/// write both kinds' own.
const IPHONE_ORIENTATIONS: &str = "UISupportedInterfaceOrientations~iphone";

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
        if let Some(schemes) = &app.url_schemes {
            self.add_url_schemes(schemes)?;
        }
        if let Some(orientation) = app.orientation {
            self.set_orientation(orientation);
        }
        for (key, description) in app.ios.usage_descriptions.iter().flatten() {
            if !is_usage_description(key) {
                bail!(
                    "\"{key}\" isn't the key of a purpose string, such as NSCameraUsageDescription"
                );
            }
            self.set(key, description);
        }
        Ok(())
    }

    /// Has iOS open the app's URLs with these schemes, as deep links: a URL
    /// type (`CFBundleURLTypes`) for those the shell's don't have, named
    /// after the app's ID, as Xcode names one.
    fn add_url_schemes(&mut self, schemes: &[String]) -> Result<()> {
        for scheme in schemes {
            check_url_scheme(scheme)?;
        }
        let existing: Vec<&str> = self
            .dictionary
            .get("CFBundleURLTypes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_dictionary)
            .filter_map(|url_type| url_type.get("CFBundleURLSchemes"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_string)
            .collect();
        let mut added: Vec<&str> = Vec::new();
        for scheme in schemes {
            if !existing.contains(&scheme.as_str()) && !added.contains(&scheme.as_str()) {
                added.push(scheme);
            }
        }
        if added.is_empty() {
            return Ok(());
        }
        let url_type: Dictionary = [
            ("CFBundleTypeRole", Value::String("Editor".to_owned())),
            (
                "CFBundleURLName",
                Value::String(self.id().unwrap_or_default().to_owned()),
            ),
            (
                "CFBundleURLSchemes",
                Value::Array(
                    added
                        .iter()
                        .map(|&scheme| Value::String(scheme.to_owned()))
                        .collect(),
                ),
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        match self
            .dictionary
            .get_mut("CFBundleURLTypes")
            .and_then(Value::as_array_mut)
        {
            Some(types) => types.push(Value::Dictionary(url_type)),
            None => {
                self.dictionary.insert(
                    "CFBundleURLTypes".to_owned(),
                    Value::Array(vec![Value::Dictionary(url_type)]),
                );
            }
        }
        Ok(())
    }

    /// Turns the app's screen on an iPhone as `orientation` says. An iPad
    /// keeps the shell's orientations: Apple asks iPad apps to support
    /// every one, so that they can share the screen, and Xcode warns that it
    /// will soon require it.
    fn set_orientation(&mut self, orientation: Orientation) {
        let orientations: &[&str] = match orientation {
            Orientation::Portrait => &["UIInterfaceOrientationPortrait"],
            Orientation::Landscape => &[
                "UIInterfaceOrientationLandscapeLeft",
                "UIInterfaceOrientationLandscapeRight",
            ],
            // As Xcode's templates have them: iPhones with Face ID don't
            // turn apps upside down.
            Orientation::Any => &[
                "UIInterfaceOrientationPortrait",
                "UIInterfaceOrientationLandscapeLeft",
                "UIInterfaceOrientationLandscapeRight",
            ],
        };
        self.dictionary.insert(
            IPHONE_ORIENTATIONS.to_owned(),
            Value::Array(
                orientations
                    .iter()
                    .map(|&orientation| Value::String(orientation.to_owned()))
                    .collect(),
            ),
        );
    }

    /// Gives an app extension, an App Clip or a watch app what follows
    /// from the app's values, as Xcode would have built it for the app: its
    /// ID under the app's in place of the shell's (`dev.example.app.Share`
    /// for `dev.omnibuild.shell.Share`), which iOS requires, and the app's
    /// version and build number, which App Store validation requires. A
    /// watch app names the app it pairs with too, which becomes the app's
    /// ID. `shell_id` is the shell's ID.
    pub fn rewrite_nested(&mut self, shell_id: &str, app: &App) -> Result<()> {
        if let Some(id) = &app.id {
            let suffix = self
                .id()
                .and_then(|own| own.strip_prefix(shell_id))
                .filter(|suffix| suffix.starts_with('.'))
                .with_context(|| {
                    format!("its bundle ID (CFBundleIdentifier) isn't under the app's, {shell_id}")
                })?;
            let own = format!("{id}{suffix}");
            self.set("CFBundleIdentifier", &own);
            if let Some(companion) = self.dictionary.get(WATCH_COMPANION) {
                if companion.as_string() != Some(shell_id) {
                    bail!(
                        "it's a watch app for another app than the shell: its {WATCH_COMPANION} isn't {shell_id}"
                    );
                }
                self.set(WATCH_COMPANION, id);
            }
        }
        if let Some(version) = &app.version {
            self.set("CFBundleShortVersionString", version);
        }
        if let Some(build) = app.build {
            self.set("CFBundleVersion", &build.to_string());
        }
        Ok(())
    }

    /// The keys whose translations iOS would show in place of the app's
    /// values: its name's, and its purpose strings'.
    pub fn translated(app: &App) -> Vec<&str> {
        let name = app.name.as_ref().map(|_| "CFBundleDisplayName");
        let descriptions = app.ios.usage_descriptions.iter().flatten();
        name.into_iter()
            .chain(descriptions.map(|(key, _)| key.as_str()))
            .collect()
    }

    /// Removes the keys, and says whether it had any of them.
    pub fn remove(&mut self, keys: &[&str]) -> bool {
        keys.iter()
            .filter(|key| self.dictionary.remove(key).is_some())
            .count()
            > 0
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

/// Whether `key` is a purpose string's, as Apple names them:
/// `NSCameraUsageDescription`, `NFCReaderUsageDescription`.
fn is_usage_description(key: &str) -> bool {
    key.strip_suffix("UsageDescription")
        .is_some_and(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()))
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

    use super::{InfoPlist, WATCH_COMPANION};
    use crate::config::{App, Orientation};
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

    /// The schemes of each URL type, and the type's name.
    fn url_types(info: &InfoPlist) -> Vec<(String, Vec<String>)> {
        info.dictionary
            .get("CFBundleURLTypes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|url_type| {
                let url_type = url_type.as_dictionary().unwrap();
                let name = url_type.get("CFBundleURLName").and_then(Value::as_string);
                let schemes = url_type["CFBundleURLSchemes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|scheme| scheme.as_string().unwrap().to_owned())
                    .collect();
                (name.unwrap_or_default().to_owned(), schemes)
            })
            .collect()
    }

    fn with_schemes(schemes: &[&str]) -> App {
        App {
            url_schemes: Some(schemes.iter().map(|&scheme| scheme.to_owned()).collect()),
            ..app()
        }
    }

    #[test]
    fn opens_the_apps_urls() {
        let mut info = InfoPlist::parse(XML.as_bytes()).unwrap();

        info.rewrite(&with_schemes(&["myapp", "other", "myapp"]))
            .unwrap();
        let first = info.clone();
        // Again, with one the app has now, and one it hasn't.
        info.rewrite(&with_schemes(&["other", "third"])).unwrap();

        assert_eq!(
            url_types(&first),
            [(
                "dev.example.app".to_owned(),
                vec!["myapp".to_owned(), "other".to_owned()]
            )]
        );
        let role = &first.dictionary["CFBundleURLTypes"].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap()["CFBundleTypeRole"];
        assert_eq!(role.as_string(), Some("Editor"));
        assert_eq!(url_types(&info)[1].1, ["third"]);
        let mut unchanged = info.clone();
        unchanged.rewrite(&with_schemes(&["third"])).unwrap();
        assert_eq!(unchanged, info);
        assert!(
            info.clone()
                .rewrite(&with_schemes(&["https"]))
                .unwrap_err()
                .to_string()
                .starts_with("\"https\" is the web's")
        );
    }

    #[test]
    fn turns_the_apps_screen_on_an_iphone() {
        let orientations = |info: &InfoPlist, key: &str| -> Vec<String> {
            info.dictionary[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|orientation| orientation.as_string().unwrap().to_owned())
                .collect()
        };
        let mut info = InfoPlist::parse(&shell_info_plist()).unwrap();
        let shell = orientations(&info, "UISupportedInterfaceOrientations");

        let mut turned = Vec::new();
        for orientation in [
            Orientation::Portrait,
            Orientation::Landscape,
            Orientation::Any,
        ] {
            info.rewrite(&App {
                orientation: Some(orientation),
                ..App::default()
            })
            .unwrap();
            turned.push(orientations(
                &info,
                "UISupportedInterfaceOrientations~iphone",
            ));
        }

        assert_eq!(
            turned,
            [
                vec!["UIInterfaceOrientationPortrait"],
                vec![
                    "UIInterfaceOrientationLandscapeLeft",
                    "UIInterfaceOrientationLandscapeRight"
                ],
                vec![
                    "UIInterfaceOrientationPortrait",
                    "UIInterfaceOrientationLandscapeLeft",
                    "UIInterfaceOrientationLandscapeRight"
                ],
            ]
        );
        // What an iPad reads stays the shell's.
        assert_eq!(
            orientations(&info, "UISupportedInterfaceOrientations"),
            shell
        );
        assert_eq!(shell, ["UIInterfaceOrientationPortrait"]);
    }

    /// An app extension's Info.plist, with this bundle ID.
    fn extension(id: &str) -> InfoPlist {
        InfoPlist::parse(XML.replace("dev.omnibuild.shell", id).as_bytes()).unwrap()
    }

    #[test]
    fn gives_an_extension_an_id_under_the_apps_and_its_version() {
        let mut info = extension("dev.omnibuild.shell.ShellNotifications");

        info.rewrite_nested("dev.omnibuild.shell", &app()).unwrap();

        assert_eq!(
            entries(&info.to_bytes()),
            [
                (
                    "CFBundleIdentifier".to_owned(),
                    Value::String("dev.example.app.ShellNotifications".to_owned())
                ),
                (
                    "CFBundleName".to_owned(),
                    Value::String("OmniBuildShell".to_owned())
                ),
                (
                    "CFBundleShortVersionString".to_owned(),
                    Value::String("2.5.0".to_owned())
                ),
                ("CFBundleVersion".to_owned(), Value::String("42".to_owned())),
            ]
        );
        // Without the app's values, as the shell has it.
        let mut unchanged = extension("dev.omnibuild.shell.ShellNotifications");
        unchanged
            .rewrite_nested("dev.omnibuild.shell", &App::default())
            .unwrap();
        assert_eq!(
            unchanged,
            extension("dev.omnibuild.shell.ShellNotifications")
        );
    }

    #[test]
    fn gives_a_watch_app_the_app_it_pairs_with() {
        let watch_app = |companion: &str| {
            let mut info = extension("dev.omnibuild.shell.watchkitapp");
            info.set(WATCH_COMPANION, companion);
            info
        };
        let mut info = watch_app("dev.omnibuild.shell");

        info.rewrite_nested("dev.omnibuild.shell", &app()).unwrap();
        let error = watch_app("dev.omnibuild.other")
            .rewrite_nested("dev.omnibuild.shell", &app())
            .unwrap_err();

        assert_eq!(info.id(), Some("dev.example.app.watchkitapp"));
        assert_eq!(
            info.dictionary
                .get(WATCH_COMPANION)
                .and_then(Value::as_string),
            Some("dev.example.app")
        );
        assert_eq!(
            error.to_string(),
            "it's a watch app for another app than the shell: its WKCompanionAppBundleIdentifier isn't dev.omnibuild.shell"
        );
        // Without the app's ID, as the shell has it.
        let mut unchanged = watch_app("dev.omnibuild.shell");
        unchanged
            .rewrite_nested("dev.omnibuild.shell", &App::default())
            .unwrap();
        assert_eq!(unchanged, watch_app("dev.omnibuild.shell"));
    }

    #[test]
    fn refuses_an_extension_whose_id_isnt_under_the_apps() {
        for id in ["dev.omnibuild.other", "dev.omnibuild.shellfish"] {
            let error = extension(id)
                .rewrite_nested("dev.omnibuild.shell", &app())
                .unwrap_err();

            assert_eq!(
                error.to_string(),
                "its bundle ID (CFBundleIdentifier) isn't under the app's, dev.omnibuild.shell"
            );
        }
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
