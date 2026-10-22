//! The app config: the values that turn a shell into an app.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// What the config gives is changed in the shell. What it leaves out stays
/// as the shell has it.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct App {
    /// The application ID on Android, the bundle ID on iOS.
    pub id: Option<String>,
    /// The name under the app's icon.
    pub name: Option<String>,
    /// The version people see: `versionName` on Android,
    /// `CFBundleShortVersionString` on iOS.
    pub version: Option<String>,
    /// The build number stores compare: `versionCode` on Android,
    /// `CFBundleVersion` on iOS.
    pub build: Option<u32>,
    /// The app's icon: a square PNG, which OmniBuild resizes to each size
    /// the shell has its icon in: each density's on Android, and on iOS
    /// each in the asset catalog and the PNGs beside it.
    pub icon: Option<PathBuf>,
    /// A folder of files the app reads at run time, such as its Lynx
    /// bundle: they go in the APK's `assets/`, or in the iOS app's bundle,
    /// replacing the shell's files of the same name.
    pub payload: Option<PathBuf>,
    /// The URL schemes that open the app, such as `myapp` for
    /// `myapp://path`, as deep links: an intent filter on the launcher
    /// activity on Android, and a URL type in `Info.plist` on iOS. They're
    /// added to any the shell has.
    #[serde(rename = "urlSchemes")]
    pub url_schemes: Option<Vec<String>>,
    /// Which way up the app's screen is on phones: the launcher activity's
    /// on Android, and the iPhone's orientations in `Info.plist` on iOS.
    pub orientation: Option<Orientation>,
    /// What only Android apps have.
    #[serde(default)]
    pub android: Android,
    /// What only iOS apps have.
    #[serde(default)]
    pub ios: Ios,
}

/// Which way up an app's screen is on phones. Tablets turn every app every
/// way, or soon will: Apple asks iPad apps to support every orientation,
/// and Android 16 ignores an app's on large screens once it targets it.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    /// Upright.
    Portrait,
    /// On its side, either way round.
    Landscape,
    /// Turning with the phone, as the system turns apps by default:
    /// upright or on either side, but not upside down.
    Any,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Android {
    /// The layers of the icon Android 8.0 and later show, which launchers
    /// crop to their own shape. Without them, an adaptive icon in the shell
    /// gets the app's `icon` whole, inside the shape.
    pub adaptive_icon: Option<AdaptiveIcon>,
    /// The permissions the app asks for (`<uses-permission>`), such as
    /// `android.permission.CAMERA`, or `CAMERA` for short. The shell's
    /// others go, but those under its own application ID, which its code
    /// relies on. Without them, the shell's.
    pub permissions: Option<Vec<String>>,
    /// The ABIs the app keeps native libraries for, such as `arm64-v8a`:
    /// the shell's others go. Without them, the shell's.
    pub abis: Option<Vec<String>>,
}

/// An adaptive icon's layers, each a square PNG of the whole layer (108dp,
/// of which launchers show the middle 72dp).
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdaptiveIcon {
    pub foreground: PathBuf,
    pub background: Background,
    /// The layer Android 13 and later tint to match the system's colours,
    /// of which only the alpha counts. Without it, the foreground's.
    pub monochrome: Option<PathBuf>,
}

/// An adaptive icon's background: a colour or a PNG.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum Background {
    /// Red, green and blue.
    Color([u8; 3]),
    Image(PathBuf),
}

impl TryFrom<String> for Background {
    type Error = String;

    /// `#RRGGBB` is a colour; anything else, a PNG's path.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let Some(hex) = value.strip_prefix('#') else {
            return Ok(Self::Image(value.into()));
        };
        match u32::from_str_radix(hex, 16) {
            Ok(rgb) if hex.len() == 6 => {
                let [_, red, green, blue] = rgb.to_be_bytes();
                Ok(Self::Color([red, green, blue]))
            }
            _ => Err(format!(
                "\"{value}\" isn't a colour: give one as #RRGGBB, such as #2F6FEB"
            )),
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Ios {
    /// Why the app uses what iOS asks its user to allow, by `Info.plist`
    /// key, such as `NSCameraUsageDescription`. They replace the shell's,
    /// in every language it has them in.
    pub usage_descriptions: Option<BTreeMap<String, String>>,
    /// The entitlements the app claims for the capabilities it uses, such
    /// as `aps-environment` for push notifications, as an Xcode project's
    /// `.entitlements` file has them. A build for devices is signed with
    /// them, and its provisioning profile has to allow each.
    pub entitlements: Option<BTreeMap<String, Entitlement>>,
    /// What the bundles nested in the app have of their own: its app
    /// extensions, App Clips and watch app, and the extensions in those,
    /// such as a watch app's widgets. Each by the part of its ID after the
    /// app's, which is the same in the shell, such as `ShellNotifications`
    /// for `dev.example.app.ShellNotifications`.
    pub bundles: Option<BTreeMap<String, Bundle>>,
}

/// What a bundle nested in the app has of its own.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// The entitlements it claims for the capabilities it uses, as the
    /// app's own do, such as the App Group it shares with the app. Its own
    /// provisioning profile has to allow each.
    pub entitlements: Option<BTreeMap<String, Entitlement>>,
}

