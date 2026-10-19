//! Signing an iOS app's bundle, as codesign does: the profile goes in the
//! bundle, every file but the executable is sealed in
//! `_CodeSignature/CodeResources`, and the executable's signature covers
//! the seal, `Info.plist` and the entitlements.

use anyhow::{Context, Result, bail};

use super::code_resources::code_resources;
use super::code_signature::Signer;
use super::info_plist::InfoPlist;
use super::macho;
use super::profile::Profile;
use crate::key::SigningKey;
use crate::zip::{Entry, Method};

/// Where in a bundle iOS looks for code of its own, such as frameworks and
/// app extensions, each signed on its own before the app is.
const NESTED_CODE: [&str; 5] = [
    "Frameworks/",
    "PlugIns/",
    "Extensions/",
    "Watch/",
    "AppClips/",
];
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
    for entry in &entries {
        let file = entry.name.strip_prefix(bundle).unwrap_or_default();
        if NESTED_CODE.iter().any(|folder| file.starts_with(folder)) {
            bail!(
                "the shell has code in its bundle ({}), which OmniBuild can't sign yet",
                entry.name
            );
        }
        if entry.attributes >> 16 & FILE_TYPE == SYMBOLIC_LINK {
            bail!(
                "{} is a symbolic link, which OmniBuild can't sign yet",
                entry.name
            );
        }
    }

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

    let executable = info
        .executable()
        .map(|executable| format!("{bundle}{executable}"))
        .context("the shell's Info.plist has no executable (CFBundleExecutable)")?;
    let signature_folder = format!("{bundle}_CodeSignature/");
    let mut files = Vec::new();
    for entry in &entries {
        // Files only: not the bundle, nor any other folder.
        if let Some(file) = entry.name.strip_prefix(bundle)
            && !file.is_empty()
            && !file.ends_with('/')
            && entry.name != executable
            && !entry.name.starts_with(&signature_folder)
        {
            files.push((file.to_owned(), entry.contents()?));
        }
    }
    let seal = code_resources(&files);
    put(
        &mut entries,
        &format!("{signature_folder}CodeResources"),
        &seal,
    );

    let info_plist = files
        .iter()
        .find(|(name, _)| name == "Info.plist")
        .map(|(_, contents)| contents.as_slice())
        .unwrap_or_default();
    let code_signer = Signer {
        identifier,
        info_plist,
        code_resources: &seal,
        entitlements: entitlements.as_ref(),
        key: identity.map(|identity| identity.key),
    };
    let index = entries
        .iter()
        .position(|entry| entry.name == executable)
        .with_context(|| {
            format!("the shell has no {executable}, the executable its Info.plist names")
        })?;
    let signed = entries[index]
        .contents()
        .and_then(|code| macho::sign(&code, |code| code_signer.sign(code)))
        .with_context(|| format!("couldn't sign {executable}"))?;
    entries[index] = entries[index].with_contents(&signed);
    Ok(entries)
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

    /// The code directory in the device shell's signed executable.
    fn code_directory(executable: &[u8]) -> Vec<u8> {
        // Where its LC_CODE_SIGNATURE says the signature is.
        let start = Reader::at(executable, 784 + 8).u32() as usize;
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

        // What `codesign --sign -` wrote for the same app.
        assert_eq!(
            sha256(&file(&signed, "OmniBuildShell.app/OmniBuildShell")),
            "2d48a6d85b82cca90e3215862f694949c4fe227036bb17feeb616b951d91102a"
        );
        assert_eq!(
            sha256(&file(
                &signed,
                "OmniBuildShell.app/_CodeSignature/CodeResources"
            )),
            "305a8450879786d090c10650a686e258e3b7bd1560d66ef66d211bfc5a561713"
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

        // codesign's, up to the end of the signature. It pads with what
        // the linker's longer signature left there, OmniBuild with zeros:
        // nothing covers the padding.
        let executable = file(&ad_hoc, "Payload/OmniBuildShell.app/OmniBuildShell");
        assert_eq!(
            sha256(&executable[..33_300]),
            "f5303bcf8a107f59e69beaf3218257f5177a1dde06734e3d2b158d33cb7381c5"
        );
        assert_eq!(executable[33_300..], [0; 12]);
        assert_eq!(
            sha256(&file(
                &ad_hoc,
                "Payload/OmniBuildShell.app/_CodeSignature/CodeResources"
            )),
            "e9a4b03b02db686c75ee96507b057763a6612e3b82e6588cfeb790200bea8b1f"
        );
        // codesign signing with the test identity and the profile's
        // entitlements wrote this seal, and this code directory but for the
        // hash of the first page. That page has the load commands, which
        // say how much room the signature has: codesign leaves 16 KB more
        // than it needs, OmniBuild none. Its CMS signature has the time it
        // signed, so differs too.
        assert_eq!(
            file(
                &signed,
                "Payload/OmniBuildShell.app/embedded.mobileprovision"
            ),
            PROFILE
        );
        assert_eq!(
            sha256(&file(
                &signed,
                "Payload/OmniBuildShell.app/_CodeSignature/CodeResources"
            )),
            "98557049bcc672e15d6c34d9008bc1f4fbbc9fc2bf02ee8ca7177d2d1d292211"
        );
        let mut directory =
            code_directory(&file(&signed, "Payload/OmniBuildShell.app/OmniBuildShell"));
        // After the 88-byte header, the identifier, and 7 special slots.
        directory[332..364].fill(0);
        assert_eq!(
            sha256(&directory),
            "e7a8ee727bb2168ec6fc20af45f69ff59cb068560bbf5e56e21f2e4f5151e058"
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
            "305a8450879786d090c10650a686e258e3b7bd1560d66ef66d211bfc5a561713"
        );
        assert_eq!(signed.len(), shell.len() - 3);
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
                with(Entry::new(
                    "OmniBuildShell.app/Frameworks/Lynx.framework/Lynx",
                    Method::Stored,
                    b"",
                )),
                &shell_info,
                None,
                "the shell has code in its bundle (OmniBuildShell.app/Frameworks/Lynx.framework/Lynx), which OmniBuild can't sign yet",
            ),
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
            "the shell's Info.plist has no executable (CFBundleExecutable)"
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
