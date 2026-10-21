//! Repackaging iOS app shells, which are zipped: a `.app` is a folder, and
//! its executable's mode wouldn't survive being copied around as one. A
//! build for devices comes as an `.ipa`, a zip with the `.app` in
//! `Payload/`.
//!
//! Changing an app's files breaks the signature Xcode gave it, so OmniBuild
//! signs the app again: a Simulator build ad hoc, as Xcode signs one, and a
//! build for devices with the user's key and provisioning profile.

mod asset_catalog;
mod bom;
mod code_resources;
mod code_signature;
mod entitlements;
mod info_plist;
mod macho;
mod profile;
mod signing;

use std::path::Path;

use anyhow::{Context, Result, bail};
use image::RgbaImage;

use crate::config::App;
use crate::key::SigningKey;
use crate::zip::{self, Entry, Method};
use crate::{icon, payload};
use info_plist::InfoPlist;
use profile::Profile;
use signing::Identity;

/// The platforms in `CFBundleSupportedPlatforms` of a build for the
/// Simulator, and of one for devices.
const SIMULATOR: &str = "iPhoneSimulator";
const DEVICES: &str = "iPhoneOS";
/// Where in its bundle an app has its extensions, each `Name.appex/`: app
/// extensions in `PlugIns/`, and in `Extensions/` those `ExtensionKit`
/// loads, as Xcode's templates now make many (App Intents, Background
/// Assets). iOS treats both alike as far as repackaging goes: each needs
/// an ID under the app's, and a signature of its own.
const EXTENSION_FOLDERS: [&str; 2] = ["PlugIns/", "Extensions/"];
/// Where an app has its App Clips, each `Name.app/`: a part of the app that
/// people run without installing it, which also needs an ID under the
/// app's (`dev.example.app.Clip`, as Xcode names one), and a signature of
/// its own, naming the app as its parent.
const APP_CLIPS: &str = "AppClips/";
/// Where an app has its watch app, `Name.app/`, which an Apple Watch paired
/// with the phone installs: its ID is under the app's too
/// (`dev.example.app.watchkitapp`, as Xcode names one), it names the app it
/// pairs with, and it has a signature of its own.
const WATCH: &str = "Watch/";

/// The folder in the shell's entries that holds the app, if it's an iOS
/// app: `Name.app/`, in a zipped `.app`, or `Payload/Name.app/`, in an
/// `.ipa`. It's the `.app` with an Info.plist, and not one inside another
/// (a watch app, say).
pub fn bundle(entries: &[Entry]) -> Option<String> {
    entries.iter().find_map(|entry| {
        let folder = entry.name.strip_suffix("Info.plist")?;
        let app = folder.strip_prefix("Payload/").unwrap_or(folder);
        (app.ends_with(".app/") && app.matches('/').count() == 1).then(|| folder.to_owned())
    })
}

/// Repackages an iOS app shell, given its entries and the folder in them
/// that holds the app, into the app `app` describes. A build for devices is
/// signed with `key` and provisioning profiles, `profiles`, one for the app
/// and each of its extensions, or one that covers them all, which a
/// Simulator build doesn't need.
pub fn repackage(
    mut entries: Vec<Entry>,
    bundle: &str,
    app: &App,
    key: Option<&SigningKey>,
    profiles: &[Vec<u8>],
) -> Result<Vec<u8>> {
    let name = format!("{bundle}Info.plist");
    let index = entries
        .iter()
        .position(|entry| entry.name == name)
        .expect("the bundle is where an Info.plist is");
    let mut info = entries[index]
        .contents()
        .and_then(|contents| InfoPlist::parse(&contents))
        .with_context(|| format!("couldn't read the shell's {name}"))?;
    let platforms = info.platforms();
    let profiles = if platforms.contains(&SIMULATOR) {
        Vec::new()
    } else if platforms.contains(&DEVICES) {
        if key.is_none() || profiles.is_empty() {
            bail!(
                "a build for devices has to be signed with a key Apple issued a certificate for, and a provisioning profile allowing it: give them with --signing-key and --profile"
            );
        }
        profiles
            .iter()
            .map(|profile| Profile::parse(profile))
            .collect::<Result<Vec<_>>>()
            .context("couldn't read a provisioning profile")?
    } else {
        bail!(
            "the shell isn't built for iOS devices or the Simulator: its CFBundleSupportedPlatforms is [{}]",
            platforms.join(", ")
        );
    };
    let shell = info.clone();
    info.rewrite(app)
        .with_context(|| format!("couldn't rewrite the shell's {name}"))?;
    // Left as it was when nothing changed, rather than written again.
    if info != shell {
        entries[index] = entries[index].with_contents(&info.to_bytes());
    }
    let shell_id = shell
        .id()
        .context("the shell's Info.plist has no bundle ID (CFBundleIdentifier)")?;
    rewrite_nested(&mut entries, bundle, shell_id, app)?;
    drop_translations(&mut entries, bundle, &InfoPlist::translated(app))?;
    if let Some(path) = &app.icon {
        set_icon(&mut entries, bundle, &shell, &icon::load(path)?)?;
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, bundle, payload)?;
    }
    let identity = key.filter(|_| !profiles.is_empty()).map(|key| Identity {
        key,
        profiles: &profiles,
    });
    let entries = signing::sign(entries, bundle, &info, identity.as_ref())?;
    Ok(zip::write(&entries).bytes)
}

