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
/// signed with `key` and the provisioning profile `profile`, which a
/// Simulator build doesn't need.
pub fn repackage(
    mut entries: Vec<Entry>,
    bundle: &str,
    app: &App,
    key: Option<&SigningKey>,
    profile: Option<&[u8]>,
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
    let profile = if platforms.contains(&SIMULATOR) {
        None
    } else if platforms.contains(&DEVICES) {
        let (Some(_), Some(profile)) = (key, profile) else {
            bail!(
                "a build for devices has to be signed with a key Apple issued a certificate for, and a provisioning profile allowing it: give them with --signing-key and --profile"
            );
        };
        Some(Profile::parse(profile).context("couldn't read the provisioning profile")?)
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
    if let Some(path) = &app.icon {
        set_icon(&mut entries, bundle, &shell, &icon::load(path)?)?;
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, bundle, payload)?;
    }
    let identity = profile
        .as_ref()
        .zip(key)
        .map(|(profile, key)| Identity { key, profile });
    let entries = signing::sign(entries, bundle, &info, identity.as_ref())?;
    Ok(zip::write(&entries).bytes)
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
    use super::profile::tests::PROFILE;
    use super::{InfoPlist, bundle, repackage};
    use crate::bytes::Reader;
    use crate::config::App;
    use crate::icon::tests::solid_png;
    use crate::zip::{self, Entry, Method, tests::SHELL, tests::entry};

    /// The iOS E2E shell, built for the Simulator and for devices, as an
    /// `.ipa`: see tests/fixtures/README.md.
    pub const SHELL_APP: &[u8] = include_bytes!("../../tests/fixtures/shell.app.zip");
    pub const SHELL_IPA: &[u8] = include_bytes!("../../tests/fixtures/shell.ipa");
    const INFO_PLIST: &str = "OmniBuildShell.app/Info.plist";
    const EXECUTABLE: &str = "OmniBuildShell.app/OmniBuildShell";
    const CATALOG: &str = "OmniBuildShell.app/Assets.car";
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
            None,
        )
        .unwrap();

        let app = zip::read(&app).unwrap();
        // Signed ad hoc, as Xcode signed it. The seal is the same as
        // Xcode's, for the same files.
        for (entry, shell_entry) in app.iter().zip(&shell) {
            assert_eq!(entry.name, shell_entry.name);
            let unchanged = entry.contents().unwrap() == shell_entry.contents().unwrap();
            assert_eq!(unchanged, entry.name != EXECUTABLE, "{}", entry.name);
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
            ..App::default()
        };
        let shell = zip::read(SHELL_APP).unwrap();

        let repackaged = repackage(shell.clone(), "OmniBuildShell.app/", &app, None, None).unwrap();

        let entries = zip::read(&repackaged).unwrap();
        let mut expected = InfoPlist::parse(&shell_info_plist()).unwrap();
        expected.rewrite(&app).unwrap();
        let info = file(&entries, INFO_PLIST);
        assert_eq!(
            InfoPlist::parse(&info.contents().unwrap()).unwrap(),
            expected
        );
        assert_eq!(mode(info), 0o100_644);
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
            None,
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

    #[test]
    fn signs_a_build_for_devices_with_the_key_and_profile() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let app = App {
            id: Some("dev.example.app".to_owned()),
            ..App::default()
        };

        let ipa = repackage(
            shell,
            "Payload/OmniBuildShell.app/",
            &app,
            Some(&identity()),
            Some(PROFILE),
        )
        .unwrap();

        let entries = zip::read(&ipa).unwrap();
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
        // The signature's entitlements are for the app's own ID.
        let executable = executable.contents().unwrap();
        let signature = Reader::at(&executable, 784 + 8).u32() as usize;
        let (_, entitlements) = blobs(&executable[signature..])
            .into_iter()
            .find(|(slot, _)| *slot == 5)
            .unwrap();
        assert!(
            String::from_utf8_lossy(&entitlements)
                .contains("<string>ABCDE12345.dev.example.app</string>")
        );
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
            (zip::read(SHELL_IPA).unwrap(), None, None, device),
            (zip::read(SHELL_IPA).unwrap(), Some(&key), None, device),
            (
                zip::read(SHELL_IPA).unwrap(),
                Some(&key),
                Some(&b"not a profile"[..]),
                "couldn't read the provisioning profile: it isn't a provisioning profile, a property list in CMS signed data",
            ),
            (
                mac,
                None,
                None,
                "the shell isn't built for iOS devices or the Simulator: its CFBundleSupportedPlatforms is [MacOSX]",
            ),
        ];

        for (shell, key, profile, message) in cases {
            let bundle = super::bundle(&shell).unwrap();

            let error = repackage(shell, &bundle, &App::default(), key, profile).unwrap_err();

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
                "couldn't sign OmniBuildShell.app/OmniBuildShell: it isn't a 64-bit Mach-O executable",
            ),
        ];

        for (shell, app, message) in cases {
            let error = repackage(shell, "OmniBuildShell.app/", &app, None, None).unwrap_err();

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
            None,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("couldn't read the payload folder ")
        );
    }
}
