//! Signing an iOS app's bundle, as codesign does: the profile goes in the
//! bundle, every file but the executable is sealed in
//! `_CodeSignature/CodeResources`, and the executable's signature covers
//! the seal, `Info.plist` and the entitlements. The code nested in the app
//! is signed the same way first, as bundles of their own, so the app's seal
//! covers what they became, as Xcode signs them: each framework in its
//! `Frameworks/`, with no entitlements, then each app extension, in its
//! `PlugIns/` or `Extensions/`, its watch app, in its `Watch/`, and each App
//! Clip, in its `AppClips/`, with a profile and entitlements of their own.
//! An extension, a watch app or an App Clip has frameworks and extensions
//! of its own in turn, such as a watch app's widgets, or the frameworks an
//! App Clip embeds, which download with it rather than with the app: those
//! are signed before it, the same way, so that signing goes from the inside
//! out, as Apple asks.

use anyhow::{Context, Result, bail};

use super::code_resources::code_resources;
use super::code_signature::Signer;
use super::entitlements::{Entitlement, Entitlements};
use super::info_plist::InfoPlist;
use super::macho;
use super::profile::Profile;
use super::{APP_CLIPS, EXTENSION_FOLDERS, WATCH};
use crate::key::SigningKey;
use crate::zip::{Entry, Method};

const FRAMEWORKS: &str = "Frameworks/";
/// The App IDs of an app's App Clips, as its signature names them.
const APP_CLIP_APPLICATIONS: &str = "com.apple.developer.associated-appclip-app-identifiers";
/// What a framework's own folders would hold of code nested in it in turn,
/// which would have to be signed before it. iOS frameworks have none, so
/// OmniBuild refuses it there, rather than sign it as it does in a
/// program.
const NESTED_CODE: [&str; 2] = [".framework/", ".appex/"];
/// A symbolic link's file type, in a zip entry's Unix mode.
const SYMBOLIC_LINK: u32 = 0o120_000;
const FILE_TYPE: u32 = 0o170_000;

/// What a build for devices is signed with: a key Apple issued a
/// certificate for, and provisioning profiles allowing it.
pub struct Identity<'a> {
    pub key: &'a SigningKey,
    /// The profiles each bundle, the app and each extension, App Clip and
    /// watch app, takes the one that fits it closest from (`Profile::fit`),
    /// as Xcode picks them: an extension or a watch app has an ID of its
    /// own, which needs a profile of its own unless a wildcard covers it,
    /// and an App Clip always does.
    pub profiles: &'a [Profile],
}

impl Identity<'_> {
    /// The profile for the bundle `id`, and the entitlements it gives it.
    fn for_bundle(&self, id: &str) -> Result<(&Profile, Entitlements)> {
        let profile = self.profile(id)?;
        let entitlements = profile
            .entitlements(id, self.key.certificate())
            .with_context(|| format!("the provisioning profile for {id} doesn't fit"))?;
        Ok((profile, entitlements))
    }

    /// The profile for the App Clip `id` of the app `app_id`, and the
    /// entitlements it gives it.
    fn for_app_clip(&self, id: &str, app_id: &str) -> Result<(&Profile, Entitlements)> {
        let profile = self.profile(id)?;
        let entitlements = profile
            .app_clip_entitlements(id, app_id, self.key.certificate())
            .with_context(|| format!("the provisioning profile for {id} doesn't fit"))?;
        Ok((profile, entitlements))
    }

    /// The profile that fits the bundle `id` closest.
    fn profile(&self, id: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .filter(|profile| profile.fit(id).is_some())
            .max_by_key(|profile| profile.fit(id))
            .with_context(|| {
                let app_ids: Vec<&str> = self.profiles.iter().map(Profile::app_id).collect();
                format!(
                    "none of the provisioning profiles includes {id}, as their App IDs say ({}): give one that does with --profile",
                    app_ids.join(", ")
                )
            })
    }
}

/// The entries with the app in the folder `bundle` signed, as `info`
/// describes it: ad hoc without an identity, which the Simulator takes, or
/// with one, as a device needs.
pub fn sign(
    entries: Vec<Entry>,
    bundle: &str,
    info: &InfoPlist,
    identity: Option<&Identity>,
) -> Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = entries
        .into_iter()
        .filter(|entry| !is_extended_attributes(&entry.name))
        .collect();
    if let Some(link) = entries
        .iter()
        .find(|entry| entry.attributes >> 16 & FILE_TYPE == SYMBOLIC_LINK)
    {
        bail!(
            "{} is a symbolic link, which OmniBuild can't sign yet",
            link.name
        );
    }
    let nested = nested_code(&entries, bundle, true)?;

    let identifier = info
        .id()
        .context("the shell's Info.plist has no bundle ID (CFBundleIdentifier)")?;
    let key = identity.map(|identity| identity.key);
    let mut profile = identity
        .map(|identity| identity.for_bundle(identifier))
        .transpose()?;
    if let Some((profile, _)) = &profile {
        put(
            &mut entries,
            &format!("{bundle}embedded.mobileprovision"),
            &profile.bytes,
        );
    }

    sign_nested(&mut entries, &nested, identity)?;
    let mut app_clips = Vec::new();
    for app_clip in &nested.app_clips {
        sign_nested(&mut entries, &app_clip.nested, identity)?;
        let folder = &app_clip.folder;
        let app_clip_info = bundle_info(&entries, folder)?;
        let id = bundle_id(&app_clip_info, folder)?;
        let profile = identity
            .map(|identity| identity.for_app_clip(id, identifier))
            .transpose()?;
        sign_program(
            &mut entries,
            folder,
            &app_clip_info,
            id,
            key,
            profile.as_ref(),
        )?;
        app_clips.push(id.to_owned());
    }
    // The app names its App Clips in turn, as they name it.
    if let Some((profile, entitlements)) = &mut profile
        && !app_clips.is_empty()
    {
        let app_ids = app_clips
            .iter()
            .map(|id| format!("{}.{id}", profile.prefix()))
            .collect();
        entitlements.insert(APP_CLIP_APPLICATIONS, Entitlement::Strings(app_ids));
    }
    sign_bundle(
        &mut entries,
        bundle,
        info,
        &Signer {
            identifier,
            info_plist: &[],
            code_resources: &[],
            entitlements: profile.as_ref().map(|(_, entitlements)| entitlements),
            key,
            main: true,
        },
    )?;
    Ok(entries)
}