/// Gives each app extension, App Clip and watch app in the shell, and each
/// extension in those in turn, such as a watch app's widgets, an ID under
/// the app's, in place of the shell's (`shell_id`), and the app's version
/// and build number.
fn rewrite_nested(entries: &mut [Entry], bundle: &str, shell_id: &str, app: &App) -> Result<()> {
    for program in signing::programs(entries, bundle)? {
        let name = format!("{program}Info.plist");
        let entry = entries
            .iter_mut()
            .find(|entry| entry.name == name)
            .with_context(|| format!("the shell has no {name}"))?;
        let shell = entry
            .contents()
            .and_then(|contents| InfoPlist::parse(&contents))
            .with_context(|| format!("couldn't read the shell's {}", entry.name))?;
        let mut info = shell.clone();
        info.rewrite_nested(shell_id, app)
            .with_context(|| format!("couldn't rewrite the shell's {}", entry.name))?;
        // Left as it was when nothing changed, rather than written again.
        if info != shell {
            *entry = entry.with_contents(&info.to_bytes());
        }
    }
    Ok(())
}

/// Takes the keys out of the shell's translations of its Info.plist,
/// `InfoPlist.strings` in each language's folder, which iOS shows in the
/// language it runs in, in place of the app's values. Xcode writes them as
/// property lists, as it does Info.plist.
fn drop_translations(entries: &mut [Entry], bundle: &str, keys: &[&str]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    for entry in entries.iter_mut().filter(|entry| {
        entry
            .name
            .strip_prefix(bundle)
            .and_then(|file| file.strip_suffix(".lproj/InfoPlist.strings"))
            .is_some_and(|language| !language.contains('/'))
    }) {
        let mut strings = entry
            .contents()
            .and_then(|contents| InfoPlist::parse(&contents))
            .with_context(|| {
                format!(
                    "couldn't read the shell's {}: OmniBuild reads the property lists Xcode makes of .strings files, not the text it makes them from",
                    entry.name
                )
            })?;
        if strings.remove(keys) {
            *entry = entry.with_contents(&strings.to_bytes());
        }
    }
    Ok(())
}

/// Gives the app its icon: in the asset catalog, where the home screen and
/// the App Store take it from, and in the PNGs that Xcode writes beside it
/// for earlier versions of iOS, which Info.plist names.
fn set_icon(entries: &mut [Entry], bundle: &str, info: &InfoPlist, icon: &RgbaImage) -> Result<()> {
    let name = info.icon_name().context(
        "the shell's Info.plist doesn't name its icon in the asset catalog (CFBundleIconName)",
    )?;
    let catalog = format!("{bundle}Assets.car");
    let entry = entries
        .iter_mut()
        .find(|entry| entry.name == catalog)
        .with_context(|| format!("the shell has no {catalog}, which its icon is in"))?;
    let car = entry
        .contents()
        .and_then(|car| asset_catalog::set_icon(&car, name, icon))
        .with_context(|| format!("couldn't give the app its icon in the shell's {catalog}"))?;
    *entry = entry.with_contents(&car);

    let files = info.icon_files();
    for entry in entries.iter_mut().filter(|entry| {
        entry
            .name
            .strip_prefix(bundle)
            .is_some_and(|file| files.iter().any(|base| is_icon_file(file, base)))
    }) {
        let png = entry
            .contents()
            .and_then(|png| icon::resized(icon, &png))
            .with_context(|| format!("couldn't replace the shell's icon {}", entry.name))?;
        *entry = entry.with_contents(&png);
    }
    Ok(())
}

/// Whether `file`, in the app's bundle, is a PNG of the icon that
/// `CFBundleIconFiles` lists as `base`: Xcode adds its scale and device, as
/// in `AppIcon76x76@2x~ipad.png`.
fn is_icon_file(file: &str, base: &str) -> bool {
    file.strip_prefix(base)
        .and_then(|variant| variant.strip_suffix(".png"))
        .is_some_and(|variant| variant.starts_with(['@', '~']))
}

