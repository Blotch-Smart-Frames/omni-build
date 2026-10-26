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
mod modules;
mod profile;
mod provisioning;
mod signing;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use image::RgbaImage;

use crate::config::{App, Entitlement, check_web_link, closest};
use crate::key::SigningKey;
use crate::modules::Module;
use crate::zip::{self, Entry, Method};
use crate::{icon, payload};
use info_plist::InfoPlist;
use profile::Profile;
pub use provisioning::{Distribution, Provisioner};
use signing::{Claims, Identity};

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
/// The domains whose links open the app, and what else it shares with
/// them, as `applinks:tides.example.com`.
const ASSOCIATED_DOMAINS: &str = "com.apple.developer.associated-domains";

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
/// that holds the app, into the app `app` describes, with the native
/// `modules` the app has. A build for devices is signed with `key` and
/// provisioning profiles, `profiles`, one for the app and each of its
/// extensions, or one that covers them all, which a Simulator build doesn't
/// need.
pub fn repackage(
    mut entries: Vec<Entry>,
    bundle: &str,
    app: &App,
    modules: &[Module],
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
    let simulator = platforms.contains(&SIMULATOR);
    let profiles = if simulator {
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
    modules::add(&mut entries, bundle, &mut info, modules, simulator)?;
    // Left as it was when nothing changed, rather than written again.
    if info != shell {
        entries[index] = entries[index].with_contents(&info.to_bytes());
    }
    let shell_id = shell
        .id()
        .context("the shell's Info.plist has no bundle ID (CFBundleIdentifier)")?;
    let nested: Vec<String> = rewrite_nested(&mut entries, bundle, shell_id, app)?
        .into_iter()
        .map(|(suffix, _)| suffix)
        .collect();
    if let Some(unknown) = app
        .ios
        .bundles
        .iter()
        .flatten()
        .map(|(suffix, _)| suffix)
        .find(|suffix| !nested.contains(suffix))
    {
        bail!(
            "ios.bundles names {unknown}, which the shell has no bundle for: its app extensions, App Clips and watch apps, by the part of their ID after the app's, are [{}]",
            nested.join(", ")
        );
    }
    for host in app.web_links.iter().flatten() {
        check_web_link(host)?;
    }
    app.check_translations()?;
    drop_translations(&mut entries, bundle, &InfoPlist::translated(app))?;
    translate_name(&mut entries, bundle, &app.translated_names())?;
    if let Some(path) = &app.icon {
        set_icon(&mut entries, bundle, &shell, &icon::load(path)?)?;
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, bundle, payload)?;
    }
    let identity = key.filter(|_| !profiles.is_empty()).map(|key| Identity {
        key,
        profiles: &profiles,
        claims: claims(&info, app),
    });
    let entries = signing::sign(entries, bundle, &info, identity.as_ref())?;
    Ok(zip::write(&entries).bytes)
}

/// A bundle a build for devices signs with a provisioning profile of its
/// own: the app, or a program nested in it.
pub struct Signed {
    /// Its ID, as repackaging gives it.
    pub id: String,
    /// Whether it's an App Clip, whose App ID is registered as one.
    pub app_clip: bool,
    /// What it claims for the capabilities it uses, if anything.
    pub claims: Option<Claims>,
}

