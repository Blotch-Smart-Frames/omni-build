//! Signing an iOS app's bundle, as codesign does: the profile goes in the
//! bundle, every file but the executable is sealed in
//! `_CodeSignature/CodeResources`, and the executable's signature covers
//! the seal, `Info.plist` and the entitlements. The code nested in the app
//! is signed the same way first, as bundles of their own, so the app's seal
//! covers what they became, as Xcode signs them: each framework in its
//! `Frameworks/`, with no entitlements, and then each app extension, in its
//! `PlugIns/` or `Extensions/`, with a profile and entitlements of its own.

use anyhow::{Context, Result, bail};

use super::EXTENSION_FOLDERS;
use super::code_resources::code_resources;
use super::code_signature::Signer;
use super::entitlements::Entitlements;
use super::info_plist::InfoPlist;
use super::macho;
use super::profile::Profile;
use crate::key::SigningKey;
use crate::zip::{Entry, Method};

const FRAMEWORKS: &str = "Frameworks/";
/// Where else in a bundle iOS looks for code of its own: a watch app and
/// App Clips, which OmniBuild doesn't sign yet.
const OTHER_CODE: [&str; 2] = ["Watch/", "AppClips/"];
/// What a nested bundle's own folders would hold of code nested in it in
/// turn, which would have to be signed before it.
const NESTED_CODE: [&str; 2] = [".framework/", ".appex/"];
/// A symbolic link's file type, in a zip entry's Unix mode.
const SYMBOLIC_LINK: u32 = 0o120_000;
const FILE_TYPE: u32 = 0o170_000;

/// What a build for devices is signed with: a key Apple issued a
/// certificate for, and provisioning profiles allowing it.
pub struct Identity<'a> {
    pub key: &'a SigningKey,
    /// The profiles each bundle, the app and each extension, takes the one
    /// that fits it closest from (`Profile::fit`), as Xcode picks them: an
    /// extension has an ID of its own, which needs a profile of its own
    /// unless a wildcard covers it.
    pub profiles: &'a [Profile],
}