/// Adds the payload's files to the app's bundle, replacing the shell's own
/// files of the same name. The app finds them as resources of its main
/// bundle, as it would files Xcode copied there.
fn add_payload(entries: &mut Vec<Entry>, bundle: &str, folder: &Path) -> Result<()> {
    for (name, contents) in payload::read(folder)? {
        let name = format!("{bundle}{name}");
        match entries.iter().position(|entry| entry.name == name) {
            Some(index) => entries[index] = entries[index].with_contents(&contents),
            None => entries.push(Entry::new(&name, Method::Deflated, &contents)),
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;

    use image::Rgba;
    use plist::Value;

    use super::asset_catalog::tests::icon_bitmap;
    use super::code_signature::tests::{blobs, identity};
    use super::profile::tests::{PROFILE, app_clip_profile};
    use super::{InfoPlist, bundle, repackage};
    use crate::bytes::Reader;
    use crate::config::{App, Ios};
    use crate::icon::tests::solid_png;
    use crate::zip::{self, Entry, Method, tests::SHELL, tests::entry};

    /// The iOS E2E shell, built for the Simulator and for devices, as an
    /// `.ipa`: see tests/fixtures/README.md.
    pub const SHELL_APP: &[u8] = include_bytes!("../../tests/fixtures/shell.app.zip");
    pub const SHELL_IPA: &[u8] = include_bytes!("../../tests/fixtures/shell.ipa");
    const INFO_PLIST: &str = "OmniBuildShell.app/Info.plist";
    const EXECUTABLE: &str = "OmniBuildShell.app/OmniBuildShell";
    const CATALOG: &str = "OmniBuildShell.app/Assets.car";
    /// The shell's English translation of its Info.plist.
    const STRINGS: &str = "OmniBuildShell.app/en.lproj/InfoPlist.strings";
    const EXTENSION_INFO: &str = "OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist";
    /// The `ExtensionKit` extension's, in Extensions/.
    const INTENTS_INFO: &str = "OmniBuildShell.app/Extensions/ShellIntents.appex/Info.plist";
    /// The App Clip's, in `AppClips/`.
    const APP_CLIP_INFO: &str = "OmniBuildShell.app/AppClips/ShellClip.app/Info.plist";
    /// The watch app's, in `Watch/`.
    const WATCH_INFO: &str = "OmniBuildShell.app/Watch/ShellWatch.app/Info.plist";
    /// The watch app's widgets', in its own `PlugIns/`.
    const WIDGETS_INFO: &str =
        "OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/Info.plist";
    /// The colour of every pixel of the app's icon in the tests.
    const ICON: Rgba<u8> = Rgba([0xe8, 0x59, 0x0c, 0xff]);

    fn file<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        entries.iter().find(|entry| entry.name == name).unwrap()
    }

    /// One of the shell's files, as Xcode wrote it.
    pub fn shell_file(name: &str) -> Vec<u8> {
        file(&zip::read(SHELL_APP).unwrap(), name)
            .contents()
            .unwrap()
    }

    /// One of the device shell's files.
    pub fn device_file(name: &str) -> Vec<u8> {
        file(&zip::read(SHELL_IPA).unwrap(), name)
            .contents()
            .unwrap()
    }

    /// The shell's Info.plist, as Xcode wrote it.
    pub fn shell_info_plist() -> Vec<u8> {
        shell_file(INFO_PLIST)
    }

    /// The shell with one of its files replaced by `replacement`, of the
    /// same name.
    fn with_file(replacement: &Entry) -> Vec<Entry> {
        zip::read(SHELL_APP)
            .unwrap()
            .into_iter()
            .map(|entry| {
                if entry.name == replacement.name {
                    replacement.clone()
                } else {
                    entry
                }
            })
            .collect()
    }

    /// The shell's Info.plist with `change` made to it.
    fn info_plist_with(change: impl FnOnce(&mut plist::Dictionary)) -> Entry {
        let mut info = Value::from_reader(Cursor::new(shell_info_plist())).unwrap();
        change(info.as_dictionary_mut().unwrap());
        let mut bytes = Vec::new();
        info.to_writer_binary(&mut bytes).unwrap();
        Entry::new(INFO_PLIST, Method::Deflated, &bytes)
    }

    /// An app config with only an icon, a PNG of one colour, written in
    /// `dir`.
    fn app_with_icon(dir: &Path) -> App {
        let path = dir.join("icon.png");
        fs::write(&path, solid_png(512, 512, ICON.0)).unwrap();
        App {
            icon: Some(path),
            ..App::default()
        }
    }

    /// The file's Unix mode, from its zip attributes.
    fn mode(entry: &Entry) -> u32 {
        entry.attributes >> 16
    }

    #[test]
    fn finds_the_app_in_a_zipped_app_or_an_ipa() {
        let app = zip::read(SHELL_APP).unwrap();
        let ipa: Vec<Entry> = app
            .iter()
            .map(|entry| Entry {
                name: format!("Payload/{}", entry.name),
                ..entry.clone()
            })
            .collect();
        let nested = [
            entry("Watch.app/Info.plist", Method::Stored, b""),
            entry("Payload/Info.plist", Method::Stored, b""),
            entry(
                "Shell.app/PlugIns/Share.appex/Info.plist",
                Method::Stored,
                b"",
            ),
            entry("Shell.app/Watch/Watch.app/Info.plist", Method::Stored, b""),
        ];

        assert_eq!(bundle(&app).unwrap(), "OmniBuildShell.app/");
        assert_eq!(bundle(&ipa).unwrap(), "Payload/OmniBuildShell.app/");
        assert_eq!(bundle(&nested).unwrap(), "Watch.app/");
        assert_eq!(bundle(&nested[1..]), None);
        assert_eq!(bundle(&zip::read(SHELL).unwrap()), None);
    }

    #[test]
    fn signs_the_shell_again_without_other_changes() {
        let shell = zip::read(SHELL_APP).unwrap();

        let app = repackage(
            shell.clone(),
            "OmniBuildShell.app/",
            &App::default(),
            None,
            &[],
        )
        .unwrap();

        let app = zip::read(&app).unwrap();
        // Signed ad hoc, as Xcode signed it: its executable, the
        // framework's and the extensions', and the seals that cover theirs,
        // the app's, the watch app's and the App Clip's. Their own seals
        // are the same as Xcode's, for the same files.
        let signed = [
            EXECUTABLE,
            "OmniBuildShell.app/Frameworks/ShellKit.framework/ShellKit",
            "OmniBuildShell.app/PlugIns/ShellNotifications.appex/ShellNotifications",
            "OmniBuildShell.app/Extensions/ShellIntents.appex/ShellIntents",
            "OmniBuildShell.app/AppClips/ShellClip.app/Frameworks/ShellKit.framework/ShellKit",
            "OmniBuildShell.app/AppClips/ShellClip.app/ShellClip",
            "OmniBuildShell.app/AppClips/ShellClip.app/_CodeSignature/CodeResources",
            "OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets",
            "OmniBuildShell.app/Watch/ShellWatch.app/ShellWatch",
            "OmniBuildShell.app/Watch/ShellWatch.app/_CodeSignature/CodeResources",
            "OmniBuildShell.app/_CodeSignature/CodeResources",
        ];
        for (entry, shell_entry) in app.iter().zip(&shell) {
            assert_eq!(entry.name, shell_entry.name);
            let unchanged = entry.contents().unwrap() == shell_entry.contents().unwrap();
            assert_eq!(
                unchanged,
                !signed.contains(&entry.name.as_str()),
                "{}",
                entry.name
            );
        }
        assert_eq!(app.len(), shell.len());
        // ditto's modes, which the executable needs to run.
        assert_eq!(
            mode(file(&app, "OmniBuildShell.app/OmniBuildShell")),
            0o100_755
        );
        assert_eq!(mode(file(&app, "OmniBuildShell.app/")), 0o40_755);
    }

    #[test]
    fn gives_the_app_its_values_and_payload() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("bundle")).unwrap();
        fs::write(dir.path().join("payload.txt"), "the app's payload").unwrap();
        fs::write(dir.path().join("bundle/main.lynx.bundle"), "bundle").unwrap();
        let app = App {
            id: Some("dev.example.app".to_owned()),
            name: Some("Repackaged".to_owned()),
            version: Some("2.5.0".to_owned()),
            build: Some(42),
            payload: Some(dir.path().to_owned()),
            ios: Ios {
                usage_descriptions: Some(
                    [(
                        "NFCReaderUsageDescription".to_owned(),
                        "Reads tags.".to_owned(),
                    )]
                    .into(),
                ),
            },
            ..App::default()
        };
        let shell = zip::read(SHELL_APP).unwrap();

        let repackaged = repackage(shell.clone(), "OmniBuildShell.app/", &app, None, &[]).unwrap();

        let entries = zip::read(&repackaged).unwrap();
        let mut expected = InfoPlist::parse(&shell_info_plist()).unwrap();
        expected.rewrite(&app).unwrap();
        let info = file(&entries, INFO_PLIST);
        assert_eq!(
            InfoPlist::parse(&info.contents().unwrap()).unwrap(),
            expected
        );
        assert_eq!(mode(info), 0o100_644);
        // The English name goes, so iOS shows the app's, but the English
        // reason for the camera, which the app keeps, stays.
        let strings = |entries: &[Entry]| -> plist::Dictionary {
            let strings = file(entries, STRINGS).contents().unwrap();
            assert!(strings.starts_with(b"bplist00"));
            Value::from_reader(Cursor::new(strings))
                .unwrap()
                .into_dictionary()
                .unwrap()
        };
        let mut english = strings(&shell);
        english.remove("CFBundleDisplayName");
        assert_eq!(strings(&entries), english);
        assert!(english.contains_key("NSCameraUsageDescription"));
        let payload = file(&entries, "OmniBuildShell.app/payload.txt");
        assert_eq!(payload.contents().unwrap(), b"the app's payload");
        let bundle = file(&entries, "OmniBuildShell.app/bundle/main.lynx.bundle");
        assert_eq!(bundle.contents().unwrap(), b"bundle");
        assert_eq!(mode(bundle), 0o100_644);
        // The payload goes after the shell's files.
        let names = |entries: &[Entry]| -> Vec<String> {
            entries.iter().map(|entry| entry.name.clone()).collect()
        };
        assert_eq!(names(&entries)[..shell.len()], names(&shell));
        assert_eq!(mode(file(&entries, EXECUTABLE)), 0o100_755);
        // Each extension's ID under the app's, and the App Clip's and the
        // watch app's, and the app's version and build, in PlugIns/ and in
        // Extensions/ alike, and the watch app's widgets' under the watch
        // app's.
        for (info, id) in [
            (EXTENSION_INFO, "dev.example.app.ShellNotifications"),
            (INTENTS_INFO, "dev.example.app.ShellIntents"),
            (APP_CLIP_INFO, "dev.example.app.Clip"),
            (WATCH_INFO, "dev.example.app.watchkitapp"),
            (
                WIDGETS_INFO,
                "dev.example.app.watchkitapp.ShellWatchWidgets",
            ),
        ] {
            let extension = InfoPlist::parse(&file(&entries, info).contents().unwrap()).unwrap();
            let mut expected = InfoPlist::parse(&file(&shell, info).contents().unwrap()).unwrap();
            expected
                .rewrite_nested("dev.omnibuild.shell", &app)
                .unwrap();
            assert_eq!(extension, expected);
            assert_eq!(extension.id(), Some(id));
        }
        // And the watch app pairs with the app.
        let watch_app =
            Value::from_reader(Cursor::new(file(&entries, WATCH_INFO).contents().unwrap()))
                .unwrap()
                .into_dictionary()
                .unwrap();
        assert_eq!(
            watch_app
                .get("WKCompanionAppBundleIdentifier")
                .and_then(Value::as_string),
            Some("dev.example.app")
        );
    }

    #[test]
    fn takes_the_apps_purpose_strings_out_of_the_translations() {
        let app = App {
            ios: Ios {
                usage_descriptions: Some(
                    [(
                        "NSCameraUsageDescription".to_owned(),
                        "Scans codes.".to_owned(),
                    )]
                    .into(),
                ),
            },
            ..App::default()
        };
        // In XML too, as Xcode can write them.
        let mut english = Vec::new();
        Value::from_reader(Cursor::new(shell_file(STRINGS)))
            .unwrap()
            .to_writer_xml(&mut english)
            .unwrap();
        let mut shell = with_file(&Entry::new(STRINGS, Method::Deflated, &english));
        // A folder's own strings, which aren't the app's.
        shell.push(Entry::new(
            "OmniBuildShell.app/Settings.bundle/en.lproj/InfoPlist.strings",
            Method::Deflated,
            b"not a plist",
        ));

        let entries =
            zip::read(&repackage(shell, "OmniBuildShell.app/", &app, None, &[]).unwrap()).unwrap();

        let strings = file(&entries, STRINGS).contents().unwrap();
        assert!(strings.starts_with(b"<?xml"));
        let strings = Value::from_reader(Cursor::new(strings)).unwrap();
        let strings = strings.as_dictionary().unwrap();
        assert!(!strings.contains_key("NSCameraUsageDescription"));
        assert!(strings.contains_key("CFBundleDisplayName"));
        let info = InfoPlist::parse(&file(&entries, INFO_PLIST).contents().unwrap()).unwrap();
        let mut expected = InfoPlist::parse(&shell_info_plist()).unwrap();
        expected.rewrite(&app).unwrap();
        assert_eq!(info, expected);

        // A purpose the translations don't have leaves them as they were.
        let nfc = App {
            ios: Ios {
                usage_descriptions: Some(
                    [(
                        "NFCReaderUsageDescription".to_owned(),
                        "Reads tags.".to_owned(),
                    )]
                    .into(),
                ),
            },
            ..App::default()
        };
        let entries = zip::read(
            &repackage(
                zip::read(SHELL_APP).unwrap(),
                "OmniBuildShell.app/",
                &nfc,
                None,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            file(&entries, STRINGS).contents().unwrap(),
            shell_file(STRINGS)
        );
    }

    #[test]
    fn reports_translations_and_purpose_strings_it_cant_use() {
        // As Xcode writes a .strings file when told to keep it as text.
        let text = with_file(&Entry::new(
            STRINGS,
            Method::Deflated,
            b"\"CFBundleDisplayName\" = \"Shell\";",
        ));
        let named = App {
            name: Some("Repackaged".to_owned()),
            ..App::default()
        };
        let not_a_purpose = App {
            ios: Ios {
                usage_descriptions: Some(
                    [("NSCamera".to_owned(), "Scans codes.".to_owned())].into(),
                ),
            },
            ..App::default()
        };

        let unread = repackage(text, "OmniBuildShell.app/", &named, None, &[]).unwrap_err();
        let unset = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &not_a_purpose,
            None,
            &[],
        )
        .unwrap_err();

        assert_eq!(
            unread.to_string(),
            "couldn't read the shell's OmniBuildShell.app/en.lproj/InfoPlist.strings: OmniBuild reads the property lists Xcode makes of .strings files, not the text it makes them from"
        );
        assert_eq!(
            format!("{unset:#}"),
            "couldn't rewrite the shell's OmniBuildShell.app/Info.plist: \"NSCamera\" isn't the key of a purpose string, such as NSCameraUsageDescription"
        );
    }

    #[test]
    fn gives_the_app_its_icon_in_the_catalog_and_the_pngs_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let shell = zip::read(SHELL_APP).unwrap();

        let repackaged = repackage(
            shell.clone(),
            "OmniBuildShell.app/",
            &app_with_icon(dir.path()),
            None,
            &[],
        )
        .unwrap();

        let entries = zip::read(&repackaged).unwrap();
        let (_, _, _, pixels) = icon_bitmap(&file(&entries, CATALOG).contents().unwrap());
        assert_eq!(pixels[..4], [0x0c, 0x59, 0xe8, 0xff], "BGRA");
        for (name, size) in [
            ("OmniBuildShell.app/AppIcon60x60@2x.png", 120),
            ("OmniBuildShell.app/AppIcon76x76@2x~ipad.png", 152),
        ] {
            let png = image::load_from_memory(&file(&entries, name).contents().unwrap())
                .unwrap()
                .into_rgba8();
            assert_eq!(png.dimensions(), (size, size), "{name}");
            assert_eq!(png.get_pixel(size / 2, size / 2), &ICON, "{name}");
        }
        assert_eq!(file(&entries, INFO_PLIST), file(&shell, INFO_PLIST));
    }

    /// The App Clip's profile for the app `dev.example.app`.
    fn example_app_clip_profile() -> Vec<u8> {
        app_clip_profile(
            "ABCDE12345.dev.example.app.Clip",
            "ABCDE12345.dev.example.app",
        )
    }

    /// The device shell repackaged as `dev.example.app`, and signed with
    /// the test key, a wildcard profile for the app and the code nested in
    /// it, and the App Clip's own.
    fn signed_for_devices() -> Vec<Entry> {
        let app = App {
            id: Some("dev.example.app".to_owned()),
            ..App::default()
        };
        let ipa = repackage(
            zip::read(SHELL_IPA).unwrap(),
            "Payload/OmniBuildShell.app/",
            &app,
            Some(&identity()),
            &[PROFILE.to_vec(), example_app_clip_profile()],
        )
        .unwrap();
        zip::read(&ipa).unwrap()
    }

    /// The entitlements in the signature of the device app's `executable`,
    /// in the code at `code` in it, whose `LC_CODE_SIGNATURE` is at
    /// `load_command`.
    fn entitlements_in(
        entries: &[Entry],
        executable: &str,
        code: usize,
        load_command: usize,
    ) -> String {
        let executable = file(entries, &format!("Payload/OmniBuildShell.app/{executable}"))
            .contents()
            .unwrap();
        let executable = &executable[code..];
        let signature = Reader::at(executable, load_command + 8).u32() as usize;
        let (_, entitlements) = blobs(&executable[signature..])
            .into_iter()
            .find(|(slot, _)| *slot == 5)
            .unwrap();
        String::from_utf8_lossy(&entitlements).into_owned()
    }

    #[test]
    fn signs_a_build_for_devices_with_the_key_and_profile() {
        let entries = signed_for_devices();

        assert_eq!(
            file(
                &entries,
                "Payload/OmniBuildShell.app/embedded.mobileprovision"
            )
            .contents()
            .unwrap(),
            PROFILE
        );
        let executable = file(&entries, "Payload/OmniBuildShell.app/OmniBuildShell");
        assert_eq!(mode(executable), 0o100_755);
        // The signatures' entitlements are for the app's own ID, and each
        // extension's, which is under it, each in its arm64 code, at the
        // start of the file.
        let entitlements = |executable: &str| entitlements_in(&entries, executable, 0, 784);
        assert!(
            entitlements("OmniBuildShell").contains("<string>ABCDE12345.dev.example.app</string>")
        );
        for (extension, name) in [
            ("PlugIns/ShellNotifications.appex/", "ShellNotifications"),
            ("Extensions/ShellIntents.appex/", "ShellIntents"),
        ] {
            assert!(
                entitlements(&format!("{extension}{name}")).contains(&format!(
                    "<string>ABCDE12345.dev.example.app.{name}</string>"
                ))
            );
            assert_eq!(
                file(
                    &entries,
                    &format!("Payload/OmniBuildShell.app/{extension}embedded.mobileprovision")
                )
                .contents()
                .unwrap(),
                PROFILE
            );
        }
        // The App Clip and the app name each other, by their App IDs.
        let clip = "AppClips/ShellClip.app/";
        assert!(entitlements(&format!("{clip}ShellClip")).contains(
            "\t<key>application-identifier</key>
\t<string>ABCDE12345.dev.example.app.Clip</string>
\t<key>com.apple.developer.on-demand-install-capable</key>
\t<true/>
\t<key>com.apple.developer.parent-application-identifiers</key>
\t<array>
\t\t<string>ABCDE12345.dev.example.app</string>
\t</array>
"
        ));
        assert!(entitlements("OmniBuildShell").contains(
            "\t<key>com.apple.developer.associated-appclip-app-identifiers</key>
\t<array>
\t\t<string>ABCDE12345.dev.example.app.Clip</string>
\t</array>
"
        ));
        assert_eq!(
            file(
                &entries,
                &format!("Payload/OmniBuildShell.app/{clip}embedded.mobileprovision")
            )
            .contents()
            .unwrap(),
            example_app_clip_profile()
        );
    }

    #[test]
    fn signs_a_watch_app_and_its_widgets_for_their_own_ids() {
        let entries = signed_for_devices();

        // The watch app's and its widgets' entitlements, in their arm64
        // code and their arm64_32 code after it, each for its own ID under
        // the app's, from the wildcard.
        for (executable, id) in [
            ("Watch/ShellWatch.app/ShellWatch", "watchkitapp"),
            (
                "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets",
                "watchkitapp.ShellWatchWidgets",
            ),
        ] {
            for (code, load_command) in [(16_384, 784), (65_536, 696)] {
                assert!(
                    entitlements_in(&entries, executable, code, load_command)
                        .contains(&format!("<string>ABCDE12345.dev.example.app.{id}</string>"))
                );
            }
        }
    }

    #[test]
    fn refuses_a_build_for_devices_without_a_key_and_profile() {
        let key = identity();
        let device = "a build for devices has to be signed with a key Apple issued a certificate for, and a provisioning profile allowing it: give them with --signing-key and --profile";
        let mac = with_file(&info_plist_with(|info| {
            info.insert(
                "CFBundleSupportedPlatforms".to_owned(),
                Value::Array(vec![Value::String("MacOSX".to_owned())]),
            );
        }));
        let cases = [
            (zip::read(SHELL_IPA).unwrap(), None, vec![], device),
            (zip::read(SHELL_IPA).unwrap(), Some(&key), vec![], device),
            (
                zip::read(SHELL_IPA).unwrap(),
                None,
                vec![PROFILE.to_vec()],
                device,
            ),
            (
                zip::read(SHELL_IPA).unwrap(),
                Some(&key),
                vec![PROFILE.to_vec(), b"not a profile".to_vec()],
                "couldn't read a provisioning profile: it isn't a provisioning profile, a property list in CMS signed data",
            ),
            (
                mac,
                None,
                vec![],
                "the shell isn't built for iOS devices or the Simulator: its CFBundleSupportedPlatforms is [MacOSX]",
            ),
        ];

        for (shell, key, profiles, message) in cases {
            let bundle = super::bundle(&shell).unwrap();

            let error = repackage(shell, &bundle, &App::default(), key, &profiles).unwrap_err();

            assert_eq!(format!("{error:#}"), message);
        }
    }

    #[test]
    fn reports_an_app_extension_it_cant_rewrite() {
        let cases = [
            (
                with_file(&info_plist_with(|info| {
                    info.remove("CFBundleIdentifier");
                })),
                App::default(),
                "the shell's Info.plist has no bundle ID (CFBundleIdentifier)",
            ),
            (
                with_file(&Entry::new(EXTENSION_INFO, Method::Stored, b"not a plist")),
                App::default(),
                "couldn't read the shell's OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist: it isn't a property list",
            ),
            (
                with_file(&Entry::new(
                    EXTENSION_INFO,
                    Method::Stored,
                    b"<plist><dict><key>CFBundleIdentifier</key><string>dev.other</string></dict></plist>",
                )),
                App {
                    id: Some("dev.example.app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist: its bundle ID (CFBundleIdentifier) isn't under the app's, dev.omnibuild.shell",
            ),
            (
                with_file(&Entry::new(
                    INTENTS_INFO,
                    Method::Stored,
                    b"<plist><dict><key>CFBundleIdentifier</key><string>dev.omnibuild.shellIntents</string></dict></plist>",
                )),
                App {
                    id: Some("dev.example.app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/Extensions/ShellIntents.appex/Info.plist: its bundle ID (CFBundleIdentifier) isn't under the app's, dev.omnibuild.shell",
            ),
            (
                with_file(&Entry::new(
                    APP_CLIP_INFO,
                    Method::Stored,
                    b"<plist><dict><key>CFBundleIdentifier</key><string>dev.omnibuild.Clip</string></dict></plist>",
                )),
                App {
                    id: Some("dev.example.app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/AppClips/ShellClip.app/Info.plist: its bundle ID (CFBundleIdentifier) isn't under the app's, dev.omnibuild.shell",
            ),
            (
                with_file(&Entry::new(
                    WATCH_INFO,
                    Method::Stored,
                    b"<plist><dict>
                        <key>CFBundleIdentifier</key><string>dev.omnibuild.shell.watchkitapp</string>
                        <key>WKCompanionAppBundleIdentifier</key><string>dev.omnibuild.other</string>
                    </dict></plist>",
                )),
                App {
                    id: Some("dev.example.app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/Watch/ShellWatch.app/Info.plist: it's a watch app for another app than the shell: its WKCompanionAppBundleIdentifier isn't dev.omnibuild.shell",
            ),
        ];

        for (shell, app, message) in cases {
            let error = repackage(shell, "OmniBuildShell.app/", &app, None, &[]).unwrap_err();

            assert!(format!("{error:#}").starts_with(message), "{error:#}");
        }
    }

    #[test]
    fn reports_code_nested_deeper_it_cant_rewrite() {
        let cases = [
            (
                with_file(&Entry::new(
                    WIDGETS_INFO,
                    Method::Stored,
                    b"<plist><dict><key>CFBundleIdentifier</key><string>dev.omnibuild.watchkitapp.ShellWatchWidgets</string></dict></plist>",
                )),
                App {
                    id: Some("dev.example.app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/Info.plist: its bundle ID (CFBundleIdentifier) isn't under the app's, dev.omnibuild.shell",
            ),
            (
                zip::read(SHELL_APP)
                    .unwrap()
                    .into_iter()
                    .filter(|entry| entry.name != WIDGETS_INFO)
                    .collect(),
                App::default(),
                "the shell has no OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/Info.plist",
            ),
            (
                zip::read(SHELL_APP)
                    .unwrap()
                    .into_iter()
                    .chain([Entry::new(
                        "OmniBuildShell.app/PlugIns/Share.bundle/Share",
                        Method::Stored,
                        b"",
                    )])
                    .collect(),
                App::default(),
                "the shell has code in its bundle that isn't a framework, an app extension, or the app's App Clip or watch app (OmniBuildShell.app/PlugIns/Share.bundle/Share), which OmniBuild can't sign yet",
            ),
        ];

        for (shell, app, message) in cases {
            let error = repackage(shell, "OmniBuildShell.app/", &app, None, &[]).unwrap_err();

            assert_eq!(format!("{error:#}"), message);
        }
    }

    #[test]
    fn reports_an_app_it_cant_repackage() {
        let mut corrupt = Entry::new(INFO_PLIST, Method::Deflated, b"");
        corrupt.data = vec![0xff; 8];
        let dir = tempfile::tempdir().unwrap();
        let with_icon = app_with_icon(dir.path());
        let cases = [
            (
                with_file(&Entry::new(INFO_PLIST, Method::Deflated, b"not a plist")),
                App::default(),
                "couldn't read the shell's OmniBuildShell.app/Info.plist: it isn't a property list",
            ),
            (
                with_file(&corrupt),
                App::default(),
                "couldn't read the shell's OmniBuildShell.app/Info.plist: OmniBuildShell.app/Info.plist is corrupt: corrupt deflate stream",
            ),
            (
                zip::read(SHELL_APP).unwrap(),
                App {
                    icon: Some(dir.path().join("missing.png")),
                    ..App::default()
                },
                "couldn't read the icon ",
            ),
            (
                with_file(&info_plist_with(|info| {
                    info.remove("CFBundleIcons");
                })),
                app_with_icon(dir.path()),
                "the shell's Info.plist doesn't name its icon in the asset catalog (CFBundleIconName)",
            ),
            (
                zip::read(SHELL_APP)
                    .unwrap()
                    .into_iter()
                    .filter(|entry| entry.name != CATALOG)
                    .collect(),
                app_with_icon(dir.path()),
                "the shell has no OmniBuildShell.app/Assets.car, which its icon is in",
            ),
            (
                with_file(&Entry::new(CATALOG, Method::Stored, b"not a catalog")),
                app_with_icon(dir.path()),
                "couldn't give the app its icon in the shell's OmniBuildShell.app/Assets.car: it isn't a BOM store",
            ),
            (
                with_file(&Entry::new(
                    "OmniBuildShell.app/AppIcon60x60@2x.png",
                    Method::Stored,
                    b"not a PNG",
                )),
                with_icon,
                "couldn't replace the shell's icon OmniBuildShell.app/AppIcon60x60@2x.png: it isn't a PNG",
            ),
            (
                zip::read(SHELL_APP).unwrap(),
                App {
                    id: Some("dev.my_app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/Info.plist: \"dev.my_app\" isn't a valid iOS bundle ID: it can hold only letters, digits, hyphens and dots",
            ),
            (
                with_file(&Entry::new(
                    EXECUTABLE,
                    Method::Stored,
                    b"not an executable",
                )),
                App::default(),
                "couldn't sign OmniBuildShell.app/OmniBuildShell: it isn't a 64-bit Mach-O executable, nor one for arm64_32",
            ),
        ];

        for (shell, app, message) in cases {
            let error = repackage(shell, "OmniBuildShell.app/", &app, None, &[]).unwrap_err();

            // The first ends with the plist parser's own description.
            assert!(format!("{error:#}").starts_with(message), "{error:#}");
        }
        let missing_payload = App {
            payload: Some(dir.path().join("missing")),
            ..App::default()
        };
        let error = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &missing_payload,
            None,
            &[],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("couldn't read the payload folder ")
        );
    }
}