/// Signs the frameworks, app extensions and watch apps `nested` in a
/// bundle, each after the code nested in it in turn, with `identity`'s key,
/// and each program with a profile for its own ID.
fn sign_nested(
    entries: &mut Vec<Entry>,
    nested: &Nested,
    identity: Option<&Identity>,
) -> Result<()> {
    let key = identity.map(|identity| identity.key);
    for framework in &nested.frameworks {
        let framework_info = bundle_info(entries, framework)?;
        sign_bundle(
            entries,
            framework,
            &framework_info,
            &Signer {
                identifier: bundle_id(&framework_info, framework)?,
                info_plist: &[],
                code_resources: &[],
                entitlements: None,
                key,
                main: false,
            },
        )?;
    }
    for program in nested.extensions.iter().chain(&nested.watch_apps) {
        sign_nested(entries, &program.nested, identity)?;
        let folder = &program.folder;
        let program_info = bundle_info(entries, folder)?;
        let id = bundle_id(&program_info, folder)?;
        let profile = identity
            .map(|identity| identity.for_bundle(id))
            .transpose()?;
        sign_program(entries, folder, &program_info, id, key, profile.as_ref())?;
    }
    Ok(())
}

/// Signs the app extension, App Clip or watch app in the folder `folder`,
/// which `info` describes and `identifier` names, with its own profile and the
/// entitlements it gives, if it has one: each is a program of its own.
fn sign_program(
    entries: &mut Vec<Entry>,
    folder: &str,
    info: &InfoPlist,
    identifier: &str,
    key: Option<&SigningKey>,
    profile: Option<&(&Profile, Entitlements)>,
) -> Result<()> {
    if let Some((profile, _)) = profile {
        put(
            entries,
            &format!("{folder}embedded.mobileprovision"),
            &profile.bytes,
        );
    }
    sign_bundle(
        entries,
        folder,
        info,
        &Signer {
            identifier,
            info_plist: &[],
            code_resources: &[],
            entitlements: profile.map(|(_, entitlements)| entitlements),
            key,
            main: true,
        },
    )
}

/// The code nested in a bundle, each bundle a folder, in the order it's
/// signed in, before the bundle.
#[derive(Default)]
struct Nested {
    frameworks: Vec<String>,
    /// In both the folders they go in.
    extensions: Vec<Program>,
    /// Only an app has these.
    watch_apps: Vec<Program>,
    app_clips: Vec<Program>,
}

/// An app extension, a watch app or an App Clip, which runs on its own: the
/// folder it's in, and the code nested in it in turn.
struct Program {
    folder: String,
    nested: Nested,
}

/// The folders of the programs nested in the app in the folder `bundle`, at
/// every depth: its app extensions, watch app and App Clips, and the
/// extensions nested in those in turn, each of which is given an ID under
/// the app's. Refuses what OmniBuild can't sign, as signing would.
pub fn programs(entries: &[Entry], bundle: &str) -> Result<Vec<String>> {
    fn add(programs: &[Program], folders: &mut Vec<String>) {
        for program in programs {
            folders.push(program.folder.clone());
            add(&program.nested.extensions, folders);
        }
    }
    let nested = nested_code(entries, bundle, true)?;
    let mut folders = Vec::new();
    for programs in [&nested.extensions, &nested.watch_apps, &nested.app_clips] {
        add(programs, &mut folders);
    }
    Ok(folders)
}

/// What ditto stores of each file's extended attributes, which unzipping
/// with anything but ditto turns into files of their own, outside the seal.
fn is_extended_attributes(name: &str) -> bool {
    let file = name.rsplit('/').find(|part| !part.is_empty());
    name.starts_with("__MACOSX/") || file.is_some_and(|file| file.starts_with("._"))
}

/// The code nested in the bundle in the folder `bundle`, and in that code in
/// turn: in an app (`app`), or in a program nested in one, which has
/// frameworks and extensions but no watch app or App Clip of its own.
/// Refuses what OmniBuild can't sign: any other code.
fn nested_code(entries: &[Entry], bundle: &str, app: bool) -> Result<Nested> {
    let mut frameworks = Vec::new();
    let mut extensions = Vec::new();
    let mut watch_apps = Vec::new();
    let mut app_clips = Vec::new();
    for entry in entries {
        // What ditto stores of extended attributes, which signing leaves
        // out, isn't code, though it's named after it (`._Share.appex`).
        let Some(file) = entry
            .name
            .strip_prefix(bundle)
            .filter(|_| !is_extended_attributes(&entry.name))
        else {
            continue;
        };
        if let Some((framework, inside)) =
            nested_bundle(&entry.name, file, FRAMEWORKS, ".framework")?
        {
            if NESTED_CODE.iter().any(|code| inside.contains(code)) {
                bail!(cant_sign(&entry.name));
            }
            frameworks.push(format!("{bundle}{FRAMEWORKS}{framework}/"));
        }
        for folder in EXTENSION_FOLDERS {
            if let Some((extension, _)) = nested_bundle(&entry.name, file, folder, ".appex")? {
                extensions.push(format!("{bundle}{folder}{extension}/"));
            }
        }
        for (folder, programs) in [(WATCH, &mut watch_apps), (APP_CLIPS, &mut app_clips)] {
            if let Some((program, _)) = nested_bundle(&entry.name, file, folder, ".app")? {
                if !app {
                    bail!(cant_sign(&entry.name));
                }
                programs.push(format!("{bundle}{folder}{program}/"));
            }
        }
    }
    let programs = |mut folders: Vec<String>| -> Result<Vec<Program>> {
        folders.sort();
        folders.dedup();
        folders
            .into_iter()
            .map(|folder| {
                let nested = nested_code(entries, &folder, false)?;
                Ok(Program { folder, nested })
            })
            .collect()
    };
    frameworks.sort();
    frameworks.dedup();
    Ok(Nested {
        frameworks,
        extensions: programs(extensions)?,
        watch_apps: programs(watch_apps)?,
        app_clips: programs(app_clips)?,
    })
}

