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
    /// The hosts of the app's website whose links open the app, such as
    /// `tides.example.com`, once the website vouches for the app: App Links
    /// on Android, an intent filter on the launcher activity, and universal
    /// links on iOS, an associated domain the app claims.
    #[serde(rename = "webLinks")]
    pub web_links: Option<Vec<String>>,
    /// Which way up the app's screen is on phones: the launcher activity's
    /// on Android, and the iPhone's orientations in `Info.plist` on iOS.
    pub orientation: Option<Orientation>,
    /// The app's values in other languages, by language tag, such as `fr`,
    /// `pt-BR` or `zh-Hans`, which a device shows in the language it's set
    /// to: a string resource's on Android, and `InfoPlist.strings` in the
    /// language's folder on iOS.
    pub translations: Option<BTreeMap<String, Translation>>,
    /// What only Android apps have.
    #[serde(default)]
    pub android: Android,
    /// What only iOS apps have.
    #[serde(default)]
    pub ios: Ios,
}

/// The app's values in one language.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Translation {
    /// The name under the app's icon.
    pub name: Option<String>,
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
    /// Whether the app uses encryption that export regulations don't
    /// exempt, `ITSAppUsesNonExemptEncryption` in `Info.plist`, which App
    /// Store Connect asks of every build that doesn't say.
    pub uses_non_exempt_encryption: Option<bool>,
    /// The code Apple gives an app that does, once it has reviewed the
    /// app's export compliance documents: `ITSEncryptionExportComplianceCode`.
    pub encryption_export_compliance_code: Option<String>,
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
/// other (App Links, universal links), which `webLinks` sets up.
pub fn check_url_scheme(scheme: &str) -> Result<()> {
    if matches!(scheme, "http" | "https") {
        bail!(
            "\"{scheme}\" is the web's: give the website's host in webLinks instead, whose links open the app once the website vouches for it"
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

/// Checks a host of the app's website the app config gives, as both
/// platforms take one: a domain name in lowercase, which Android matches
/// as it is, such as `tides.example.com`, or `*.example.com` for its
/// subdomains. No scheme, port or path: the website's own files say which
/// of its paths open the app.
pub fn check_web_link(host: &str) -> Result<()> {
    let name = host.strip_prefix("*.").unwrap_or(host);
    let labels: Vec<&str> = name.split('.').collect();
    let valid = labels.len() > 1
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                })
        });
    if !valid {
        bail!(
            "\"{host}\" isn't a website's host: give its domain name in lowercase, such as tides.example.com, or *.example.com for its subdomains, with no scheme or path"
        );
    }
    Ok(())
}

/// Checks a language tag the app config gives, as both platforms take
/// one: a language, then its script and region if it needs them, as BCP 47
/// writes them (`fr`, `pt-BR`, `zh-Hans`, `es-419`). Android's resource
/// tables have room for no more.
pub fn check_language_tag(tag: &str) -> Result<()> {
    let mut subtags = tag.split('-').peekable();
    let language = subtags.next().unwrap_or_default();
    let mut valid = (2..=3).contains(&language.len())
        && language
            .chars()
            .all(|character| character.is_ascii_lowercase());
    // A script is a capital letter, then three lowercase ones.
    if let Some(script) = subtags.next_if(|subtag| subtag.len() == 4) {
        let mut characters = script.chars();
        valid &= characters
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
            && characters.all(|character| character.is_ascii_lowercase());
    }
    // A region is two capital letters, or three digits for one larger than
    // a country.
    if let Some(region) = subtags.next() {
        valid &= (region.len() == 2
            && region
                .chars()
                .all(|character| character.is_ascii_uppercase()))
            || (region.len() == 3 && region.chars().all(|character| character.is_ascii_digit()));
    }
    if !valid || subtags.next().is_some() {
        bail!(
            "\"{tag}\" isn't a language tag OmniBuild takes: give a language, then its script and region if it needs them, such as fr, pt-BR or zh-Hans"
        );
    }
    Ok(())
}

/// The value among `values`, by language tag, that fits the language `tag`
/// best: its own, or that of the tag it falls back to, dropping subtags
/// from its end, as RFC 4647's lookup does: `zh-Hant-TW`, then `zh-Hant`,
/// then `zh`.
pub fn closest<'a>(values: &BTreeMap<&str, &'a str>, tag: &str) -> Option<&'a str> {
    let mut tag = tag;
    loop {
        if let Some(&value) = values.get(tag) {
            return Some(value);
        }
        tag = &tag[..tag.rfind('-')?];
    }
}

impl App {
    /// Checks the language tags of the app's translations.
    pub fn check_translations(&self) -> Result<()> {
        self.translations
            .iter()
            .flatten()
            .try_for_each(|(tag, _)| check_language_tag(tag))
    }

