//! Signing an iOS app's bundle, as codesign does: the profile goes in the
//! bundle, every file but the executable is sealed in
//! `_CodeSignature/CodeResources`, and the executable's signature covers
//! the seal, `Info.plist` and the entitlements. Each framework in the app's
//! `Frameworks/` is signed the same way first, as a bundle of its own with
//! no entitlements, so the app's seal covers what they became, as Xcode
//! signs them.

use anyhow::{Context, Result, bail};

use super::code_resources::code_resources;
use super::code_signature::Signer;
use super::info_plist::InfoPlist;
use super::macho;
use super::profile::Profile;
use crate::key::SigningKey;
use crate::zip::{Entry, Method};

/// Where in a bundle iOS looks for code of its own besides frameworks: app
/// extensions, a watch app and App Clips, which need provisioning profiles
/// of their own.
const OTHER_CODE: [&str; 4] = ["PlugIns/", "Extensions/", "Watch/", "AppClips/"];
const FRAMEWORKS: &str = "Frameworks/";
/// A symbolic link's file type, in a zip entry's Unix mode.
const SYMBOLIC_LINK: u32 = 0o120_000;
const FILE_TYPE: u32 = 0o170_000;

/// What a build for devices is signed with: a key Apple issued a
/// certificate for, and a provisioning profile allowing it.
pub struct Identity<'a> {
    pub key: &'a SigningKey,
    pub profile: &'a Profile,
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
    let mut frameworks = Vec::new();
    for entry in &entries {
        let file = entry.name.strip_prefix(bundle).unwrap_or_default();
        if OTHER_CODE.iter().any(|folder| file.starts_with(folder)) {
            bail!(
                "the shell has code in its bundle that isn't a framework ({}), which OmniBuild can't sign yet",
                entry.name
            );
        }
        // The Frameworks folder's own entry, which isn't code.
        if let Some(code) = file
            .strip_prefix(FRAMEWORKS)
            .filter(|code| !code.is_empty())
        {
            // A framework's folder, as `Name.framework/`, and only that: a
            // library outside one, or code in a framework's own folders,
            // would have to be signed on its own too.
            let (folder, inside) = code.split_once('/').unwrap_or((code, ""));
            if !folder.ends_with(".framework") || inside.contains(".framework/") {
                bail!(
                    "the shell has code in its bundle that isn't a framework ({}), which OmniBuild can't sign yet",
                    entry.name
                );
            }
            frameworks.push(format!("{bundle}{FRAMEWORKS}{folder}/"));
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

    let identifier = info
        .id()
        .context("the shell's Info.plist has no bundle ID (CFBundleIdentifier)")?;
    let entitlements = identity
        .map(|identity| {
            identity
                .profile
                .entitlements(identifier, identity.key.certificate())
        })
        .transpose()
        .context("the provisioning profile doesn't fit the app")?;
    if let Some(identity) = identity {
        put(
            &mut entries,
            &format!("{bundle}embedded.mobileprovision"),
            &identity.profile.bytes,
        );
    }
    let key = identity.map(|identity| identity.key);

    for framework in &frameworks {
        let name = format!("{framework}Info.plist");
        let framework_info = entries
            .iter()
            .find(|entry| entry.name == name)
            .with_context(|| format!("the shell has no {name}"))?
            .contents()
            .and_then(|contents| InfoPlist::parse(&contents))
            .with_context(|| format!("couldn't read the shell's {name}"))?;
        let identifier = framework_info
            .id()
            .with_context(|| format!("the shell's {name} has no bundle ID (CFBundleIdentifier)"))?;
        sign_bundle(
            &mut entries,
            framework,
            &framework_info,
            &Signer {
                identifier,
                info_plist: &[],
                code_resources: &[],
                entitlements: None,
                key,
                main: false,
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
            entitlements: entitlements.as_ref(),
            key,
            main: true,
        },
    )?;
    Ok(entries)
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
    use crate::ios::profile::tests::PROFILE;
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
        // framework first.
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
            hash("OmniBuildShell"),
            "cf017c2fc059c6a8dc8ae12f21428ca70cba266ef4804c5774c28e5921f893c6"
        );
        assert_eq!(
            hash("_CodeSignature/CodeResources"),
            "7d089ac569d0a99b9e30a8fc77acd41ff83c4e0c200647ee3e019086073e3fc6"
        );
        let names = |entries: &[Entry]| -> Vec<String> {
            entries.iter().map(|entry| entry.name.clone()).collect()
        };
        assert_eq!(names(&signed), names(&shell));
    }

    #[test]
    fn signs_a_device_build_ad_hoc_or_with_an_identity_as_codesign_does() {
        let shell = zip::read(SHELL_IPA).unwrap();
        let key = identity();
        let profile = Profile::parse(PROFILE).unwrap();
        let identity = Identity {
            key: &key,
            profile: &profile,
        };

        let ad_hoc = sign(shell.clone(), DEVICE, &info(&shell, DEVICE), None).unwrap();
        let signed = sign(
            shell.clone(),
            DEVICE,
            &info(&shell, DEVICE),
            Some(&identity),
        )
        .unwrap();

        let app = |signed: &[Entry], name: &str| file(signed, &format!("{DEVICE}{name}"));
        // The framework's, as codesign signs it, up to the end of its
        // signature. codesign pads it with what the linker's signature left
        // there, OmniBuild with zeros: nothing covers the padding.
        let framework = app(&ad_hoc, "Frameworks/ShellKit.framework/ShellKit");
        assert_eq!(
            sha256(&framework[..33_245]),
            "c84a33211a8427b8d3c9da360ac75cb1a25dc946802dad469b775a8b3d242d90"
        );
        assert_eq!(framework[33_245..], [0; 19]);
        // The framework's seal, and the app's executable and seal, as
        // codesign signs the app around that framework.
        let seal = "Frameworks/ShellKit.framework/_CodeSignature/CodeResources";
        assert_eq!(
            sha256(&app(&ad_hoc, seal)),
            "b1bbabcd35a08d9d16e15144633ab1a9ff54ffdefc49a9885235bf4c847615e7"
        );
        assert_eq!(
            sha256(&app(&ad_hoc, "OmniBuildShell")),
            "450f1b22c243215fb98bc9f9c9b30d49c5ad700e10bc65a9cf28ac26d9a02197"
        );
        assert_eq!(
            sha256(&app(&ad_hoc, "_CodeSignature/CodeResources")),
            "5414fd7067d133959327f65567989ea8cccd323a2c2ac72ad48229bf1defa6da"
        );
        // codesign signing with the test identity, and the app with the
        // profile's entitlements, wrote the framework's seal, and these code
        // directories but for the hash of the first page. That page has the
        // load commands, which say how much room the signature has:
        // codesign leaves 16 KB more than it needs, OmniBuild none. A CMS
        // signature has the time it was made, so the framework's signed
        // executable differs each time, and with it the app's seal, which
        // the app's code directory hashes in turn.
        assert_eq!(app(&signed, "embedded.mobileprovision"), PROFILE);
        assert_eq!(
            sha256(&app(&signed, seal)),
            "b1bbabcd35a08d9d16e15144633ab1a9ff54ffdefc49a9885235bf4c847615e7"
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
        // slots: 7 of the app's, the third from last its seal's, and 3 of
        // the framework's.
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
            without(
                &app(&signed, "Frameworks/ShellKit.framework/ShellKit"),
                720,
                &[213]
            ),
            "9fbd90b02c41c4df12db009fb09de9540ab5d75e0b3e283da7b4c1c69193a573"
        );
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
            "7d089ac569d0a99b9e30a8fc77acd41ff83c4e0c200647ee3e019086073e3fc6"
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
                "the shell has code in its bundle that isn't a framework ({name}), which OmniBuild can't sign yet"
            )
        };
        let framework_info = |plist: &[u8]| {
            shell_with(Entry::new(
                "OmniBuildShell.app/Frameworks/ShellKit.framework/Info.plist",
                Method::Stored,
                plist,
            ))
        };
        let cases = [
            (
                code("OmniBuildShell.app/PlugIns/Share.appex/Share"),
                not_a_framework("OmniBuildShell.app/PlugIns/Share.appex/Share"),
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
        let profile = Profile::parse(PROFILE).unwrap();
        // A key whose certificate the profile doesn't allow.
        let stranger = Identity {
            key: &KEY,
            profile: &profile,
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
                "the provisioning profile doesn't fit the app",
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