/// Why OmniBuild refuses the code at `name`.
fn cant_sign(name: &str) -> String {
    format!(
        "the shell has code in its bundle that isn't a framework, an app extension, or the app's App Clip or watch app ({name}), which OmniBuild can't sign yet"
    )
}

/// The name of the bundle the app's file `file` is in, if it's in `folder`,
/// where every bundle is a `Name<kind>/`, such as a framework in
/// `Frameworks/`, and the file's path inside it. Anything else there is
/// refused. `name` is the file's entry.
fn nested_bundle<'a>(
    name: &str,
    file: &'a str,
    folder: &str,
    kind: &str,
) -> Result<Option<(&'a str, &'a str)>> {
    // The folder's own entry, which isn't code.
    let Some(code) = file.strip_prefix(folder).filter(|code| !code.is_empty()) else {
        return Ok(None);
    };
    let (nested, inside) = code.split_once('/').unwrap_or((code, ""));
    if !nested.ends_with(kind) {
        bail!(cant_sign(name));
    }
    Ok(Some((nested, inside)))
}

/// The Info.plist of the nested bundle in the folder `folder`.
fn bundle_info(entries: &[Entry], folder: &str) -> Result<InfoPlist> {
    let name = format!("{folder}Info.plist");
    entries
        .iter()
        .find(|entry| entry.name == name)
        .with_context(|| format!("the shell has no {name}"))?
        .contents()
        .and_then(|contents| InfoPlist::parse(&contents))
        .with_context(|| format!("couldn't read the shell's {name}"))
}

/// The nested bundle's ID, from its Info.plist, `info`.
fn bundle_id<'a>(info: &'a InfoPlist, folder: &str) -> Result<&'a str> {
    info.id().with_context(|| {
        format!("the shell's {folder}Info.plist has no bundle ID (CFBundleIdentifier)")
    })
}

/// Seals and signs the bundle in the folder `folder`, which `info`
/// describes, with `signer`'s identity: its files, and then its executable,
/// whose signature covers the seal and `Info.plist`.
fn sign_bundle(
    entries: &mut Vec<Entry>,
    folder: &str,
    info: &InfoPlist,
    signer: &Signer,
) -> Result<()> {
    let executable = info
        .executable()
        .map(|executable| format!("{folder}{executable}"))
        .with_context(|| {
            format!("the shell's {folder}Info.plist has no executable (CFBundleExecutable)")
        })?;
    let signature_folder = format!("{folder}_CodeSignature/");
    let mut files = Vec::new();
    for entry in entries.iter() {
        // Files only: not the bundle, nor any other folder.
        if let Some(file) = entry.name.strip_prefix(folder)
            && !file.is_empty()
            && !file.ends_with('/')
            && entry.name != executable
            && !entry.name.starts_with(&signature_folder)
        {
            files.push((file.to_owned(), entry.contents()?));
        }
    }
    let seal = code_resources(&files);
    put(entries, &format!("{signature_folder}CodeResources"), &seal);

    let info_plist = files
        .iter()
        .find(|(name, _)| name == "Info.plist")
        .map(|(_, contents)| contents.as_slice())
        .unwrap_or_default();
    let signer = Signer {
        info_plist,
        code_resources: &seal,
        ..*signer
    };
    let index = entries
        .iter()
        .position(|entry| entry.name == executable)
        .with_context(|| {
            format!("the shell has no {executable}, the executable its Info.plist names")
        })?;
    let code = entries[index]
        .contents()
        .and_then(|code| macho::sign(&code, |code| signer.sign(code)))
        .with_context(|| format!("couldn't sign {executable}"))?;
    entries[index] = entries[index].with_contents(&code);
    Ok(())
}

