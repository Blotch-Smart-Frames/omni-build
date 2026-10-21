//! The resource seal, `_CodeSignature/CodeResources`: a hash of each of
//! an app's files besides its executable, which the executable's
//! signature hashes in turn, so that changing any file breaks it. It's
//! written as codesign writes it for an iOS app, which the tests compare it
//! with byte for byte:
//!
//! - `files`: each file's SHA-1, for iOS versions before 9.
//! - `files2`: each file's SHA-256, but for `Info.plist`, which the
//!   signature hashes itself, and `PkgInfo`. In a bundle whose executable
//!   is signed with SHA-1 hashes as well, a watch app for watchOS before 12
//!   (`macho::needs_sha1`), its SHA-1 too, as codesign seals one.
//! - `rules` and `rules2`: codesign's rules for which files each list
//!   seals, and how. OmniBuild applies those its shells' files need:
//!   localized resources (`.lproj`), but for `Base.lproj`'s, are optional,
//!   so iOS accepts an app without some of its languages, and their
//!   `locversion.plist` isn't sealed at all. A framework's files are sealed
//!   like any other, its own seal and signed code among them.

use plist::{Dictionary, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// The seal of `files`, by their paths in the bundle, with each file's SHA-1
/// in `files2` too if `sha1`.
pub fn code_resources(files: &[(String, Vec<u8>)], sha1: bool) -> Vec<u8> {
    let mut files: Vec<&(String, Vec<u8>)> = files
        .iter()
        .filter(|(name, _)| !(is_localized(name) && name.ends_with("/locversion.plist")))
        .collect();
    files.sort_by(|(a, _), (b, _)| a.cmp(b));
    // An optional file's hash is in a dictionary that says so.
    let optional = |name: &str| is_optional(name).then(|| ("optional", Value::Boolean(true)));
    let v1: Dictionary = files
        .iter()
        .map(|(name, contents)| {
            let hash = Value::Data(Sha1::digest(contents).to_vec());
            let hash = match optional(name) {
                Some(optional) => dictionary(&[("hash", hash), optional]),
                None => hash,
            };
            (name.clone(), hash)
        })
        .collect();
    let v2: Dictionary = files
        .iter()
        .filter(|(name, _)| name != "Info.plist" && name != "PkgInfo")
        .map(|(name, contents)| {
            let hashes: Vec<(&str, Value)> = [
                sha1.then(|| ("hash", Value::Data(Sha1::digest(contents).to_vec()))),
                Some(("hash2", Value::Data(Sha256::digest(contents).to_vec()))),
                optional(name),
            ]
            .into_iter()
            .flatten()
            .collect();
            (name.clone(), dictionary(&hashes))
        })
        .collect();
    let seal = dictionary(&[
        ("files", Value::Dictionary(v1)),
        ("files2", Value::Dictionary(v2)),
        ("rules", rules()),
        ("rules2", rules2()),
    ]);
    let mut xml = Vec::new();
    seal.to_writer_xml(&mut xml)
        .expect("writing into memory can't fail");
    xml.push(b'\n');
    xml
}

/// Whether the file is in a language's folder (`^.*\.lproj/`). As
/// codesign's rule, case-sensitive.
fn is_localized(name: &str) -> bool {
    name.split('/').rev().skip(1).any(|folder| {
        folder
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension == "lproj")
    })
}

/// Whether the file is sealed as optional: a localized one, but for the
/// base localization's, which a heavier rule seals as any other.
fn is_optional(name: &str) -> bool {
    is_localized(name) && !name.starts_with("Base.lproj/")
}

/// codesign's rules for `files`.
fn rules() -> Value {
    dictionary(&[
        ("^.*", Value::Boolean(true)),
        (r"^.*\.lproj/", optional(1000.0)),
        (r"^.*\.lproj/locversion.plist$", omit(1100.0)),
        (r"^Base\.lproj/", weight(1010.0)),
        ("^version.plist$", Value::Boolean(true)),
    ])
}

/// codesign's rules for `files2`.
fn rules2() -> Value {
    dictionary(&[
        (r".*\.dSYM($|/)", weight(11.0)),
        (r"^(.*/)?\.DS_Store$", omit(2000.0)),
        ("^.*", Value::Boolean(true)),
        (r"^.*\.lproj/", optional(1000.0)),
        (r"^.*\.lproj/locversion.plist$", omit(1100.0)),
        (r"^Base\.lproj/", weight(1010.0)),
        (r"^Info\.plist$", omit(20.0)),
        ("^PkgInfo$", omit(20.0)),
        (r"^embedded\.provisionprofile$", weight(20.0)),
        (r"^version\.plist$", weight(20.0)),
    ])
}

