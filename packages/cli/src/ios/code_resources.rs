//! The resource seal, `_CodeSignature/CodeResources`: a hash of each of
//! an app's files besides its executable, which the executable's
//! signature hashes in turn, so that changing any file breaks it. It's
//! written as codesign writes it for an iOS app, which the tests compare it
//! with byte for byte:
//!
//! - `files`: each file's SHA-1, for iOS versions before 9.
//! - `files2`: each file's SHA-256, but for `Info.plist`, which the
//!   signature hashes itself, and `PkgInfo`.
//! - `rules` and `rules2`: codesign's rules for which files each list
//!   seals, and how. OmniBuild applies only those its shells' files need:
//!   localized resources (`.lproj`) would be sealed as optional, and none
//!   of the shells has any.

use plist::{Dictionary, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// The seal of `files`, by their paths in the bundle.
pub fn code_resources(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut files: Vec<&(String, Vec<u8>)> = files.iter().collect();
    files.sort_by(|(a, _), (b, _)| a.cmp(b));
    let v1: Dictionary = files
        .iter()
        .map(|(name, contents)| (name.clone(), Value::Data(Sha1::digest(contents).to_vec())))
        .collect();
    let v2: Dictionary = files
        .iter()
        .filter(|(name, _)| name != "Info.plist" && name != "PkgInfo")
        .map(|(name, contents)| {
            let hash = Value::Data(Sha256::digest(contents).to_vec());
            (name.clone(), dictionary(&[("hash2", hash)]))
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
        // out of order.
        let files: Vec<(String, Vec<u8>)> = [
            "payload.txt",
            "Info.plist",
            "AppIcon60x60@2x.png",
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

        let seal = code_resources(&files);

        // What codesign wrote signing the same app ad hoc.
        assert_eq!(
            format!("{:x}", Sha256::digest(&seal)),
            "305a8450879786d090c10650a686e258e3b7bd1560d66ef66d211bfc5a561713"
        );
    }
}