/// The bundles of the app a build for devices becomes with `app`, given
/// its entries and the folder in them that holds the app, that each need a
/// provisioning profile: the app, then each program nested in it, by the
/// IDs repackaging gives them, with what each claims.
pub fn signed_bundles(entries: &[Entry], bundle: &str, app: &App) -> Result<Vec<Signed>> {
    // A copy, which the nested bundles' Info.plists are rewritten in as
    // repackaging rewrites them, for the IDs they end up with.
    let mut entries = entries.to_vec();
    let mut info = signing::bundle_info(&entries, bundle)?;
    let platforms = info.platforms();
    if !platforms.contains(&DEVICES) {
        bail!(
            "only a build for iOS devices, an .ipa, is signed with provisioning profiles, and the shell's CFBundleSupportedPlatforms is [{}]",
            platforms.join(", ")
        );
    }
    let shell_id = signing::bundle_id(&info, bundle)?.to_owned();
    info.rewrite(app)
        .with_context(|| format!("couldn't rewrite the shell's {bundle}Info.plist"))?;
    let mut claims = claims(&info, app);
    let id = info.id().unwrap_or_default().to_owned();
    let nested = rewrite_nested(&mut entries, bundle, &shell_id, app)?;
    let app_clips = format!("{bundle}{APP_CLIPS}");
    let bundles = std::iter::once((id.clone(), false)).chain(
        nested
            .into_iter()
            .map(|(suffix, folder)| (format!("{id}.{suffix}"), folder.starts_with(&app_clips))),
    );
    Ok(bundles
        .map(|(id, app_clip)| Signed {
            claims: claims.remove(&id),
            id,
            app_clip,
        })
        .collect())
}

/// What the app and the bundles nested in it claim, by their IDs: the
/// app, described by `info`, what `ios.entitlements` gives, and the
/// domains of `webLinks`, and each bundle, what `ios.bundles` gives it.
fn claims(info: &InfoPlist, app: &App) -> BTreeMap<String, Claims> {
    let id = info.id().unwrap_or_default();
    let mut claims = BTreeMap::new();
    let mut own = app.ios.entitlements.clone();
    let mut sources: Vec<&str> = own.iter().map(|_| "ios.entitlements").collect();
    if let Some(hosts) = &app.web_links {
        // Universal links, among any other associated domains the app
        // claims, such as its shared web credentials'.
        let claimed = own
            .get_or_insert_default()
            .entry(ASSOCIATED_DOMAINS.to_owned())
            .or_insert_with(|| Entitlement::Strings(Vec::new()));
        if let Entitlement::String(domain) = claimed {
            let domain = std::mem::take(domain);
            *claimed = Entitlement::Strings(vec![domain]);
        }
        // Not if it's a boolean, which the profile refuses.
        if let Entitlement::Strings(domains) = claimed {
            for host in hosts {
                let domain = format!("applinks:{host}");
                if !domains.contains(&domain) {
                    domains.push(domain);
                }
            }
        }
        sources.push("webLinks");
    }
    if let Some(entitlements) = own {
        claims.insert(
            id.to_owned(),
            Claims {
                source: sources.join(" and "),
                entitlements,
            },
        );
    }
    for (suffix, bundle) in app.ios.bundles.iter().flatten() {
        if let Some(entitlements) = &bundle.entitlements {
            claims.insert(
                format!("{id}.{suffix}"),
                Claims {
                    source: format!("ios.bundles.{suffix}.entitlements"),
                    entitlements: entitlements.clone(),
                },
            );
        }
    }
    claims
}

/// Gives each app extension, App Clip and watch app in the shell, and each
/// extension in those in turn, such as a watch app's widgets, an ID under
/// the app's, in place of the shell's (`shell_id`), and the app's version
/// and build number. Returns the part of each one's ID after the app's,
/// and the folder it's in.
fn rewrite_nested(
    entries: &mut [Entry],
    bundle: &str,
    shell_id: &str,
    app: &App,
) -> Result<Vec<(String, String)>> {
    let mut suffixes = Vec::new();
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
        suffixes.extend(
            shell
                .id()
                .and_then(|own| own.strip_prefix(shell_id))
                .and_then(|suffix| suffix.strip_prefix('.'))
                .map(|suffix| (suffix.to_owned(), program.clone())),
        );
    }
    Ok(suffixes)
}

/// Takes the keys out of the shell's translations of its Info.plist,
/// `InfoPlist.strings` in each language's folder, which iOS shows in the
/// language it runs in, in place of the app's values. Xcode writes them as
/// property lists, as it does Info.plist.
fn drop_translations(entries: &mut [Entry], bundle: &str, keys: &[&str]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    for entry in entries
        .iter_mut()
        .filter(|entry| translation_language(&entry.name, bundle).is_some())
    {
        let mut strings = read_translation(entry)?;
        if strings.remove(keys) {
            *entry = entry.with_contents(&strings.to_bytes());
        }
    }
    Ok(())
}

