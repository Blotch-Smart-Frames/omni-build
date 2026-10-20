//! The app config: the values that turn a shell into an app.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
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
    /// What only Android apps have.
    #[serde(default)]
    pub android: Android,
    /// What only iOS apps have.
    #[serde(default)]
    pub ios: Ios,
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

    use super::{AdaptiveIcon, Android, App, Background, Ios};

    #[test]
    fn reads_an_app_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r#"{ "id": "dev.example.app", "name": "Example", "version": "2.5.0", "build": 42,
                "icon": "icon.png", "payload": "bundle",
                "android": {
                    "adaptiveIcon": { "foreground": "front.png", "background": "back.png", "monochrome": "mono.png" },
                    "permissions": ["CAMERA"], "abis": ["arm64-v8a"]
                },
                "ios": { "usageDescriptions": { "NSCameraUsageDescription": "Scans codes." } } }"#,
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
    fn reports_a_config_it_cant_read_or_understand() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        let colour = dir.path().join("colour.json");
        fs::write(&path, r#"{ "id": "dev.example.app", "colour": "blue" }"#).unwrap();

        let missing = App::read(&dir.path().join("missing.json")).unwrap_err();
        let unknown_field = App::read(&path).unwrap_err();
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
