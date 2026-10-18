//! Repackaging Android app shells.

mod chunk;
mod icon;
pub mod key;
mod manifest;
mod resources;
mod signing;
mod string_pool;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::App;
use crate::zip::{self, Entry, Method};
use key::SigningKey;
use manifest::Rewritten;
use resources::ResourceTable;

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
    if rewritten.manifest != contents {
        *manifest = manifest.with_contents(&rewritten.manifest);
    }
    if rewritten.name_resource.is_some() || !rewritten.icon_resources.is_empty() {
        set_resources(&mut entries, app, &rewritten)?;
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, payload)?;
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

/// Gives the app its name and icon through the shell's resource table.
fn set_resources(entries: &mut [Entry], app: &App, rewritten: &Rewritten) -> Result<()> {
    let table = entries
        .iter()
        .position(|entry| entry.name == "resources.arsc")
        .context("the shell has no resources.arsc")?;
    let mut resources = entries[table]
        .contents()
        .and_then(|contents| ResourceTable::parse(&contents))
        .context("couldn't read the shell's resources.arsc")?;
    if let (Some(name), Some(resource)) = (&app.name, rewritten.name_resource) {
        resources
            .set_string(resource, name)
            .context("couldn't give the app its name in resources.arsc")?;
        entries[table] = entries[table].with_contents(&resources.to_bytes());
    }
    if let Some(path) = &app.icon {
        let icon = icon::load(path)?;
        // Each density's PNG, for the icon and the round icon, which may
        // share them.
        let mut files = Vec::new();
        for &resource in &rewritten.icon_resources {
            files.extend(
                resources
                    .strings(resource)
                    .context("couldn't find the shell's icon in resources.arsc")?,
            );
        }
        files.sort();
        files.dedup();
        for file in files {
            let entry = entries
                .iter_mut()
                .find(|entry| entry.name == file)
                .with_context(|| format!("the shell has no {file}, which its icon names"))?;
            let png = entry
                .contents()
                .and_then(|shell_icon| icon::resized(&icon, &shell_icon))
                .with_context(|| format!("couldn't replace the shell's icon {file}"))?;
            *entry = entry.with_contents(&png);
        }
    }
    Ok(())
}

/// Adds the payload's files to the APK's `assets/`, replacing the shell's
/// own files of the same name.
fn add_payload(entries: &mut Vec<Entry>, folder: &Path) -> Result<()> {
    for (name, path) in payload_files(folder)? {
        let contents =
            fs::read(&path).with_context(|| format!("couldn't read {}", path.display()))?;
        let name = format!("assets/{name}");
        match entries.iter().position(|entry| entry.name == name) {
            Some(index) => entries[index] = entries[index].with_contents(&contents),
            None => entries.push(Entry::new(&name, Method::Deflated, &contents)),
        }
    }
    Ok(())
}