/// Gives the app its name in other languages, by language tag, in the
/// translations of its Info.plist: in each the shell has, the name that
/// fits its language best, if any, and in a translation of its own, the
/// name of each language the shell has none in. iOS shows the name in the
/// language it runs in, of those the app has a folder for.
fn translate_name(
    entries: &mut Vec<Entry>,
    bundle: &str,
    names: &BTreeMap<&str, &str>,
) -> Result<()> {
    let mut translated = Vec::new();
    for entry in entries.iter_mut() {
        let Some(language) = translation_language(&entry.name, bundle) else {
            continue;
        };
        // A folder named as Xcode once named them, such as zh_CN, is a
        // language tag all the same.
        let tag = language.replace('_', "-");
        if let Some(name) = closest(names, &tag) {
            let mut strings = read_translation(entry)?;
            strings.set_display_name(name);
            *entry = entry.with_contents(&strings.to_bytes());
        }
        translated.push(tag);
    }
    for (&tag, &name) in names {
        if !translated.iter().any(|language| language == tag) {
            let strings = InfoPlist::translated_name(name).to_bytes();
            let file = format!("{bundle}{tag}.lproj/InfoPlist.strings");
            entries.push(Entry::new(&file, Method::Deflated, &strings));
        }
    }
    Ok(())
}

/// The language of a translation of the app's Info.plist, if `file` is
/// one: `InfoPlist.strings` in the language's folder, such as `fr.lproj`,
/// in the app's bundle, rather than a folder in it.
fn translation_language<'a>(file: &'a str, bundle: &str) -> Option<&'a str> {
    file.strip_prefix(bundle)
        .and_then(|file| file.strip_suffix(".lproj/InfoPlist.strings"))
        .filter(|language| !language.contains('/'))
}