impl Identity<'_> {
    /// The profile for the bundle `id`, and the entitlements it gives it.
    fn for_bundle(&self, id: &str) -> Result<(&Profile, Entitlements)> {
        let profile = self
            .profiles
            .iter()
            .filter(|profile| profile.fit(id).is_some())
            .max_by_key(|profile| profile.fit(id))
            .with_context(|| {
                let app_ids: Vec<&str> = self.profiles.iter().map(Profile::app_id).collect();
                format!(
                    "none of the provisioning profiles includes {id}, as their App IDs say ({}): give one that does with --profile",
                    app_ids.join(", ")
                )
            })?;
        let entitlements = profile
            .entitlements(id, self.key.certificate())
            .with_context(|| format!("the provisioning profile for {id} doesn't fit"))?;
        Ok((profile, entitlements))
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
    // What ditto stores of each file's extended attributes, which
    // unzipping with anything but ditto turns into files of their own,
    // outside the seal.
    let mut entries: Vec<Entry> = entries
        .into_iter()
        .filter(|entry| {
            let file = entry.name.rsplit('/').find(|part| !part.is_empty());
            !entry.name.starts_with("__MACOSX/") && !file.is_some_and(|file| file.starts_with("._"))
        })
        .collect();
    let (frameworks, extensions) = nested_code(&entries, bundle)?;

    let identifier = info
        .id()
        .context("the shell's Info.plist has no bundle ID (CFBundleIdentifier)")?;
    let key = identity.map(|identity| identity.key);
    let profile = identity
        .map(|identity| identity.for_bundle(identifier))
        .transpose()?;
    if let Some((profile, _)) = &profile {
        put(
            &mut entries,
            &format!("{bundle}embedded.mobileprovision"),
            &profile.bytes,
        );
    }

    for framework in &frameworks {
        let framework_info = bundle_info(&entries, framework)?;
        sign_bundle(
            &mut entries,
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
    for extension in &extensions {
        let extension_info = bundle_info(&entries, extension)?;
        let identifier = bundle_id(&extension_info, extension)?;
        let profile = identity
            .map(|identity| identity.for_bundle(identifier))
            .transpose()?;
        if let Some((profile, _)) = &profile {
            put(
                &mut entries,
                &format!("{extension}embedded.mobileprovision"),
                &profile.bytes,
            );
        }
        sign_bundle(
            &mut entries,
            extension,
            &extension_info,
            &Signer {
                identifier,
                info_plist: &[],
                code_resources: &[],
                entitlements: profile.as_ref().map(|(_, entitlements)| entitlements),
                key,
                // An extension's executable is a program of its own.
                main: true,
            },
        )?;
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

/// The code nested in the app in the folder `bundle`, which is signed
/// before it: its frameworks, and then its app extensions, in both the
/// folders they go in, each a folder.
/// Refuses what OmniBuild can't sign, or seal: any other code, and symbolic
/// links.
fn nested_code(entries: &[Entry], bundle: &str) -> Result<(Vec<String>, Vec<String>)> {
    let mut frameworks = Vec::new();
    let mut extensions = Vec::new();
    for entry in entries {
        let file = entry.name.strip_prefix(bundle).unwrap_or_default();
        if OTHER_CODE.iter().any(|folder| file.starts_with(folder)) {
            bail!(cant_sign(&entry.name));
        }
        if let Some(framework) = nested_bundle(&entry.name, file, FRAMEWORKS, ".framework")? {
            frameworks.push(format!("{bundle}{FRAMEWORKS}{framework}/"));
        }
        for folder in EXTENSION_FOLDERS {
            if let Some(extension) = nested_bundle(&entry.name, file, folder, ".appex")? {
                extensions.push(format!("{bundle}{folder}{extension}/"));
            }
        }
        if entry.attributes >> 16 & FILE_TYPE == SYMBOLIC_LINK {
            bail!(
                "{} is a symbolic link, which OmniBuild can't sign yet",
                entry.name
            );
        }
    }
    frameworks.sort();
    frameworks.dedup();
    extensions.sort();
    extensions.dedup();
    Ok((frameworks, extensions))
}

/// Why OmniBuild refuses the code at `name`.
fn cant_sign(name: &str) -> String {
    format!(
        "the shell has code in its bundle that isn't a framework or an app extension ({name}), which OmniBuild can't sign yet"
    )
}

/// The name of the bundle the app's file `file` is in, if it's in `folder`,
/// where every bundle is a `Name<kind>/`, such as a framework in
/// `Frameworks/`. Anything else there, or code nested in the bundle, which
/// would have to be signed on its own first, is refused. `name` is the
/// file's entry.
fn nested_bundle<'a>(
    name: &str,
    file: &'a str,
    folder: &str,
    kind: &str,
) -> Result<Option<&'a str>> {
    // The folder's own entry, which isn't code.
    let Some(code) = file.strip_prefix(folder).filter(|code| !code.is_empty()) else {
        return Ok(None);
    };
    let (nested, inside) = code.split_once('/').unwrap_or((code, ""));
    if !nested.ends_with(kind) || NESTED_CODE.iter().any(|code| inside.contains(code)) {
        bail!(cant_sign(name));
    }
    Ok(Some(nested))
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

    use super::{Identity, sign};
    use crate::bytes::Reader;
    use crate::ios::code_resources::code_resources;
    use crate::ios::code_signature::tests::{blobs, identity};
    use crate::ios::info_plist::InfoPlist;
    use crate::ios::profile::Profile;
    use crate::ios::profile::tests::{PROFILE, profile_for};
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
        // framework and its extensions first.
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
            hash("OmniBuildShell"),
            "8dc4e7e1077ad5e384cade7deeaedcace9a4d8c45135c33d1cc4e78d7569ae02"
        );
        assert_eq!(
            hash("_CodeSignature/CodeResources"),
            "fb4f5937ed25968b32fe67085980a0c664c90d47fc34042bd1ab002801e6ecba"
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
        // The framework's and the extensions', as codesign signs them, up
        // to the end of their signatures. codesign pads each with what the
        // linker's signature left there, OmniBuild with zeros: nothing
        // covers the padding.
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
        // Their seals, and the app's executable, up to the end of its
        // signature, and seal, as codesign signs the app around that
        // framework and those extensions.
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
        let executable = app("OmniBuildShell");
        assert_eq!(
            sha256(&executable[..33_300]),
            "b1e7b9abdce4d2afa81d8f703edcb4fac6b63e5b06f590a00b4d7c1c3b790bcd"
        );
        assert_eq!(executable[33_300..], [0; 12]);
        assert_eq!(
            sha256(&app("_CodeSignature/CodeResources")),
            "4edfb64094908380096ee2a13516b380bf3a0ee96a74677895d78ec789707c29"
        );
    }

    #[test]
    fn signs_a_device_build_with_an_identity_as_codesign_does() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        let profile = Profile::parse(PROFILE).unwrap();
        let identity = Identity {
            key: &key,
            profiles: std::slice::from_ref(&profile),
        };

        let signed = sign(
            shell.clone(),
            DEVICE,
            &info(&shell, DEVICE),
            Some(&identity),
        )
        .unwrap();

        let app = |signed: &[Entry], name: &str| file(signed, &format!("{DEVICE}{name}"));
        let seal = "Frameworks/ShellKit.framework/_CodeSignature/CodeResources";
        let extension = "PlugIns/ShellNotifications.appex/ShellNotifications";
        let extension_seal = "PlugIns/ShellNotifications.appex/_CodeSignature/CodeResources";
        let intents = "Extensions/ShellIntents.appex/ShellIntents";
        // codesign signing with the test identity, and the extensions and
        // the app with the entitlements the profile gives each, wrote the
        // framework's and the extensions' seals, and these code directories
        // but for the hash of the first page. That page has the
        // load commands, which say how much room the signature has:
        // codesign leaves 16 KB more than it needs, OmniBuild none. A CMS
        // signature has the time it was made, so the framework's signed
        // executable differs each time, and with it the app's seal, which
        // the app's code directory hashes in turn.
        assert_eq!(app(&signed, "embedded.mobileprovision"), PROFILE);
        for extension in [
            "PlugIns/ShellNotifications.appex/",
            "Extensions/ShellIntents.appex/",
        ] {
            assert_eq!(
                app(&signed, &format!("{extension}embedded.mobileprovision")),
                PROFILE
            );
        }
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
        let files: Vec<(String, Vec<u8>)> = signed
            .iter()
            .filter_map(|entry| {
                let name = entry.name.strip_prefix(DEVICE)?;
                let sealed = !name.is_empty()
                    && !name.ends_with('/')
                    && name != "OmniBuildShell"
                    && !name.starts_with("_CodeSignature/");
                sealed.then(|| (name.to_owned(), entry.contents().unwrap()))
            })
            .collect();
        assert_eq!(
            app(&signed, "_CodeSignature/CodeResources"),
            code_resources(&files)
        );
        // Each with its LC_CODE_SIGNATURE where it is, and its first page's
        // hash after the 88-byte header, the identifier, and the special
        // slots: 7 of the app's and each extension's, the third from last
        // their seal's, and 3 of the framework's. The extensions' seals are
        // the same each time.
        let without = |executable: &[u8], load_command: usize, hashes: &[usize]| {
            let mut directory = code_directory(executable, load_command);
            for &hash in hashes {
                directory[hash..hash + 32].fill(0);
            }
            sha256(&directory)
        };
        assert_eq!(
            without(&app(&signed, "OmniBuildShell"), 784, &[332, 332 - 3 * 32]),
            "a256c0df08ee701402118e79085834a70cad5f70494e9376269cf6b3cc739593"
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
            without(
                &app(&signed, "Frameworks/ShellKit.framework/ShellKit"),
                720,
                &[213]
            ),
            "9fbd90b02c41c4df12db009fb09de9540ab5d75e0b3e283da7b4c1c69193a573"
        );
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
    fn gives_each_bundle_the_profile_that_fits_it_closest() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        let wildcard = Profile::parse(PROFILE).unwrap();
        let app_only = profile_for("ABCDE12345.dev.omnibuild.shell");
        let profiles = [wildcard, app_only];

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
    }

    #[test]
    fn leaves_out_what_ditto_stores_of_extended_attributes() {
        let mut shell = zip::read(SHELL_APP).unwrap();
        for name in [
            "OmniBuildShell.app/._Assets.car",
            "OmniBuildShell.app/.__CodeSignature",
            "__MACOSX/OmniBuildShell.app/._PkgInfo",
        ] {
            shell.push(Entry::new(name, Method::Stored, b"attributes"));
        }

        let signed = sign(shell.clone(), SIMULATOR, &info(&shell, SIMULATOR), None).unwrap();

        // Sealed as before.
        assert_eq!(
            sha256(&file(
                &signed,
                "OmniBuildShell.app/_CodeSignature/CodeResources"
            )),
            "fb4f5937ed25968b32fe67085980a0c664c90d47fc34042bd1ab002801e6ecba"
        );
        assert_eq!(signed.len(), shell.len() - 3);
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
        let code = |name: &str| shell_with(Entry::new(name, Method::Stored, b""));
        let not_a_framework = |name: &str| {
            format!(
                "the shell has code in its bundle that isn't a framework or an app extension ({name}), which OmniBuild can't sign yet"
            )
        };
        let framework_info = |plist: &[u8]| {
            shell_with(Entry::new(
                "OmniBuildShell.app/Frameworks/ShellKit.framework/Info.plist",
                Method::Stored,
                plist,
            ))
        };
        let extension_info = |plist: &[u8]| {
            shell_with(Entry::new(
                "OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist",
                Method::Stored,
                plist,
            ))
        };
        let cases = [
            (
                code("OmniBuildShell.app/Watch/Watch.app/Watch"),
                not_a_framework("OmniBuildShell.app/Watch/Watch.app/Watch"),
            ),
            (
                code("OmniBuildShell.app/AppClips/Clip.app/Clip"),
                not_a_framework("OmniBuildShell.app/AppClips/Clip.app/Clip"),
            ),
            (
                code("OmniBuildShell.app/PlugIns/Share.bundle/Share"),
                not_a_framework("OmniBuildShell.app/PlugIns/Share.bundle/Share"),
            ),
            (
                code("OmniBuildShell.app/Extensions/Intents.bundle/Intents"),
                not_a_framework("OmniBuildShell.app/Extensions/Intents.bundle/Intents"),
            ),
            (
                code("OmniBuildShell.app/Extensions/ShellIntents.appex/PlugIns/A.appex/A"),
                not_a_framework("OmniBuildShell.app/Extensions/ShellIntents.appex/PlugIns/A.appex/A"),
            ),
            (
                code("OmniBuildShell.app/PlugIns/ShellNotifications.appex/Frameworks/A.framework/A"),
                not_a_framework(
                    "OmniBuildShell.app/PlugIns/ShellNotifications.appex/Frameworks/A.framework/A",
                ),
            ),
            (
                extension_info(b"<plist><dict/></plist>"),
                "the shell's OmniBuildShell.app/PlugIns/ShellNotifications.appex/Info.plist has no bundle ID (CFBundleIdentifier)".to_owned(),
            ),
            (
                code("OmniBuildShell.app/PlugIns/Share.appex/Share"),
                "the shell has no OmniBuildShell.app/PlugIns/Share.appex/Info.plist".to_owned(),
            ),
            (
                shell_with(Entry::new(
                    "OmniBuildShell.app/PlugIns/ShellNotifications.appex/ShellNotifications",
                    Method::Stored,
                    b"not an executable",
                )),
                "couldn't sign OmniBuildShell.app/PlugIns/ShellNotifications.appex/ShellNotifications".to_owned(),
            ),
            (
                code("OmniBuildShell.app/Frameworks/libswiftCore.dylib"),
                not_a_framework("OmniBuildShell.app/Frameworks/libswiftCore.dylib"),
            ),
            (
                code("OmniBuildShell.app/Frameworks/A.framework/Frameworks/B.framework/B"),
                not_a_framework("OmniBuildShell.app/Frameworks/A.framework/Frameworks/B.framework/B"),
            ),
            (
                code("OmniBuildShell.app/Frameworks/Lynx.framework/Lynx"),
                "the shell has no OmniBuildShell.app/Frameworks/Lynx.framework/Info.plist".to_owned(),
            ),
            (
                framework_info(b"not a plist"),
                "couldn't read the shell's OmniBuildShell.app/Frameworks/ShellKit.framework/Info.plist".to_owned(),
            ),
            (
                zip::read(SHELL_APP)
                    .unwrap()
                    .into_iter()
                    .filter(|entry| !entry.name.ends_with("ShellKit.framework/ShellKit"))
                    .collect(),
                "the shell has no OmniBuildShell.app/Frameworks/ShellKit.framework/ShellKit, the executable its Info.plist names".to_owned(),
            ),
            (
                framework_info(b"<plist><dict/></plist>"),
                "the shell's OmniBuildShell.app/Frameworks/ShellKit.framework/Info.plist has no bundle ID (CFBundleIdentifier)".to_owned(),
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