/// The files in a payload folder and its subfolders: each one's path
/// relative to it, with `/` between folders, and its path on disk. Hidden
/// files (`.DS_Store`, `.gitkeep`) are left out, as aapt2 leaves them out
/// of assets.
fn payload_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    let mut folders = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = folders.pop() {
        let children = fs::read_dir(&folder)
            .and_then(Iterator::collect::<io::Result<Vec<_>>>)
            .with_context(|| format!("couldn't read the payload folder {}", folder.display()))?;
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if child.path().is_dir() {
                folders.push((child.path(), format!("{prefix}{name}/")));
            } else {
                files.push((format!("{prefix}{name}"), child.path()));
            }
        }
    }
    files.sort();
    Ok(files)
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
    use std::fs;
    use std::path::Path;

    use super::{is_jar_signature, repackage};
    use crate::android::icon::tests::solid_png;
    use crate::android::key::tests::KEY;
    use crate::config::App;
    use crate::zip::{self, Entry, Method, tests::SHELL, tests::entry};

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
            name: Some("Repackaged".to_owned()),
            ..App::default()
        };

        let apk = repackage(SHELL, &app, &KEY).unwrap();

        let (shell, apk) = (zip::read(SHELL).unwrap(), zip::read(&apk).unwrap());
        let contains =
            |data: &[u8], text: &[u8]| data.windows(text.len()).any(|window| window == text);
        let utf16 =
            |text: &str| -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        assert_eq!(
            (apk[2].name.as_str(), apk[4].name.as_str()),
            ("AndroidManifest.xml", "resources.arsc")
        );
        assert!(contains(
            &apk[2].contents().unwrap(),
            &utf16("dev.example.app")
        ));
        assert!(contains(&apk[4].contents().unwrap(), b"Repackaged"));
        // The table stays uncompressed, as Android requires.
        assert_eq!(apk[4].method, Method::Stored);
        // Every other entry is as it was.
        assert_eq!(apk[..2], shell[..2]);
        assert_eq!(apk[3], shell[3]);
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

    /// A shell with only these of its files, and the other given ones.
    fn shell_with(names: &[&str], others: Vec<Entry>) -> Vec<u8> {
        let mut entries: Vec<Entry> = zip::read(SHELL)
            .unwrap()
            .into_iter()
            .filter(|entry| names.contains(&entry.name.as_str()))
            .collect();
        entries.extend(others);
        zip::write(&entries).bytes
    }

    /// The shell's resource table with its package's size broken, which
    /// only shows when a resource is looked up.
    fn broken_table() -> Entry {
        let mut table = zip::read(SHELL).unwrap().swap_remove(4).contents().unwrap();
        // After the table's header and its 68-byte string pool.
        table[84..88].copy_from_slice(&4u32.to_le_bytes());
        Entry::new("resources.arsc", Method::Stored, &table)
    }

    fn with_icon(path: &Path) -> App {
        App {
            icon: Some(path.to_owned()),
            ..App::default()
        }
    }

    #[test]
    fn gives_the_shell_the_apps_icon_and_payload() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(512, 512, [0xe8, 0x59, 0x0c, 0xff])).unwrap();
        let payload = dir.path().join("payload");
        fs::create_dir_all(payload.join("bundle")).unwrap();
        fs::write(payload.join("payload.txt"), "the app's payload").unwrap();
        fs::write(payload.join("bundle/main.lynx.bundle"), "bundle").unwrap();
        fs::write(payload.join(".DS_Store"), "hidden").unwrap();
        let app = App {
            icon: Some(icon),
            payload: Some(payload),
            ..App::default()
        };

        let apk = zip::read(&repackage(SHELL, &app, &KEY).unwrap()).unwrap();

        let names: Vec<&str> = apk.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "META-INF/com/android/build/gradle/app-metadata.properties",
                "assets/payload.txt",
                "AndroidManifest.xml",
                "res/o-.png",
                "resources.arsc",
                "assets/bundle/main.lynx.bundle",
            ]
        );
        let icon = image::load_from_memory(&apk[3].contents().unwrap())
            .unwrap()
            .into_rgba8();
        assert_eq!(icon.dimensions(), (192, 192));
        assert_eq!(icon.get_pixel(96, 96).0, [0xe8, 0x59, 0x0c, 0xff]);
        assert_eq!(apk[3].method, Method::Stored);
        assert_eq!(apk[1].contents().unwrap(), b"the app's payload");
        assert_eq!(apk[5].contents().unwrap(), b"bundle");
        assert_eq!(apk[5].method, Method::Deflated);
    }

    #[test]
    fn reports_a_shell_whose_icon_it_cant_replace() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(64, 64, [0; 4])).unwrap();
        let cases = [
            (
                shell_with(&["AndroidManifest.xml"], vec![]),
                "the shell has no resources.arsc",
            ),
            (
                shell_with(&["AndroidManifest.xml", "resources.arsc"], vec![]),
                "the shell has no res/o-.png, which its icon names",
            ),
            (
                shell_with(
                    &["AndroidManifest.xml", "resources.arsc"],
                    vec![Entry::new(
                        "res/o-.png",
                        Method::Stored,
                        b"<adaptive-icon/>",
                    )],
                ),
                "couldn't replace the shell's icon res/o-.png: it isn't a PNG, and OmniBuild only replaces PNG icons so far, not adaptive ones",
            ),
            (
                shell_with(&["AndroidManifest.xml"], vec![broken_table()]),
                "couldn't find the shell's icon in resources.arsc: it's corrupt or cut short",
            ),
        ];

        for (shell, message) in cases {
            let error = repackage(&shell, &with_icon(&icon), &KEY).unwrap_err();

            assert_eq!(format!("{error:#}"), message);
        }
        let missing_icon =
            repackage(SHELL, &with_icon(&dir.path().join("missing.png")), &KEY).unwrap_err();
        assert!(
            missing_icon
                .to_string()
                .starts_with("couldn't read the icon ")
        );
    }

    #[test]
    fn reports_a_payload_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();
        let missing = App {
            payload: Some(dir.path().join("missing")),
            ..App::default()
        };

        let error = repackage(SHELL, &missing, &KEY).unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("couldn't read the payload folder ")
        );
    }

    #[cfg(unix)]
    #[test]
    fn reports_a_payload_file_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();
        // A link to nothing looks like a file, but can't be read.
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("payload.txt"))
            .unwrap();
        let app = App {
            payload: Some(dir.path().to_owned()),
            ..App::default()
        };

        let error = repackage(SHELL, &app, &KEY).unwrap_err();

        assert!(error.to_string().starts_with("couldn't read "));
        assert!(error.to_string().ends_with("payload.txt"));
    }

    #[test]
    fn reports_a_shell_without_a_table_it_can_name() {
        let named = App {
            name: Some("Repackaged".to_owned()),
            ..App::default()
        };
        let without_table = shell_with(&["AndroidManifest.xml"], vec![]);
        let bad_table = shell_with(
            &["AndroidManifest.xml"],
            vec![entry("resources.arsc", Method::Stored, b"table")],
        );
        let broken = shell_with(&["AndroidManifest.xml"], vec![broken_table()]);

        let missing = repackage(&without_table, &named, &KEY).unwrap_err();
        let corrupt = repackage(&bad_table, &named, &KEY).unwrap_err();
        let unnamed = repackage(&broken, &named, &KEY).unwrap_err();

        assert_eq!(missing.to_string(), "the shell has no resources.arsc");
        assert_eq!(
            format!("{corrupt:#}"),
            "couldn't read the shell's resources.arsc: it isn't a resource table"
        );
        assert_eq!(
            format!("{unnamed:#}"),
            "couldn't give the app its name in resources.arsc: it's corrupt or cut short"
        );
    }
}