/// An entitlement's value, of the kinds capabilities take.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(
    untagged,
    expecting = "an entitlement is a string, a boolean, or an array of strings"
)]
pub enum Entitlement {
    String(String),
    Boolean(bool),
    /// Such as the App IDs an App Clip and its app name each other by.
    Strings(Vec<String>),
}

/// Checks a URL scheme the app config gives, as both platforms take one:
/// RFC 3986's letters, in lowercase, which Android matches as they are
/// while the RFC and iOS ignore their case. The web's are refused: a web
/// link opens an app only once the app and its website vouch for each
/// other (App Links, universal links).
pub fn check_url_scheme(scheme: &str) -> Result<()> {
    if matches!(scheme, "http" | "https") {
        bail!(
            "\"{scheme}\" is the web's: web links open an app as universal links, for which an iOS app claims its website's domain in ios.entitlements, and App Links, which OmniBuild doesn't set up yet"
        );
    }
    let mut characters = scheme.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '+' | '-' | '.')
        });
    if !valid {
        bail!(
            "\"{scheme}\" isn't a URL scheme: it needs a lowercase letter first, then lowercase letters, digits, +, - or ., such as myapp"
        );
    }
    Ok(())
}

impl App {
    /// Reads an app config from a JSON file. The paths in it are relative
    /// to the file's folder.
    pub fn read(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path)
            .with_context(|| format!("couldn't read the app config {}", path.display()))?;
        let app: Self = serde_json::from_str(&json)
            .with_context(|| format!("the app config {} isn't valid", path.display()))?;
        let folder = path.parent().unwrap_or(path);
        let adaptive_icon = app.android.adaptive_icon.map(|icon| AdaptiveIcon {
            foreground: folder.join(icon.foreground),
            background: match icon.background {
                Background::Image(image) => Background::Image(folder.join(image)),
                color @ Background::Color(_) => color,
            },
            monochrome: icon.monochrome.map(|monochrome| folder.join(monochrome)),
        });
        Ok(Self {
            icon: app.icon.map(|icon| folder.join(icon)),
            payload: app.payload.map(|payload| folder.join(payload)),
            android: Android {
                adaptive_icon,
                ..app.android
            },
            ..app
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        AdaptiveIcon, Android, App, Background, Bundle, Entitlement, Ios, Orientation,
        check_url_scheme,
    };

    #[test]
    fn reads_an_app_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r#"{ "id": "dev.example.app", "name": "Example", "version": "2.5.0", "build": 42,
                "icon": "icon.png", "payload": "bundle", "urlSchemes": ["example"],
                "orientation": "landscape",
                "android": {
                    "adaptiveIcon": { "foreground": "front.png", "background": "back.png", "monochrome": "mono.png" },
                    "permissions": ["CAMERA"], "abis": ["arm64-v8a"]
                },
                "ios": {
                    "usageDescriptions": { "NSCameraUsageDescription": "Scans codes." },
                    "entitlements": {
                        "aps-environment": "development",
                        "com.apple.developer.associated-domains": ["applinks:example.com"],
                        "com.apple.developer.kernel.increased-memory-limit": true
                    },
                    "bundles": {
                        "ShellNotifications": {
                            "entitlements": { "com.apple.security.application-groups": ["group.dev.example"] }
                        },
                        "Clip": {}
                    }
                } }"#,
        )
        .unwrap();

        assert_eq!(
            App::read(&path).unwrap(),
            App {
                id: Some("dev.example.app".to_owned()),
                name: Some("Example".to_owned()),
                version: Some("2.5.0".to_owned()),
                build: Some(42),
                // Relative to the config's folder.
                icon: Some(dir.path().join("icon.png")),
                payload: Some(dir.path().join("bundle")),
                url_schemes: Some(vec!["example".to_owned()]),
                orientation: Some(Orientation::Landscape),
                android: Android {
                    adaptive_icon: Some(AdaptiveIcon {
                        foreground: dir.path().join("front.png"),
                        background: Background::Image(dir.path().join("back.png")),
                        monochrome: Some(dir.path().join("mono.png")),
                    }),
                    permissions: Some(vec!["CAMERA".to_owned()]),
                    abis: Some(vec!["arm64-v8a".to_owned()]),
                },
                ios: Ios {
                    usage_descriptions: Some(
                        [(
                            "NSCameraUsageDescription".to_owned(),
                            "Scans codes.".to_owned()
                        )]
                        .into()
                    ),
                    entitlements: Some(
                        [
                            (
                                "aps-environment".to_owned(),
                                Entitlement::String("development".to_owned())
                            ),
                            (
                                "com.apple.developer.associated-domains".to_owned(),
                                Entitlement::Strings(vec!["applinks:example.com".to_owned()])
                            ),
                            (
                                "com.apple.developer.kernel.increased-memory-limit".to_owned(),
                                Entitlement::Boolean(true)
                            ),
                        ]
                        .into()
                    ),
                    bundles: Some(
                        [
                            ("Clip".to_owned(), Bundle::default()),
                            (
                                "ShellNotifications".to_owned(),
                                Bundle {
                                    entitlements: Some(
                                        [(
                                            "com.apple.security.application-groups".to_owned(),
                                            Entitlement::Strings(vec![
                                                "group.dev.example".to_owned()
                                            ])
                                        )]
                                        .into()
                                    )
                                }
                            ),
                        ]
                        .into()
                    ),
                },
            }
        );
    }

    #[test]
    fn reads_a_background_colour() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r##"{ "android": { "adaptiveIcon": { "foreground": "front.png", "background": "#2F6FEb" } } }"##,
        )
        .unwrap();

        let icon = App::read(&path).unwrap().android.adaptive_icon.unwrap();

        assert_eq!(icon.background, Background::Color([0x2f, 0x6f, 0xeb]));
        assert_eq!(icon.monochrome, None);
    }

    #[test]
    fn takes_url_schemes_as_both_platforms_do() {
        for scheme in ["myapp", "my-app.v2+beta", "x"] {
            check_url_scheme(scheme).unwrap();
        }
        for (scheme, message) in [
            ("MyApp", "isn't a URL scheme"),
            ("2app", "isn't a URL scheme"),
            ("my_app", "isn't a URL scheme"),
            ("", "isn't a URL scheme"),
            ("https", "is the web's"),
            ("http", "is the web's"),
        ] {
            let error = check_url_scheme(scheme).unwrap_err().to_string();

            assert!(
                error.starts_with(&format!("\"{scheme}\" {message}")),
                "{error}"
            );
        }
    }

    #[test]
    fn reports_a_config_it_cant_read_or_understand() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        let colour = dir.path().join("colour.json");
        let orientation = dir.path().join("orientation.json");
        let entitlement = dir.path().join("entitlement.json");
        fs::write(&path, r#"{ "id": "dev.example.app", "colour": "blue" }"#).unwrap();
        fs::write(&orientation, r#"{ "orientation": "upside down" }"#).unwrap();
        fs::write(
            &entitlement,
            r#"{ "ios": { "entitlements": { "com.apple.developer.healthkit.access": [1] } } }"#,
        )
        .unwrap();

        let missing = App::read(&dir.path().join("missing.json")).unwrap_err();
        let unknown_field = App::read(&path).unwrap_err();
        let unknown_orientation = App::read(&orientation).unwrap_err();
        let odd_entitlement = App::read(&entitlement).unwrap_err();
        let bad_colours: Vec<String> = ["#2F6FE", "#2F6FEG", "#2F6FEB00"]
            .iter()
            .map(|background| {
                fs::write(
                    &colour,
                    format!(
                        r#"{{ "android": {{ "adaptiveIcon": {{ "foreground": "f.png", "background": "{background}" }} }} }}"#
                    ),
                )
                .unwrap();
                format!("{:#}", App::read(&colour).unwrap_err())
            })
            .collect();

        assert!(
            missing
                .to_string()
                .starts_with("couldn't read the app config ")
        );
        assert!(format!("{unknown_field:#}").contains("unknown field `colour`"));
        assert!(
            format!("{unknown_orientation:#}").contains(
                "unknown variant `upside down`, expected one of `portrait`, `landscape`, `any`"
            ),
            "{unknown_orientation:#}"
        );
        assert!(
            format!("{odd_entitlement:#}")
                .contains("an entitlement is a string, a boolean, or an array of strings"),
            "{odd_entitlement:#}"
        );
        for (error, background) in bad_colours.iter().zip(["#2F6FE", "#2F6FEG", "#2F6FEB00"]) {
            assert!(
                error.contains(&format!(
                    "\"{background}\" isn't a colour: give one as #RRGGBB"
                )),
                "{error}"
            );
        }
    }
}