/// A translation of the shell's Info.plist, which Xcode writes as a
/// property list, as it does Info.plist.
fn read_translation(entry: &Entry) -> Result<InfoPlist> {
    entry
        .contents()
        .and_then(|contents| InfoPlist::parse(&contents))
        .with_context(|| {
            format!(
                "couldn't read the shell's {}: OmniBuild reads the property lists Xcode makes of .strings files, not the text it makes them from",
                entry.name
            )
        })
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;

    use image::Rgba;
    use plist::Value;

    use super::asset_catalog::tests::icon_bitmap;
    use super::code_signature::tests::{blobs, identity};
    use super::profile::tests::{PROFILE, app_clip_profile};
    use super::{InfoPlist, bundle, claims, signed_bundles};
    use crate::bytes::Reader;
    use crate::config::{App, Bundle, Entitlement, Ios, Translation};
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

    /// Repackages the shell with no native modules, as most tests do.
    fn repackage(
        entries: Vec<Entry>,
        bundle: &str,
        app: &App,
        key: Option<&crate::key::SigningKey>,
        profiles: &[Vec<u8>],
    ) -> anyhow::Result<Vec<u8>> {
        super::repackage(entries, bundle, app, &[], key, profiles)
    }

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
                ..Ios::default()
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
                ..Ios::default()
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
                ..Ios::default()
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

    /// The app's name in these languages.
    fn translated(name: Option<&str>, names: &[(&str, &str)]) -> App {
        App {
            name: name.map(str::to_owned),
            translations: Some(
                names
                    .iter()
                    .map(|&(tag, name)| {
                        (
                            tag.to_owned(),
                            Translation {
                                name: Some(name.to_owned()),
                            },
                        )
                    })
                    .collect(),
            ),
            ..App::default()
        }
    }

    /// The name in a translation of the app's Info.plist.
    fn translated_name(entries: &[Entry], language: &str) -> Option<String> {
        let file = file(
            entries,
            &format!("OmniBuildShell.app/{language}.lproj/InfoPlist.strings"),
        );
        let strings = Value::from_reader(Cursor::new(file.contents().unwrap())).unwrap();
        strings
            .as_dictionary()
            .unwrap()
            .get("CFBundleDisplayName")
            .and_then(Value::as_string)
            .map(str::to_owned)
    }

    #[test]
    fn gives_the_app_its_name_in_other_languages() {
        let app = translated(
            Some("Tides"),
            &[
                ("en", "Tides in English"),
                ("fr", "Marées"),
                ("zh-Hans", "潮汐"),
            ],
        );

        let entries = zip::read(
            &repackage(
                zip::read(SHELL_APP).unwrap(),
                "OmniBuildShell.app/",
                &app,
                None,
                &[],
            )
            .unwrap(),
        )
        .unwrap();

        // In the shell's English, and in a translation of their own for the
        // languages the shell hasn't, as Xcode writes them.
        assert_eq!(
            translated_name(&entries, "en").as_deref(),
            Some("Tides in English")
        );
        assert_eq!(translated_name(&entries, "fr").as_deref(), Some("Marées"));
        assert_eq!(
            translated_name(&entries, "zh-Hans").as_deref(),
            Some("潮汐")
        );
        let french = file(&entries, "OmniBuildShell.app/fr.lproj/InfoPlist.strings");
        assert!(french.contents().unwrap().starts_with(b"bplist00"));
        // Sealed with the rest.
        let seal = file(&entries, "OmniBuildShell.app/_CodeSignature/CodeResources")
            .contents()
            .unwrap();
        assert!(
            seal.windows(26)
                .any(|window| window == b"fr.lproj/InfoPlist.strings")
        );

        // A shell's language with no translation of the name of its own, or
        // one close enough, shows the app's.
        let british = translated(Some("Tides"), &[("en-GB", "Tides, innit")]);
        let entries = zip::read(
            &repackage(
                zip::read(SHELL_APP).unwrap(),
                "OmniBuildShell.app/",
                &british,
                None,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(translated_name(&entries, "en"), None);
        assert_eq!(
            translated_name(&entries, "en-GB").as_deref(),
            Some("Tides, innit")
        );
        // Its own, in a language close to one the shell has, in a folder
        // named as Xcode once named them.
        let mut shell = zip::read(SHELL_APP).unwrap();
        for entry in &mut shell {
            entry.name = entry.name.replace("/en.lproj/", "/en_GB.lproj/");
        }
        let entries =
            zip::read(&repackage(shell, "OmniBuildShell.app/", &british, None, &[]).unwrap())
                .unwrap();
        assert_eq!(
            translated_name(&entries, "en_GB").as_deref(),
            Some("Tides, innit")
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.name.contains("/en-GB.lproj/"))
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
                ..Ios::default()
            },
            ..App::default()
        };

        let unread = repackage(text.clone(), "OmniBuildShell.app/", &named, None, &[]).unwrap_err();
        // A translation it can't read in a language the app has a name in.
        let untranslated = repackage(
            text,
            "OmniBuildShell.app/",
            &translated(None, &[("en", "Tides")]),
            None,
            &[],
        )
        .unwrap_err();
        let wrong_tag = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &translated(None, &[("en_GB", "Tides")]),
            None,
            &[],
        )
        .unwrap_err();
        let unset = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &not_a_purpose,
            None,
            &[],
        )
        .unwrap_err();

        for error in [unread, untranslated] {
            assert_eq!(
                error.to_string(),
                "couldn't read the shell's OmniBuildShell.app/en.lproj/InfoPlist.strings: OmniBuild reads the property lists Xcode makes of .strings files, not the text it makes them from"
            );
        }
        assert!(
            wrong_tag
                .to_string()
                .starts_with("\"en_GB\" isn't a language tag OmniBuild takes")
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

    /// A keychain group, claimed for the bundle `ios.entitlements` or
    /// `ios.bundles` names, as an app config claims it.
    fn keychain_group(group: &str) -> BTreeMap<String, Entitlement> {
        [(
            "keychain-access-groups".to_owned(),
            Entitlement::Strings(vec![group.to_owned()]),
        )]
        .into()
    }

    /// The device shell repackaged as `dev.example.app`, the app claiming a
    /// keychain group and its notification extension one of its own, and
    /// signed with the test key, a wildcard profile for the app and the
    /// code nested in it, and the App Clip's own.
    fn signed_for_devices() -> Vec<Entry> {
        let app = App {
            id: Some("dev.example.app".to_owned()),
            ios: Ios {
                entitlements: Some(keychain_group("$(AppIdentifierPrefix)dev.example.shared")),
                bundles: Some(
                    [(
                        "ShellNotifications".to_owned(),
                        Bundle {
                            entitlements: Some(keychain_group(
                                "$(AppIdentifierPrefix)$(CFBundleIdentifier)",
                            )),
                        },
                    )]
                    .into(),
                ),
                ..Ios::default()
            },
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
        // The app and its notification extension each claim the keychain
        // group they ask for, which the profile allows, the extension's
        // named after its own ID, and the other extension claims none.
        let keychain_group = |group: &str| {
            format!(
                "\t<key>keychain-access-groups</key>
\t<array>
\t\t<string>{group}</string>
\t</array>
"
            )
        };
        assert!(
            entitlements("OmniBuildShell")
                .contains(&keychain_group("ABCDE12345.dev.example.shared"))
        );
        assert!(
            entitlements("PlugIns/ShellNotifications.appex/ShellNotifications").contains(
                &keychain_group("ABCDE12345.dev.example.app.ShellNotifications")
            )
        );
        assert!(
            !entitlements("Extensions/ShellIntents.appex/ShellIntents")
                .contains("keychain-access-groups")
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
    fn checks_entitlements_against_the_profile_only_for_devices() {
        let app = App {
            ios: Ios {
                entitlements: Some(
                    [(
                        "aps-environment".to_owned(),
                        Entitlement::String("development".to_owned()),
                    )]
                    .into(),
                ),
                ..Ios::default()
            },
            ..App::default()
        };

        let device = repackage(
            zip::read(SHELL_IPA).unwrap(),
            "Payload/OmniBuildShell.app/",
            &app,
            Some(&identity()),
            &[PROFILE.to_vec()],
        )
        .unwrap_err();
        // The Simulator takes the entitlements Xcode linked into the
        // shell's executable, which OmniBuild leaves as they are.
        let simulator = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &app,
            None,
            &[],
        );

        assert_eq!(
            format!("{device:#}"),
            "the provisioning profile for dev.omnibuild.shell doesn't allow aps-environment, which it claims in ios.entitlements: turn the capability on for its App ID, and make the profile again"
        );
        assert!(simulator.is_ok());
    }

    #[test]
    fn checks_each_bundles_entitlements_against_its_own_profile() {
        let with_bundles = |bundles: &[(&str, Option<BTreeMap<String, Entitlement>>)]| App {
            id: Some("dev.example.app".to_owned()),
            ios: Ios {
                bundles: Some(
                    bundles
                        .iter()
                        .map(|(suffix, entitlements)| {
                            (
                                (*suffix).to_owned(),
                                Bundle {
                                    entitlements: entitlements.clone(),
                                },
                            )
                        })
                        .collect(),
                ),
                ..Ios::default()
            },
            ..App::default()
        };
        let for_devices = |app: &App| {
            repackage(
                zip::read(SHELL_IPA).unwrap(),
                "Payload/OmniBuildShell.app/",
                app,
                Some(&identity()),
                &[PROFILE.to_vec(), example_app_clip_profile()],
            )
        };
        let unknown = with_bundles(&[("ShellShare", None)]);

        // The App Clip's profile allows no keychain group.
        let clip = for_devices(&with_bundles(&[(
            "Clip",
            Some(keychain_group("$(AppIdentifierPrefix)dev.example.shared")),
        )]))
        .unwrap_err();
        let device = for_devices(&unknown).unwrap_err();
        let simulator = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &unknown,
            None,
            &[],
        )
        .unwrap_err();
        // A bundle that claims nothing is signed as it would be without it.
        let nothing = for_devices(&with_bundles(&[("watchkitapp.ShellWatchWidgets", None)]));

        assert_eq!(
            format!("{clip:#}"),
            "the provisioning profile for dev.example.app.Clip doesn't allow keychain-access-groups, which it claims in ios.bundles.Clip.entitlements: turn the capability on for its App ID, and make the profile again"
        );
        let names = "ios.bundles names ShellShare, which the shell has no bundle for: its app extensions, App Clips and watch apps, by the part of their ID after the app's, are [";
        assert!(format!("{device:#}").starts_with(names), "{device:#}");
        assert!(format!("{simulator:#}").starts_with(names), "{simulator:#}");
        assert!(nothing.is_ok());
    }

    #[test]
    fn claims_the_domains_of_its_website_links_among_its_others() {
        let info = InfoPlist::parse(&shell_info_plist()).unwrap();
        let claimed = |domains: Option<Entitlement>, hosts: Option<&[&str]>| {
            let app = App {
                web_links: hosts.map(|hosts| hosts.iter().map(|&host| host.to_owned()).collect()),
                ios: Ios {
                    entitlements: domains.map(|domains| {
                        [("com.apple.developer.associated-domains".to_owned(), domains)].into()
                    }),
                    ..Ios::default()
                },
                ..App::default()
            };
            claims(&info, &app)
                .into_iter()
                .map(|(id, claims)| {
                    (
                        id,
                        claims.source,
                        claims
                            .entitlements
                            .get("com.apple.developer.associated-domains")
                            .cloned(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let strings = |strings: &[&str]| {
            Some(Entitlement::Strings(
                strings.iter().map(|&string| string.to_owned()).collect(),
            ))
        };
        let tides = ["tides.example.com", "tides.example.com"];
        let app = "dev.omnibuild.shell".to_owned();

        assert_eq!(
            claimed(None, Some(&tides)),
            [(
                app.clone(),
                "webLinks".to_owned(),
                strings(&["applinks:tides.example.com"])
            )]
        );
        // After those the app claims itself, once each.
        assert_eq!(
            claimed(
                strings(&[
                    "webcredentials:tides.example.com",
                    "applinks:tides.example.com"
                ]),
                Some(&["tides.example.com", "*.example.com"])
            ),
            [(
                app.clone(),
                "ios.entitlements and webLinks".to_owned(),
                strings(&[
                    "webcredentials:tides.example.com",
                    "applinks:tides.example.com",
                    "applinks:*.example.com"
                ])
            )]
        );
        assert_eq!(
            claimed(
                Some(Entitlement::String(
                    "webcredentials:tides.example.com".to_owned()
                )),
                Some(&tides)
            ),
            [(
                app.clone(),
                "ios.entitlements and webLinks".to_owned(),
                strings(&[
                    "webcredentials:tides.example.com",
                    "applinks:tides.example.com"
                ])
            )]
        );
        // Not domains, which the profile refuses.
        assert_eq!(
            claimed(Some(Entitlement::Boolean(true)), Some(&tides)),
            [(
                app.clone(),
                "ios.entitlements and webLinks".to_owned(),
                Some(Entitlement::Boolean(true))
            )]
        );
        assert_eq!(claimed(None, None), []);
    }

    #[test]
    fn checks_its_website_links_hosts_and_domains() {
        let app = |host: &str| App {
            web_links: Some(vec![host.to_owned()]),
            ..App::default()
        };

        // The test profile allows no associated domains.
        let device = repackage(
            zip::read(SHELL_IPA).unwrap(),
            "Payload/OmniBuildShell.app/",
            &app("tides.example.com"),
            Some(&identity()),
            &[PROFILE.to_vec()],
        )
        .unwrap_err();
        let simulator = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &app("https://tides.example.com"),
            None,
            &[],
        )
        .unwrap_err();

        assert_eq!(
            format!("{device:#}"),
            "the provisioning profile for dev.omnibuild.shell doesn't allow com.apple.developer.associated-domains, which it claims in webLinks: turn the capability on for its App ID, and make the profile again"
        );
        assert!(
            format!("{simulator:#}")
                .starts_with("\"https://tides.example.com\" isn't a website's host"),
            "{simulator:#}"
        );
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

    /// A file of the device shell's Info.plists, with `change` made to it.
    fn device_plist_with(name: &str, change: impl FnOnce(&mut plist::Dictionary)) -> Entry {
        let mut info = Value::from_reader(Cursor::new(device_file(name))).unwrap();
        change(info.as_dictionary_mut().unwrap());
        let mut bytes = Vec::new();
        info.to_writer_binary(&mut bytes).unwrap();
        Entry::new(name, Method::Deflated, &bytes)
    }

    /// The device shell with one of its files replaced by `replacement`.
    fn device_with(replacement: &Entry) -> Vec<Entry> {
        zip::read(SHELL_IPA)
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

    #[test]
    fn adds_native_modules_and_signs_their_frameworks() {
        let dir = tempfile::tempdir().unwrap();
        let modules = [super::modules::tests::module(
            vec![super::modules::tests::xcframework(dir.path(), "Haptics")],
            &["Haptics"],
        )];
        let runtime = |info: &mut plist::Dictionary| {
            info.insert("OmniBuildRuntime".to_owned(), "example@1".into());
        };
        let simulator = super::repackage(
            with_file(&info_plist_with(runtime)),
            "OmniBuildShell.app/",
            &App::default(),
            &modules,
            None,
            &[],
        )
        .unwrap();
        let device = super::repackage(
            device_with(&device_plist_with(
                "Payload/OmniBuildShell.app/Info.plist",
                runtime,
            )),
            "Payload/OmniBuildShell.app/",
            &App {
                id: Some("dev.example.app".to_owned()),
                ..App::default()
            },
            &modules,
            Some(&identity()),
            &[PROFILE.to_vec(), example_app_clip_profile()],
        )
        .unwrap();

        // A shell that names no runtime takes none.
        let error = super::repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &App::default(),
            &modules,
            None,
            &[],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .ends_with("names its runtime in its Info.plist, as OmniBuildRuntime")
        );
        for (app, bundle) in [
            (simulator, "OmniBuildShell.app/"),
            (device, "Payload/OmniBuildShell.app/"),
        ] {
            let entries = zip::read(&app).unwrap();
            let contents = |name: &str| {
                file(&entries, &format!("{bundle}{name}"))
                    .contents()
                    .unwrap()
            };
            // Signed as the shell's own framework is: with a seal of its own,
            // which the app's seal covers in turn.
            let seal = contents("Frameworks/Haptics.framework/_CodeSignature/CodeResources");
            assert!(
                String::from_utf8(seal)
                    .unwrap()
                    .contains("<key>Info.plist</key>")
            );
            let app_seal = String::from_utf8(contents("_CodeSignature/CodeResources")).unwrap();
            assert!(app_seal.contains("<key>Frameworks/Haptics.framework/Haptics</key>"));
            let info = Value::from_reader(Cursor::new(contents("Info.plist"))).unwrap();
            let listed = |key: &str| {
                info.as_dictionary().unwrap()[key].as_array().unwrap()[0]
                    .as_string()
                    .unwrap()
                    .to_owned()
            };
            assert_eq!(listed("OmniBuildModuleFrameworks"), "Haptics.framework");
            assert_eq!(listed("OmniBuildModules"), "Haptics");
        }
    }

    #[test]
    fn names_the_bundles_a_build_for_devices_signs() {
        let groups = Entitlement::Strings(vec!["group.dev.example".to_owned()]);
        let app = App {
            id: Some("dev.example.app".to_owned()),
            web_links: Some(vec!["example.com".to_owned()]),
            ios: Ios {
                bundles: Some(BTreeMap::from([(
                    "ShellNotifications".to_owned(),
                    Bundle {
                        entitlements: Some(BTreeMap::from([(
                            "com.apple.security.application-groups".to_owned(),
                            groups.clone(),
                        )])),
                    },
                )])),
                ..Ios::default()
            },
            ..App::default()
        };

        let bundles = signed_bundles(
            &zip::read(SHELL_IPA).unwrap(),
            "Payload/OmniBuildShell.app/",
            &app,
        )
        .unwrap();

        let ids: Vec<_> = bundles
            .iter()
            .map(|bundle| (bundle.id.as_str(), bundle.app_clip))
            .collect();
        assert_eq!(
            ids,
            [
                ("dev.example.app", false),
                ("dev.example.app.ShellIntents", false),
                ("dev.example.app.ShellNotifications", false),
                ("dev.example.app.watchkitapp", false),
                ("dev.example.app.watchkitapp.ShellWatchWidgets", false),
                ("dev.example.app.Clip", true),
            ]
        );
        let app_claims = bundles[0].claims.as_ref().unwrap();
        assert_eq!(app_claims.source, "webLinks");
        assert_eq!(
            app_claims.entitlements["com.apple.developer.associated-domains"],
            Entitlement::Strings(vec!["applinks:example.com".to_owned()])
        );
        let extension = bundles[2].claims.as_ref().unwrap();
        assert_eq!(
            extension.source,
            "ios.bundles.ShellNotifications.entitlements"
        );
        assert_eq!(
            extension.entitlements["com.apple.security.application-groups"],
            groups
        );
        assert!(bundles[1].claims.is_none());

        // Without an app config, the shell's own IDs.
        let shell = signed_bundles(
            &zip::read(SHELL_IPA).unwrap(),
            "Payload/OmniBuildShell.app/",
            &App::default(),
        )
        .unwrap();

        assert_eq!(shell[0].id, "dev.omnibuild.shell");
        assert_eq!(shell[5].id, "dev.omnibuild.shell.Clip");
    }

    #[test]
    fn says_why_it_cant_name_the_bundles() {
        const DEVICE_INFO: &str = "Payload/OmniBuildShell.app/Info.plist";
        const DEVICE_EXTENSION_INFO: &str =
            "Payload/OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist";
        let error = |entries: &[Entry], app: &App| {
            let bundle = bundle(entries).unwrap();
            format!("{:#}", signed_bundles(entries, &bundle, app).err().unwrap())
        };
        let with_id = App {
            id: Some("dev.example.app".to_owned()),
            ..App::default()
        };

        assert_eq!(
            error(&zip::read(SHELL_APP).unwrap(), &App::default()),
            "only a build for iOS devices, an .ipa, is signed with provisioning profiles, and the shell's CFBundleSupportedPlatforms is [iPhoneSimulator]"
        );
        assert!(
            error(
                &device_with(&Entry::new(DEVICE_INFO, Method::Stored, b"not a plist")),
                &App::default()
            )
            .starts_with(&format!("couldn't read the shell's {DEVICE_INFO}"))
        );
        assert_eq!(
            error(
                &device_with(&device_plist_with(DEVICE_INFO, |info| {
                    info.remove("CFBundleIdentifier");
                })),
                &App::default()
            ),
            format!("the shell's {DEVICE_INFO} has no bundle ID (CFBundleIdentifier)")
        );
        assert!(
            error(
                &zip::read(SHELL_IPA).unwrap(),
                &App {
                    id: Some("not an id!".to_owned()),
                    ..App::default()
                }
            )
            .starts_with(&format!("couldn't rewrite the shell's {DEVICE_INFO}: "))
        );
        assert!(
            error(
                &device_with(&device_plist_with(DEVICE_EXTENSION_INFO, |info| {
                    info.insert(
                        "CFBundleIdentifier".to_owned(),
                        Value::String("dev.other.Share".to_owned()),
                    );
                })),
                &with_id
            )
            .starts_with(&format!(
                "couldn't rewrite the shell's {DEVICE_EXTENSION_INFO}: "
            ))
        );
    }
}
