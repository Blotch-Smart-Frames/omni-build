//! Repackaging Android app shells.

pub mod key;
mod manifest;
mod signing;
mod string_pool;

use anyhow::{Context, Result, bail};

use crate::config::App;
use crate::zip;
use key::SigningKey;

/// Repackages an APK shell into the app `app` describes, and signs it with
/// `key`.
pub fn repackage(shell: &[u8], app: &App, key: &SigningKey) -> Result<Vec<u8>> {
    let mut entries = zip::read(shell).context("the shell isn't an APK")?;
    let Some(manifest) = entries
        .iter_mut()
        .find(|entry| entry.name == "AndroidManifest.xml")
    else {
        bail!("the shell isn't an APK: it has no AndroidManifest.xml");
    };
    let contents = manifest.contents()?;
    let rewritten = manifest::rewrite(&contents, app)
        .context("couldn't rewrite the shell's AndroidManifest.xml")?;
    // Left as it was when nothing changed, rather than compressed again.
    if rewritten != contents {
        *manifest = manifest.with_contents(&rewritten);
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
    use crate::config::App;
    use crate::zip::{self, Method, tests::SHELL, tests::entry};

    #[test]
    fn signs_the_shell_with_the_key() {
        let app = repackage(SHELL, &App::default(), &KEY).unwrap();

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

        let app = repackage(&shell, &App::default(), &KEY).unwrap();

        assert_eq!(zip::read(&app).unwrap(), zip::read(SHELL).unwrap());
    }

    #[test]
    fn gives_the_shell_the_apps_values() {
        let app = App {
            id: Some("dev.example.app".to_owned()),
            ..App::default()
        };

        let apk = repackage(SHELL, &app, &KEY).unwrap();

        let (shell, apk) = (zip::read(SHELL).unwrap(), zip::read(&apk).unwrap());
        let manifest = apk[2].contents().unwrap();
        assert_eq!(apk[2].name, "AndroidManifest.xml");
        assert!(manifest.windows(30).any(|window| {
            window
                == "dev.example.app"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>()
        }));
        // Every other entry is as it was.
        assert_eq!(
            [&apk[..2], &apk[3..]].concat(),
            [&shell[..2], &shell[3..]].concat()
        );
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
        let not_zip = repackage(b"not a zip", &App::default(), &KEY).unwrap_err();
        let no_manifest = repackage(
            &zip::write(&[entry("classes.dex", Method::Deflated, b"dex")]).bytes,
            &App::default(),
            &KEY,
        )
        .unwrap_err();
        let bad_manifest = repackage(
            &zip::write(&[entry("AndroidManifest.xml", Method::Stored, b"<manifest/>")]).bytes,
            &App::default(),
            &KEY,
        )
        .unwrap_err();
        let corrupt_entry = repackage(
            &zip::write(&[entry("AndroidManifest.xml", Method::Deflated, &[0xff; 8])]).bytes,
            &App::default(),
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
        assert_eq!(
            format!("{bad_manifest:#}"),
            "couldn't rewrite the shell's AndroidManifest.xml: it isn't binary XML"
        );
        assert_eq!(corrupt_entry.to_string(), "AndroidManifest.xml is corrupt");
    }
}