fn optional(weight: f64) -> Value {
    dictionary(&[
        ("optional", Value::Boolean(true)),
        ("weight", Value::Real(weight)),
    ])
}

fn omit(weight: f64) -> Value {
    dictionary(&[
        ("omit", Value::Boolean(true)),
        ("weight", Value::Real(weight)),
    ])
}

fn weight(weight: f64) -> Value {
    dictionary(&[("weight", Value::Real(weight))])
}

fn dictionary(entries: &[(&str, Value)]) -> Value {
    Value::Dictionary(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::code_resources;
    use crate::ios::tests::shell_file;

    #[test]
    fn seals_the_files_as_codesign_does() {
        // The Simulator shell's files, besides its executable and its seal,
        // out of order: its framework's as Xcode signed them, and English
        // strings.
        let files: Vec<(String, Vec<u8>)> = [
            "payload.txt",
            "Info.plist",
            "AppIcon60x60@2x.png",
            "en.lproj/InfoPlist.strings",
            "Frameworks/ShellKit.framework/ShellKit",
            "Frameworks/ShellKit.framework/_CodeSignature/CodeResources",
            "Frameworks/ShellKit.framework/Info.plist",
            "Assets.car",
            "AppIcon76x76@2x~ipad.png",
            "PkgInfo",
        ]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                shell_file(&format!("OmniBuildShell.app/{name}")),
            )
        })
        .collect();

        let seal = code_resources(&files, false);

        // What codesign wrote signing the same app ad hoc.
        assert_eq!(format!("{:x}", Sha256::digest(&seal)), SIMULATOR_SEAL);
    }

    /// codesign's seal of the Simulator shell, with its framework as it is.
    const SIMULATOR_SEAL: &str = "e10333ecc0de39a0368835f25af896b40188f2c5e74cda7a55eed64a3b65b190";

    #[test]
    fn seals_localized_files_as_optional_but_the_base_ones() {
        let file = |name: &str| (name.to_owned(), name.as_bytes().to_vec());

        let seal = code_resources(
            &[
                file("Base.lproj/Main.storyboardc"),
                file("fr.lproj/InfoPlist.strings"),
                file("fr.lproj/locversion.plist"),
                file("Settings.bundle/de.lproj/Root.strings"),
            ],
            false,
        );

        let seal = plist::Value::from_reader_xml(seal.as_slice()).unwrap();
        let files = seal.as_dictionary().unwrap()["files2"]
            .as_dictionary()
            .unwrap();
        let optional = |name: &str| {
            files[name]
                .as_dictionary()
                .unwrap()
                .get("optional")
                .is_some()
        };
        assert!(!optional("Base.lproj/Main.storyboardc"));
        assert!(optional("fr.lproj/InfoPlist.strings"));
        assert!(optional("Settings.bundle/de.lproj/Root.strings"));
        assert!(!files.contains_key("fr.lproj/locversion.plist"));
    }

    #[test]
    fn seals_with_sha1_hashes_too_where_the_code_has_them() {
        // A bundle with a watch app's executable for watchOS 11, which
        // codesign signs with SHA-1 hashes too, and these files.
        let files: Vec<(String, Vec<u8>)> = [
            (
                "Info.plist",
                &b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.example.watch</string><key>CFBundleExecutable</key><string>Mini</string></dict></plist>\n"[..],
            ),
            ("PkgInfo", b"APPL????"),
            ("payload.txt", b"payload"),
            ("Base.lproj/Main.strings", b"base"),
            ("en.lproj/Localizable.strings", b"english"),
            ("en.lproj/locversion.plist", b"version"),
        ]
        .into_iter()
        .map(|(name, contents)| (name.to_owned(), contents.to_vec()))
        .collect();

        let seal = code_resources(&files, true);

        // What codesign wrote signing it ad hoc: each file's SHA-1 in
        // files2 too, before its SHA-256, and the localized one's before
        // that it's optional.
        assert_eq!(
            format!("{:x}", Sha256::digest(&seal)),
            "4aa7ff0e924a4b424a71b01d83a26cfc8998bf996cfc070d6619b4e3954b282e"
        );
    }
}
