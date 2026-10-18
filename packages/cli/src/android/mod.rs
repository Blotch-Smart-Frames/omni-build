//! Repackaging Android app shells.

pub mod key;
mod signing;

use anyhow::{Context, Result, bail};

use crate::zip;
use key::SigningKey;

/// Repackages an APK shell and signs it with `key`.
pub fn repackage(shell: &[u8], key: &SigningKey) -> Result<Vec<u8>> {
    let entries = zip::read(shell).context("the shell isn't an APK")?;
    if !entries
        .iter()
        .any(|entry| entry.name == "AndroidManifest.xml")
    {
        bail!("the shell isn't an APK: it has no AndroidManifest.xml");
    }
    // The shell's own v1 signature, if it has one, describes the shell's
    // contents, not the app's. The new v2 and v3 signatures replace any it
    // had, since they cover the whole file.
    let entries: Vec<_> = entries
        .into_iter()
        .filter(|entry| !is_jar_signature(&entry.name))
        .collect();
    Ok(signing::sign(&zip::write(&entries), key))
}

/// Whether an entry is part of a v1 (JAR) signature: `META-INF/MANIFEST.MF`,
/// and each signer's signature file and signature block.
fn is_jar_signature(name: &str) -> bool {
    name.strip_prefix("META-INF/").is_some_and(|file| {
        !file.contains('/')
            && (file == "MANIFEST.MF"
                || [".SF", ".RSA", ".DSA", ".EC"]
                    .iter()
                    .any(|extension| file.ends_with(extension)))
    })
}

#[cfg(test)]
mod tests {
    use super::{is_jar_signature, repackage};
    use crate::android::key::tests::KEY;
    use crate::zip::{self, Method, tests::SHELL, tests::entry};

    #[test]
    fn signs_the_shell_with_the_key() {
        let app = repackage(SHELL, &KEY).unwrap();

        assert_eq!(zip::read(&app).unwrap(), zip::read(SHELL).unwrap());
        assert!(
            app.windows(KEY.certificate().len())
                .any(|window| window == KEY.certificate())
        );
    }

    #[test]
    fn drops_the_shells_v1_signature() {
        let mut entries = zip::read(SHELL).unwrap();
        entries.push(entry(
            "META-INF/MANIFEST.MF",
            Method::Deflated,
            b"Manifest-Version: 1.0",
        ));
        entries.push(entry(
            "META-INF/CERT.SF",
            Method::Deflated,
            b"Signature-Version: 1.0",
        ));
        let shell = zip::write(&entries).bytes;

        let app = repackage(&shell, &KEY).unwrap();

        assert_eq!(zip::read(&app).unwrap(), zip::read(SHELL).unwrap());
    }

    #[test]
    fn knows_v1_signature_files() {
        for name in [
            "META-INF/MANIFEST.MF",
            "META-INF/CERT.SF",
            "META-INF/CERT.RSA",
            "META-INF/KEY.DSA",
            "META-INF/KEY.EC",
        ] {
            assert!(is_jar_signature(name), "{name}");
        }
        for name in [
            "META-INF/com/android/build/gradle/app-metadata.properties",
            "META-INF/services/CERT.SF",
            "META-INF/kotlin.kotlin_module",
            "MANIFEST.MF",
        ] {
            assert!(!is_jar_signature(name), "{name}");
        }
    }

    #[test]
    fn rejects_what_isnt_an_apk() {
        let not_zip = repackage(b"not a zip", &KEY).unwrap_err();
        let no_manifest = repackage(
            &zip::write(&[entry("classes.dex", Method::Deflated, b"dex")]).bytes,
            &KEY,
        )
        .unwrap_err();

        assert_eq!(
            format!("{not_zip:#}"),
            "the shell isn't an APK: it isn't a zip archive"
        );
        assert_eq!(
            no_manifest.to_string(),
            "the shell isn't an APK: it has no AndroidManifest.xml"
        );
    }
}