    /// The app's name in other languages, by language tag.
    pub fn translated_names(&self) -> BTreeMap<&str, &str> {
        self.translations
            .iter()
            .flatten()
            .filter_map(|(tag, translation)| Some((tag.as_str(), translation.name.as_deref()?)))
            .collect()
    }

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

    use std::collections::BTreeMap;

    use super::{
        AdaptiveIcon, Android, App, Background, Bundle, Entitlement, Ios, Orientation, Translation,
        check_language_tag, check_url_scheme, check_web_link, closest,
    };

    #[test]
    fn reads_an_app_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r#"{ "id": "dev.example.app", "name": "Example", "version": "2.5.0", "build": 42,
                "icon": "icon.png", "payload": "bundle", "urlSchemes": ["example"],
                "webLinks": ["example.com"], "orientation": "landscape",
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
                web_links: Some(vec!["example.com".to_owned()]),
                orientation: Some(Orientation::Landscape),
                translations: None,
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
                    ..Ios::default()
                },
            }
        );
    }

    #[test]
    fn reads_the_apps_name_in_other_languages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r#"{ "name": "Tides", "translations": { "fr": { "name": "Marées" }, "de": {} } }"#,
        )
        .unwrap();

        let app = App::read(&path).unwrap();

        assert_eq!(
            app.translations,
            Some(BTreeMap::from([
                ("de".to_owned(), Translation::default()),
                (
                    "fr".to_owned(),
                    Translation {
                        name: Some("Marées".to_owned())
                    }
                ),
            ]))
        );
        // A language with no name has none to give.
        assert_eq!(app.translated_names(), BTreeMap::from([("fr", "Marées")]));
        assert!(app.check_translations().is_ok());
        let mut wrong = app;
        wrong
            .translations
            .as_mut()
            .unwrap()
            .insert("FR".to_owned(), Translation::default());
        assert!(
            wrong
                .check_translations()
                .unwrap_err()
                .to_string()
                .starts_with("\"FR\" isn't a language tag OmniBuild takes")
        );
    }

    #[test]
    fn checks_language_tags() {
        for tag in ["fr", "fil", "pt-BR", "zh-Hans", "zh-Hant-TW", "es-419"] {
            assert!(check_language_tag(tag).is_ok(), "{tag}");
        }
        for tag in [
            "",
            "f",
            "germ",
            "FR",
            "fr_FR",
            "fr-fr",
            "pt-br",
            "zh-hans",
            "zh-HANS",
            "es-41",
            "es-4190",
            "fr-FR-x",
            "fr-",
            "de-DE-1901",
        ] {
            assert_eq!(
                check_language_tag(tag).unwrap_err().to_string(),
                format!(
                    "\"{tag}\" isn't a language tag OmniBuild takes: give a language, then its script and region if it needs them, such as fr, pt-BR or zh-Hans"
                ),
                "{tag}"
            );
        }
    }

    #[test]
    fn finds_the_value_that_fits_a_language_best() {
        let values = BTreeMap::from([("fr", "French"), ("zh-Hant", "Traditional Chinese")]);

        assert_eq!(closest(&values, "fr"), Some("French"));
        assert_eq!(closest(&values, "fr-CA"), Some("French"));
        assert_eq!(closest(&values, "zh-Hant-TW"), Some("Traditional Chinese"));
        assert_eq!(closest(&values, "zh"), None);
        assert_eq!(closest(&values, "de-DE"), None);
    }

    #[test]
    fn reads_what_the_app_says_about_its_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r#"{ "ios": { "usesNonExemptEncryption": true, "encryptionExportComplianceCode": "a code" } }"#,
        )
        .unwrap();

        assert_eq!(
            App::read(&path).unwrap().ios,
            Ios {
                uses_non_exempt_encryption: Some(true),
                encryption_export_compliance_code: Some("a code".to_owned()),
                ..Ios::default()
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
    fn takes_a_websites_hosts_as_both_platforms_do() {
        for host in ["tides.example.com", "*.example.com", "a-1.example", "x.y"] {
            check_web_link(host).unwrap();
        }
        let long = format!("{}.example.com", "a".repeat(64));
        for host in [
            "Tides.example.com",
            "https://tides.example.com",
            "tides.example.com/forecast",
            "tides.example.com:8443",
            "localhost",
            "-tides.example.com",
            "tides-.example.com",
            "tides..example.com",
            "*.",
            "",
            &long,
        ] {
            let error = check_web_link(host).unwrap_err().to_string();

            assert!(
                error.starts_with(&format!("\"{host}\" isn't a website's host")),
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