/// Replaces the contents of the entry `name`, or adds it.
fn put(entries: &mut Vec<Entry>, name: &str, contents: &[u8]) {
    match entries.iter().position(|entry| entry.name == name) {
        Some(index) => entries[index] = entries[index].with_contents(contents),
        None => entries.push(Entry::new(name, Method::Deflated, contents)),
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{Identity, programs, sign};
    use crate::bytes::Reader;
    use crate::ios::code_resources::code_resources;
    use crate::ios::code_signature::tests::{blobs, identity};
    use crate::ios::info_plist::InfoPlist;
    use crate::ios::profile::Profile;
    use crate::ios::profile::tests::{CLIP_PROFILE, PROFILE, profile_for};
    use crate::ios::tests::{SHELL_APP, SHELL_IPA};
    use crate::key::tests::KEY;
    use crate::zip::{self, Entry, Method};

    const SIMULATOR: &str = "OmniBuildShell.app/";
    const DEVICE: &str = "Payload/OmniBuildShell.app/";

    fn file(entries: &[Entry], name: &str) -> Vec<u8> {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .contents()
            .unwrap()
    }

    fn info(entries: &[Entry], bundle: &str) -> InfoPlist {
        InfoPlist::parse(&file(entries, &format!("{bundle}Info.plist"))).unwrap()
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// The code directory in a signed executable for devices, whose
    /// `LC_CODE_SIGNATURE` is at `load_command`.
    fn code_directory(executable: &[u8], load_command: usize) -> Vec<u8> {
        // Where its LC_CODE_SIGNATURE says the signature is.
        let start = Reader::at(executable, load_command + 8).u32() as usize;
        blobs(&executable[start..])
            .into_iter()
            .find(|(slot, _)| *slot == 0)
            .unwrap()
            .1
    }

    #[test]
    fn signs_a_simulator_build_ad_hoc_as_codesign_does() {
        let shell = zip::read(SHELL_APP).unwrap();

        let signed = sign(shell.clone(), SIMULATOR, &info(&shell, SIMULATOR), None).unwrap();

        // What `codesign --sign -` wrote for the same app, signing its
        // framework, its extensions, its watch app and its App Clip first,
        // and the watch app's widgets and the App Clip's framework before
        // those.
        let hash = |name: &str| sha256(&file(&signed, &format!("{SIMULATOR}{name}")));
        assert_eq!(
            hash("Frameworks/ShellKit.framework/ShellKit"),
            "162318c03013062dbe5eb0aa9338bb927af6e6ba7a7d274ba1dd9f7c299b7439"
        );
        assert_eq!(
            hash("Frameworks/ShellKit.framework/_CodeSignature/CodeResources"),
            "d7044e4217aa0def54023d0779ed9ae5925eea2fa6da7cace8cc6f9e9f7e4efa"
        );
        assert_eq!(
            hash("PlugIns/ShellNotifications.appex/ShellNotifications"),
            "a2072e6c7a899f9ffca6b7e25d2ac358b3d9c855a2d75681e14693069a82056b"
        );
        assert_eq!(
            hash("PlugIns/ShellNotifications.appex/_CodeSignature/CodeResources"),
            "7658aade53eacf49ded0e617f0291389e76888bffd7c199d50a94b46ff3f77dc"
        );
        assert_eq!(
            hash("Extensions/ShellIntents.appex/ShellIntents"),
            "4486f72b0c553be9c15e9d0d35b5ef2bc0358aa260e3447f4bb5d61e8ecd9687"
        );
        assert_eq!(
            hash("Extensions/ShellIntents.appex/_CodeSignature/CodeResources"),
            "2457e2567324ae0b2782bc3994aef4892eef6c5cc7c059a3e1181f0b56323ce0"
        );
        assert_eq!(
            hash("Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets"),
            "0d17553a913e101628a15bd802a6be9b9718a753a70464c1fd03a3708cd26b8e"
        );
        assert_eq!(
            hash(
                "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/_CodeSignature/CodeResources"
            ),
            "5a7ceda9903a27c8fb76adbb56b7e43cbd440216ed3536fa2568ec8ad2dd2bbc"
        );
        assert_eq!(
            hash("Watch/ShellWatch.app/ShellWatch"),
            "e7a06e33a43b214e82737dd8bd4f78809804dec41923417f4ae9f8744bf248e6"
        );
        assert_eq!(
            hash("Watch/ShellWatch.app/_CodeSignature/CodeResources"),
            "8792ccb9f1789636d69978179a9a6e09c266018dc026aa12833ad0d98e4a4c1b"
        );
        // The App Clip's copy of the framework, signed as the app's is.
        assert_eq!(
            hash("AppClips/ShellClip.app/Frameworks/ShellKit.framework/ShellKit"),
            "162318c03013062dbe5eb0aa9338bb927af6e6ba7a7d274ba1dd9f7c299b7439"
        );
        assert_eq!(
            hash(
                "AppClips/ShellClip.app/Frameworks/ShellKit.framework/_CodeSignature/CodeResources"
            ),
            "d7044e4217aa0def54023d0779ed9ae5925eea2fa6da7cace8cc6f9e9f7e4efa"
        );
        assert_eq!(
            hash("AppClips/ShellClip.app/ShellClip"),
            "b46b901c9cc1d530644753b6cfa623ffe07a1b8836004cbeadbf737dfe4f155b"
        );
        assert_eq!(
            hash("AppClips/ShellClip.app/_CodeSignature/CodeResources"),
            "417f072dab2e6d83298fd169c6bd09f2cb785f7fc2b6d05ed6c311a30af96071"
        );
        assert_eq!(
            hash("OmniBuildShell"),
            "32198e66a5739461e5f7c401a4dca7a14f6b997dfbb40f64983108f3e8158905"
        );
        assert_eq!(
            hash("_CodeSignature/CodeResources"),
            "d9a6e1052162d718c577a6dce72ed355968a714de5ce189a61a3a28a0e38109a"
        );
        let names = |entries: &[Entry]| -> Vec<String> {
            entries.iter().map(|entry| entry.name.clone()).collect()
        };
        assert_eq!(names(&signed), names(&shell));
    }

    #[test]
    fn signs_a_device_build_ad_hoc_as_codesign_does() {
        let shell = zip::read(SHELL_IPA).unwrap();

        let ad_hoc = sign(shell.clone(), DEVICE, &info(&shell, DEVICE), None).unwrap();

        let app = |name: &str| file(&ad_hoc, &format!("{DEVICE}{name}"));
        // The framework's, the extensions', the watch app's and the App
        // Clip's, and the watch app's widgets' and the App Clip's
        // framework's, as codesign signs them, up to the end of their
        // signatures. codesign pads each with what the linker's signature
        // left there, OmniBuild with zeros: nothing covers the padding.
        let framework = app("Frameworks/ShellKit.framework/ShellKit");
        assert_eq!(
            sha256(&framework[..33_245]),
            "c84a33211a8427b8d3c9da360ac75cb1a25dc946802dad469b775a8b3d242d90"
        );
        assert_eq!(framework[33_245..], [0; 19]);
        let extension = "PlugIns/ShellNotifications.appex/ShellNotifications";
        let extension_code = app(extension);
        assert_eq!(
            sha256(&extension_code[..33_319]),
            "60676ed08a0c2d4ecee6b1e15feff304897d5127cf7859fe9f672193a1ccc619"
        );
        assert_eq!(extension_code[33_319..], [0; 9]);
        let intents = app("Extensions/ShellIntents.appex/ShellIntents");
        assert_eq!(
            sha256(&intents[..33_313]),
            "7dbc8e4902edcff1ccca086fbc055ad663d645fb472de8f35e42c56b32c1b441"
        );
        assert_eq!(intents[33_313..], [0; 15]);
        let app_clip = app("AppClips/ShellClip.app/ShellClip");
        assert_eq!(
            sha256(&app_clip[..33_305]),
            "6ac6467974f457850e50fc95e9b7a445826e1a4a6dba3e00a59b2adb6a6fbbe1"
        );
        assert_eq!(app_clip[33_305..], [0; 23]);
        let app_clip_framework =
            app("AppClips/ShellClip.app/Frameworks/ShellKit.framework/ShellKit");
        assert_eq!(app_clip_framework, framework);
        // The watch app's and its widgets' have two architectures, arm64
        // and arm64_32: all of each, with the padding after each signature
        // zeroed.
        assert_eq!(
            sha256(&app("Watch/ShellWatch.app/ShellWatch")),
            "5ed9ddaa1fb2466e40129a667189cf26b4a0a0a069a11ba544205e076efff05e"
        );
        assert_eq!(
            sha256(&app(
                "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets"
            )),
            "48c9f25dfc4c36b61268138b30d01931353d64626dade8e11e2e189262b236ae"
        );
        // Their seals, and the app's executable, up to the end of its
        // signature, and seal, as codesign signs each bundle around the code
        // in it, with that code's padding zeroed.
        let seal = "Frameworks/ShellKit.framework/_CodeSignature/CodeResources";
        assert_eq!(
            sha256(&app(seal)),
            "b1bbabcd35a08d9d16e15144633ab1a9ff54ffdefc49a9885235bf4c847615e7"
        );
        let extension_seal = "PlugIns/ShellNotifications.appex/_CodeSignature/CodeResources";
        assert_eq!(
            sha256(&app(extension_seal)),
            "fe8436aba8da57b0e838863d3977e0f20252e335c54980436b47fa9ab0eabaf3"
        );
        assert_eq!(
            sha256(&app(
                "Extensions/ShellIntents.appex/_CodeSignature/CodeResources"
            )),
            "b81a25462ebf381000f24bbf2fc2618c316c88a85feebbf40f3d77d39ce8b37d"
        );
        assert_eq!(
            sha256(&app(
                "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/_CodeSignature/CodeResources"
            )),
            "3543351c50065d5659c249f4293999db8600943124699279d24954d32cea9af3"
        );
        assert_eq!(
            sha256(&app("Watch/ShellWatch.app/_CodeSignature/CodeResources")),
            "886699158684fa42124f96bb19a481c3e9b211416755c1720e5c5c4f9dc163c5"
        );
        assert_eq!(
            app(
                "AppClips/ShellClip.app/Frameworks/ShellKit.framework/_CodeSignature/CodeResources"
            ),
            app(seal)
        );
        assert_eq!(
            sha256(&app("AppClips/ShellClip.app/_CodeSignature/CodeResources")),
            "def99de64284e838427592212fab894f0edc4e4e571af91c94b3ce2648a24851"
        );
        let executable = app("OmniBuildShell");
        assert_eq!(
            sha256(&executable[..33_300]),
            "691ebd47df60573b497f111034f3907afee81be0b11502a0564d4c6223ecf8b8"
        );
        assert_eq!(executable[33_300..], [0; 12]);
        assert_eq!(
            sha256(&app("_CodeSignature/CodeResources")),
            "e599946c837d53e0afdc583ae4b4d2e23850103de4e2ef9eda1812c01af430c0"
        );
    }

    /// The device shell signed with the test identity: a wildcard profile
    /// for the app, its extensions, its watch app and the watch app's
    /// widgets, and the App Clip's own. codesign signing the same files with
    /// the test identity, and the extensions, the widgets, the watch app,
    /// the App Clip and the app with the entitlements their profiles give
    /// each, the App Clip and the app naming each other, wrote what the
    /// tests below compare.
    fn signed_with_identity() -> Vec<Entry> {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        let profiles = [
            Profile::parse(PROFILE).unwrap(),
            Profile::parse(CLIP_PROFILE).unwrap(),
        ];
        let identity = Identity {
            key: &key,
            profiles: &profiles,
        };
        sign(
            shell.clone(),
            DEVICE,
            &info(&shell, DEVICE),
            Some(&identity),
        )
        .unwrap()
    }

    /// One of the device app's files, as signed.
    fn device_app(signed: &[Entry], name: &str) -> Vec<u8> {
        file(signed, &format!("{DEVICE}{name}"))
    }

    /// The seal of the signed bundle in the device app's folder `folder`,
    /// whose executable is `executable`, as codesign would write it for
    /// what OmniBuild signed in it.
    fn seal_of(signed: &[Entry], folder: &str, executable: &str) -> Vec<u8> {
        let files: Vec<(String, Vec<u8>)> = signed
            .iter()
            .filter_map(|entry| {
                let name = entry.name.strip_prefix(&format!("{DEVICE}{folder}"))?;
                let sealed = !name.is_empty()
                    && !name.ends_with('/')
                    && name != executable
                    && !name.starts_with("_CodeSignature/");
                sealed.then(|| (name.to_owned(), entry.contents().unwrap()))
            })
            .collect();
        code_resources(&files)
    }

    #[test]
    fn seals_a_device_build_with_an_identity_as_codesign_does() {
        let signed = signed_with_identity();

        let app = device_app;
        let seal = "Frameworks/ShellKit.framework/_CodeSignature/CodeResources";
        let extension_seal = "PlugIns/ShellNotifications.appex/_CodeSignature/CodeResources";
        // Each bundle's profile, and the seals codesign wrote, but for those
        // of the bundles with code nested in them, the app's, the watch
        // app's and the App Clip's: a CMS signature has the time it was
        // made, so each nested executable differs each time, and with it
        // the seal around it.
        assert_eq!(app(&signed, "embedded.mobileprovision"), PROFILE);
        for extension in [
            "PlugIns/ShellNotifications.appex/",
            "Extensions/ShellIntents.appex/",
            "Watch/ShellWatch.app/",
            "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/",
        ] {
            assert_eq!(
                app(&signed, &format!("{extension}embedded.mobileprovision")),
                PROFILE
            );
        }
        assert_eq!(
            app(&signed, "AppClips/ShellClip.app/embedded.mobileprovision"),
            CLIP_PROFILE
        );
        assert_eq!(
            sha256(&app(&signed, seal)),
            "b1bbabcd35a08d9d16e15144633ab1a9ff54ffdefc49a9885235bf4c847615e7"
        );
        assert_eq!(
            sha256(&app(&signed, extension_seal)),
            "b3e87bc4f5618828de99e2b6c47979b9c4810a58426b4d2373cb2cb6b81484ce"
        );
        assert_eq!(
            sha256(&app(
                &signed,
                "Extensions/ShellIntents.appex/_CodeSignature/CodeResources"
            )),
            "f0ef23e2b0ac2897e4105e1d6d29ee4c092a4ec9da21f3d3dfff5b82610e0e5c"
        );
        assert_eq!(
            sha256(&app(
                &signed,
                "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/_CodeSignature/CodeResources"
            )),
            "2066ee48d5c14e46cef561e57560a3b6c55a78c4215cf1bf4b76e85909b62175"
        );
        assert_eq!(
            app(
                &signed,
                "AppClips/ShellClip.app/Frameworks/ShellKit.framework/_CodeSignature/CodeResources"
            ),
            app(&signed, seal)
        );
        for (folder, executable) in [
            ("", "OmniBuildShell"),
            ("Watch/ShellWatch.app/", "ShellWatch"),
            ("AppClips/ShellClip.app/", "ShellClip"),
        ] {
            assert_eq!(
                app(&signed, &format!("{folder}_CodeSignature/CodeResources")),
                seal_of(&signed, folder, executable),
                "{folder}"
            );
        }
    }

    #[test]
    fn signs_a_device_build_with_an_identity_as_codesign_does() {
        let signed = signed_with_identity();

        let app = device_app;
        let extension = "PlugIns/ShellNotifications.appex/ShellNotifications";
        let intents = "Extensions/ShellIntents.appex/ShellIntents";
        let app_clip = "AppClips/ShellClip.app/ShellClip";
        // The code directories codesign wrote, but for the hash of the
        // first page. That page has the load commands, which say how much
        // room the signature has: codesign leaves 16 KB more than it needs,
        // OmniBuild none. And the seals of the bundles with code nested in
        // them, the app's, the watch app's and the App Clip's, which differ
        // each time. Each with its LC_CODE_SIGNATURE where it is, and its
        // first page's hash after the 88-byte header, the identifier, and
        // the special slots: 7 of each program's, the third from last its
        // seal's, and 3 of the framework's. The extensions' and the
        // widgets' seals are the same each time.
        let without = |executable: &[u8], load_command: usize, hashes: &[usize]| {
            let mut directory = code_directory(executable, load_command);
            for &hash in hashes {
                directory[hash..hash + 32].fill(0);
            }
            sha256(&directory)
        };
        assert_eq!(
            without(&app(&signed, "OmniBuildShell"), 784, &[332, 332 - 3 * 32]),
            "818d90d6792b47bded8d134a39d79036ae649e9bb27682e39cc12837cade2bea"
        );
        assert_eq!(
            without(&app(&signed, extension), 784, &[351]),
            "9e67240f78e7b810c6be8d9ab374e42cb0a6daba21f26e15fa293e34540263ea"
        );
        assert_eq!(
            without(&app(&signed, intents), 784, &[345]),
            "4b6ee1211feb0b840209977b8efe24765274b08e4c0af49fab5fa7f156f694c9"
        );
        assert_eq!(
            without(&app(&signed, app_clip), 784, &[337, 337 - 3 * 32]),
            "5db5f7ce8c37a83fadaab49f0310bc9a260135a4832314de3529897f03c15ba8"
        );
        // The watch app's and its widgets' for each of their architectures,
        // the arm64_32 code with its 32-bit load commands.
        let watch = app(&signed, "Watch/ShellWatch.app/ShellWatch");
        assert_eq!(
            without(&watch[16_384..], 784, &[344, 344 - 3 * 32]),
            "d05ccb9330c586647129819354722cef7cc750cdb80b115de9189d11deef850f"
        );
        assert_eq!(
            without(&watch[65_536..], 696, &[344, 344 - 3 * 32]),
            "aa380df0e78019609ace2bb72091429d4efbb1f9bf801cae9725a8cc30f8dacd"
        );
        let widgets = app(
            &signed,
            "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets",
        );
        assert_eq!(
            without(&widgets[16_384..], 784, &[362]),
            "d8e0c6394ac0a46bf1e94729bc948e835d62a96d42d74acd12596727502f73e8"
        );
        assert_eq!(
            without(&widgets[65_536..], 696, &[362]),
            "91c976d370f7955df2cce8e98ac4dd47ac8613f14509443e6f07ca9fdcf392d1"
        );
        // The framework's, and the App Clip's copy of it, signed the same.
        for framework in [
            "Frameworks/ShellKit.framework/ShellKit",
            "AppClips/ShellClip.app/Frameworks/ShellKit.framework/ShellKit",
        ] {
            assert_eq!(
                without(&app(&signed, framework), 720, &[213]),
                "9fbd90b02c41c4df12db009fb09de9540ab5d75e0b3e283da7b4c1c69193a573"
            );
        }
    }

    #[test]
    fn refuses_an_extension_no_profile_is_for() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        let app_only = [profile_for("ABCDE12345.dev.omnibuild.shell")];

        let error = sign(
            shell.clone(),
            DEVICE,
            &info(&shell, DEVICE),
            Some(&Identity {
                key: &key,
                profiles: &app_only,
            }),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "none of the provisioning profiles includes dev.omnibuild.shell.ShellIntents, as their App IDs say (ABCDE12345.dev.omnibuild.shell): give one that does with --profile"
        );
    }

    #[test]
    fn refuses_an_app_clip_without_an_app_clips_profile() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        // A wildcard includes the App Clip's ID, but isn't an App Clip's.
        let wildcard = [Profile::parse(PROFILE).unwrap()];
        // And these don't include it at all.
        let others = [
            profile_for("ABCDE12345.dev.omnibuild.shell"),
            profile_for("ABCDE12345.dev.omnibuild.shell.Shell*"),
            profile_for("ABCDE12345.dev.omnibuild.shell.watchkitapp"),
            profile_for("ABCDE12345.dev.omnibuild.shell.watchkitapp.*"),
        ];
        let cases = [
            (
                &wildcard[..],
                "the provisioning profile for dev.omnibuild.shell.Clip doesn't fit: it isn't an App Clip's: it doesn't allow com.apple.developer.on-demand-install-capable, as the profile of an App ID registered as an App Clip's does",
            ),
            (
                &others[..],
                "none of the provisioning profiles includes dev.omnibuild.shell.Clip, as their App IDs say (ABCDE12345.dev.omnibuild.shell, ABCDE12345.dev.omnibuild.shell.Shell*, ABCDE12345.dev.omnibuild.shell.watchkitapp, ABCDE12345.dev.omnibuild.shell.watchkitapp.*): give one that does with --profile",
            ),
        ];

        for (profiles, message) in cases {
            let error = sign(
                shell.clone(),
                DEVICE,
                &info(&shell, DEVICE),
                Some(&Identity {
                    key: &key,
                    profiles,
                }),
            )
            .unwrap_err();

            assert_eq!(format!("{error:#}"), message);
        }
    }

    #[test]
    fn gives_each_bundle_the_profile_that_fits_it_closest() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        let wildcard = Profile::parse(PROFILE).unwrap();
        let app_only = profile_for("ABCDE12345.dev.omnibuild.shell");
        let app_clip = Profile::parse(CLIP_PROFILE).unwrap();
        let profiles = [wildcard, app_only, app_clip];

        let signed = sign(
            shell.clone(),
            DEVICE,
            &info(&shell, DEVICE),
            Some(&Identity {
                key: &key,
                profiles: &profiles,
            }),
        )
        .unwrap();

        // The app's own App ID over the wildcard, which the extensions have.
        let embedded = |folder: &str| {
            file(
                &signed,
                &format!("{DEVICE}{folder}embedded.mobileprovision"),
            )
        };
        assert_eq!(embedded(""), profiles[1].bytes);
        assert_eq!(embedded("PlugIns/ShellNotifications.appex/"), PROFILE);
        assert_eq!(embedded("Extensions/ShellIntents.appex/"), PROFILE);
        assert_eq!(embedded("Watch/ShellWatch.app/"), PROFILE);
        assert_eq!(
            embedded("Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/"),
            PROFILE
        );
        assert_eq!(embedded("AppClips/ShellClip.app/"), CLIP_PROFILE);
    }

    #[test]
    fn leaves_out_what_ditto_stores_of_extended_attributes() {
        let mut shell = zip::read(SHELL_APP).unwrap();
        for name in [
            "OmniBuildShell.app/._Assets.car",
            "OmniBuildShell.app/.__CodeSignature",
            "__MACOSX/OmniBuildShell.app/._PkgInfo",
            // Not an extension, though it's named like one, in PlugIns/.
            "OmniBuildShell.app/PlugIns/._ShellNotifications.appex",
        ] {
            shell.push(Entry::new(name, Method::Stored, b"attributes"));
        }

        let signed = sign(shell.clone(), SIMULATOR, &info(&shell, SIMULATOR), None).unwrap();
        let nested = programs(&shell, SIMULATOR).unwrap();

        // Sealed as before.
        assert_eq!(
            sha256(&file(
                &signed,
                "OmniBuildShell.app/_CodeSignature/CodeResources"
            )),
            "d9a6e1052162d718c577a6dce72ed355968a714de5ce189a61a3a28a0e38109a"
        );
        assert_eq!(signed.len(), shell.len() - 4);
        assert_eq!(nested, programs(&signed, SIMULATOR).unwrap());
    }

    #[test]
    fn lists_the_programs_nested_at_every_depth() {
        let shell = zip::read(SHELL_IPA).unwrap();

        let nested = programs(&shell, DEVICE).unwrap();

        // Extensions, the watch app and its widgets, and the App Clip, but
        // no framework, which has no ID of its own to give.
        assert_eq!(
            nested,
            [
                "Extensions/ShellIntents.appex/",
                "PlugIns/ShellNotifications.appex/",
                "Watch/ShellWatch.app/",
                "Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/",
                "AppClips/ShellClip.app/",
            ]
            .map(|folder| format!("{DEVICE}{folder}"))
        );
    }

    /// The Simulator shell with `entry` in place of its file of the same
    /// name, or added.
    fn shell_with(entry: Entry) -> Vec<Entry> {
        let name = entry.name.clone();
        zip::read(SHELL_APP)
            .unwrap()
            .into_iter()
            .filter(|existing| existing.name != name)
            .chain([entry])
            .collect()
    }

    #[test]
    fn reports_code_it_cant_sign() {
        let shell = zip::read(SHELL_APP).unwrap();
        let shell_info = info(&shell, SIMULATOR);
        let cases = [
            "OmniBuildShell.app/Watch/Watch.appex/Watch",
            "OmniBuildShell.app/AppClips/Clip.appex/Clip",
            "OmniBuildShell.app/PlugIns/Share.bundle/Share",
            "OmniBuildShell.app/Extensions/Intents.bundle/Intents",
            "OmniBuildShell.app/Frameworks/libswiftCore.dylib",
            "OmniBuildShell.app/Frameworks/A.framework/Frameworks/B.framework/B",
            // The same in the code nested in the app, and in that in turn.
            "OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/Widgets.bundle/Widgets",
            "OmniBuildShell.app/AppClips/ShellClip.app/Frameworks/libswiftCore.dylib",
            "OmniBuildShell.app/AppClips/ShellClip.app/Frameworks/ShellKit.framework/PlugIns/A.appex/A",
            "OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/Frameworks/libA.dylib",
            // Only the app has a watch app or App Clips.
            "OmniBuildShell.app/Watch/ShellWatch.app/Watch/Other.app/Other",
            "OmniBuildShell.app/PlugIns/ShellNotifications.appex/AppClips/Clip.app/Clip",
        ];

        for name in cases {
            let entries = shell_with(Entry::new(name, Method::Stored, b""));

            let error = sign(entries, SIMULATOR, &shell_info, None).unwrap_err();

            assert_eq!(
                error.to_string(),
                format!(
                    "the shell has code in its bundle that isn't a framework, an app extension, or the app's App Clip or watch app ({name}), which OmniBuild can't sign yet"
                )
            );
        }
    }

    #[test]
    fn reports_a_nested_bundle_it_cant_read() {
        let shell = zip::read(SHELL_APP).unwrap();
        let shell_info = info(&shell, SIMULATOR);
        let with_file = |name: &str, contents: &[u8]| {
            shell_with(Entry::new(
                &format!("OmniBuildShell.app/{name}"),
                Method::Stored,
                contents,
            ))
        };
        let cases = [
            (
                with_file(
                    "AppClips/ShellClip.app/Info.plist",
                    b"<plist><dict/></plist>",
                ),
                "the shell's OmniBuildShell.app/AppClips/ShellClip.app/Info.plist has no bundle ID (CFBundleIdentifier)",
            ),
            (
                with_file(
                    "PlugIns/ShellNotifications.appex/Info.plist",
                    b"<plist><dict/></plist>",
                ),
                "the shell's OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist has no bundle ID (CFBundleIdentifier)",
            ),
            (
                with_file("PlugIns/Share.appex/Share", b""),
                "the shell has no OmniBuildShell.app/PlugIns/Share.appex/Info.plist",
            ),
            (
                with_file(
                    "PlugIns/ShellNotifications.appex/ShellNotifications",
                    b"not an executable",
                ),
                "couldn't sign OmniBuildShell.app/PlugIns/ShellNotifications.appex/ShellNotifications",
            ),
            (
                with_file("Watch/Other.app/Other", b""),
                "the shell has no OmniBuildShell.app/Watch/Other.app/Info.plist",
            ),
            (
                with_file("Watch/ShellWatch.app/PlugIns/Other.appex/Other", b""),
                "the shell has no OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/Other.appex/Info.plist",
            ),
            (
                with_file(
                    "AppClips/ShellClip.app/Frameworks/ShellKit.framework/Info.plist",
                    b"not a plist",
                ),
                "couldn't read the shell's OmniBuildShell.app/AppClips/ShellClip.app/Frameworks/ShellKit.framework/Info.plist",
            ),
            (
                with_file("AppClips/Other.app/Other", b""),
                "the shell has no OmniBuildShell.app/AppClips/Other.app/Info.plist",
            ),
            (
                with_file("AppClips/ShellClip.app/ShellClip", b"not an executable"),
                "couldn't sign OmniBuildShell.app/AppClips/ShellClip.app/ShellClip",
            ),
            (
                with_file("Frameworks/Lynx.framework/Lynx", b""),
                "the shell has no OmniBuildShell.app/Frameworks/Lynx.framework/Info.plist",
            ),
            (
                with_file("Frameworks/ShellKit.framework/Info.plist", b"not a plist"),
                "couldn't read the shell's OmniBuildShell.app/Frameworks/ShellKit.framework/Info.plist",
            ),
            (
                zip::read(SHELL_APP)
                    .unwrap()
                    .into_iter()
                    .filter(|entry| !entry.name.ends_with("ShellKit.framework/ShellKit"))
                    .collect(),
                "the shell has no OmniBuildShell.app/Frameworks/ShellKit.framework/ShellKit, the executable its Info.plist names",
            ),
            (
                with_file(
                    "Frameworks/ShellKit.framework/Info.plist",
                    b"<plist><dict/></plist>",
                ),
                "the shell's OmniBuildShell.app/Frameworks/ShellKit.framework/Info.plist has no bundle ID (CFBundleIdentifier)",
            ),
        ];

        for (entries, message) in cases {
            let error = sign(entries, SIMULATOR, &shell_info, None).unwrap_err();

            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn reports_an_app_it_cant_sign() {
        let shell = zip::read(SHELL_APP).unwrap();
        let profiles = [Profile::parse(PROFILE).unwrap()];
        // A key whose certificate the profile doesn't allow.
        let stranger = Identity {
            key: &KEY,
            profiles: &profiles,
        };
        let key = identity();
        let others = [profile_for("TEAM.dev.other.*"), profile_for("TEAM.dev.app")];
        let elsewhere = Identity {
            key: &key,
            profiles: &others,
        };
        let with = |entry: Entry| -> Vec<Entry> {
            let name = entry.name.clone();
            shell
                .iter()
                .filter(|existing| existing.name != name)
                .cloned()
                .chain([entry])
                .collect()
        };
        let mut link = Entry::new("OmniBuildShell.app/link", Method::Stored, b"payload.txt");
        link.attributes = 0o120_755 << 16;
        let not_macho = Entry::new(
            "OmniBuildShell.app/OmniBuildShell",
            Method::Stored,
            b"not an executable",
        );
        let shell_info = info(&shell, SIMULATOR);
        let no_keys = InfoPlist::parse(b"<plist><dict/></plist>").unwrap();
        let other_executable = InfoPlist::parse(
            b"<plist><dict>
                <key>CFBundleIdentifier</key><string>dev.my_app</string>
                <key>CFBundleExecutable</key><string>Missing</string>
            </dict></plist>",
        )
        .unwrap();
        let cases = [
            (
                with(link),
                &shell_info,
                None,
                "OmniBuildShell.app/link is a symbolic link, which OmniBuild can't sign yet",
            ),
            (
                shell.clone(),
                &no_keys,
                None,
                "the shell's Info.plist has no bundle ID (CFBundleIdentifier)",
            ),
            (
                shell.clone(),
                &other_executable,
                Some(&stranger),
                "the provisioning profile for dev.my_app doesn't fit",
            ),
            (
                shell.clone(),
                &other_executable,
                Some(&elsewhere),
                "none of the provisioning profiles includes dev.my_app, as their App IDs say (TEAM.dev.other.*, TEAM.dev.app): give one that does with --profile",
            ),
            (
                shell.clone(),
                &other_executable,
                None,
                "the shell has no OmniBuildShell.app/Missing, the executable its Info.plist names",
            ),
            (
                with(not_macho),
                &shell_info,
                None,
                "couldn't sign OmniBuildShell.app/OmniBuildShell",
            ),
        ];

        for (entries, info, identity, message) in cases {
            let error = sign(entries, SIMULATOR, info, identity).unwrap_err();

            assert_eq!(error.to_string(), message);
        }
        let no_executable = InfoPlist::parse(
            b"<plist><dict><key>CFBundleIdentifier</key><string>dev.app</string></dict></plist>",
        )
        .unwrap();
        assert_eq!(
            sign(shell.clone(), SIMULATOR, &no_executable, None)
                .unwrap_err()
                .to_string(),
            "the shell's OmniBuildShell.app/Info.plist has no executable (CFBundleExecutable)"
        );
        let mut corrupt = Entry::new("OmniBuildShell.app/payload.txt", Method::Deflated, b"");
        corrupt.data = vec![0xff; 8];
        assert!(
            sign(with(corrupt), SIMULATOR, &shell_info, None)
                .unwrap_err()
                .to_string()
                .ends_with("is corrupt")
        );
    }
}
