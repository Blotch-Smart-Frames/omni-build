//! Repackaging Android app shells: APKs, which install on a device, and
//! Android App Bundles (AABs), which Google Play turns into APKs.

mod binary_xml;
mod chunk;
mod jar_signing;
mod manifest;
mod proto_resources;
mod proto_xml;
mod protobuf;
mod resources;
mod signing;
mod string_pool;

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::App;
use crate::icon;
use crate::key::SigningKey;
use crate::payload;
use crate::zip::{self, Entry, Method};
use binary_xml::BinaryXml;
use manifest::{Changes, Document};
use proto_resources::ProtoResources;
use proto_xml::ProtoXml;
use resources::{ResourceTable, Resources};

/// Where a package keeps what OmniBuild rewrites. An APK keeps it at its
/// root, compiled for a device; an AAB keeps it in its base module, in
/// protobuf, for bundletool to compile for each device.
pub struct Layout {
    manifest: &'static str,
    resources: &'static str,
    /// What the resource table's file paths, and `assets/`, are under.
    base: &'static str,
    bundle: bool,
}

const APK: Layout = Layout {
    manifest: "AndroidManifest.xml",
    resources: "resources.arsc",
    base: "",
    bundle: false,
};

const AAB: Layout = Layout {
    manifest: "base/manifest/AndroidManifest.xml",
    resources: "base/resources.pb",
    base: "base/",
    bundle: true,
};

/// The layout of a shell's entries, if it's an APK or an AAB.
pub fn layout(entries: &[Entry]) -> Option<&'static Layout> {
    let has = |name: &str| entries.iter().any(|entry| entry.name == name);
    if has("BundleConfig.pb") {
        Some(&AAB)
    } else if has("AndroidManifest.xml") {
        Some(&APK)
    } else {
        None
    }
}

/// Repackages an APK or AAB shell, given its entries and their layout, into
/// the app `app` describes, and signs it with `key`: an APK with schemes v2
/// and v3, for devices, and an AAB with a JAR signature, as Google Play
/// checks an upload.
pub fn repackage(
    entries: Vec<Entry>,
    layout: &Layout,
    app: &App,
    key: &SigningKey,
) -> Result<Vec<u8>> {
    // The shell's own JAR signature, if it has one, describes the shell's
    // contents, not the app's.
    let mut entries: Vec<_> = entries
        .into_iter()
        .filter(|entry| !is_jar_signature(&entry.name))
        .collect();

    let index = entries
        .iter()
        .position(|entry| entry.name == layout.manifest)
        .with_context(|| format!("the shell has no {}", layout.manifest))?;
    let contents = entries[index].contents()?;
    let (manifest, changes) = rewrite_manifest(&contents, layout, app)
        .with_context(|| format!("couldn't rewrite the shell's {}", layout.manifest))?;
    // Left as it was when nothing changed, rather than compressed again.
    if manifest != contents {
        entries[index] = entries[index].with_contents(&manifest);
    }
    if changes.name_resource.is_some() || !changes.icon_resources.is_empty() {
        set_resources(&mut entries, layout, app, &changes)?;
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, layout, payload)?;
    }
    if layout.bundle {
        Ok(zip::write(&jar_signing::sign(entries, key)?).bytes)
    } else {
        Ok(signing::sign(&zip::write(&entries), key))
    }
}

fn rewrite_manifest(contents: &[u8], layout: &Layout, app: &App) -> Result<(Vec<u8>, Changes)> {
    let mut document: Box<dyn Document> = if layout.bundle {
        Box::new(ProtoXml::parse(contents)?)
    } else {
        Box::new(BinaryXml::parse(contents)?)
    };
    let changes = manifest::rewrite(document.as_mut(), app)?;
    Ok((document.to_bytes(), changes))
}

fn read_resources(contents: &[u8], layout: &Layout) -> Result<Box<dyn Resources>> {
    Ok(if layout.bundle {
        Box::new(ProtoResources::parse(contents)?)
    } else {
        Box::new(ResourceTable::parse(contents)?)
    })
}

/// Gives the app its name and icon through the shell's resource table.
fn set_resources(
    entries: &mut [Entry],
    layout: &Layout,
    app: &App,
    changes: &Changes,
) -> Result<()> {
    let table = entries
        .iter()
        .position(|entry| entry.name == layout.resources)
        .with_context(|| format!("the shell has no {}", layout.resources))?;
    let mut resources = entries[table]
        .contents()
        .and_then(|contents| read_resources(&contents, layout))
        .with_context(|| format!("couldn't read the shell's {}", layout.resources))?;
    if let (Some(name), Some(resource)) = (&app.name, changes.name_resource) {
        resources
            .set_string(resource, name)
            .with_context(|| format!("couldn't give the app its name in {}", layout.resources))?;
        entries[table] = entries[table].with_contents(&resources.to_bytes());
    }
    if let Some(path) = &app.icon {
        let icon = icon::load(path)?;
        // Each density's PNG, for the icon and the round icon, which may
        // share them.
        let mut files = Vec::new();
        for &resource in &changes.icon_resources {
            files.extend(resources.strings(resource).with_context(|| {
                format!("couldn't find the shell's icon in {}", layout.resources)
            })?);
        }
        files.sort();
        files.dedup();
        for file in files {
            let name = format!("{}{file}", layout.base);
            let entry = entries
                .iter_mut()
                .find(|entry| entry.name == name)
                .with_context(|| format!("the shell has no {name}, which its icon names"))?;
            let png = entry
                .contents()
                .and_then(|shell_icon| icon::resized(&icon, &shell_icon))
                .with_context(|| format!("couldn't replace the shell's icon {name}"))?;
            *entry = entry.with_contents(&png);
        }
    }
    Ok(())
}

/// Adds the payload's files to the package's `assets/`, replacing the
/// shell's own files of the same name.
fn add_payload(entries: &mut Vec<Entry>, layout: &Layout, folder: &Path) -> Result<()> {
    for (name, contents) in payload::read(folder)? {
        let name = format!("{}assets/{name}", layout.base);
        match entries.iter().position(|entry| entry.name == name) {
            Some(index) => entries[index] = entries[index].with_contents(&contents),
            None => entries.push(Entry::new(&name, Method::Deflated, &contents)),
        }
    }
    Ok(())
}

/// Whether an entry is part of a JAR signature (an APK's v1 signature, or
/// an AAB's): `META-INF/MANIFEST.MF`, and each signer's signature file and
/// signature block.
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

    use anyhow::Result;

    use super::{ProtoResources, ProtoXml, Resources, SigningKey, is_jar_signature, layout};
    use crate::android::manifest::{Document, Value};
    use crate::android::proto_xml::tests::SHELL_AAB;
    use crate::config::App;
    use crate::icon::tests::solid_png;
    use crate::key::tests::KEY;
    use crate::zip::{self, Entry, Method, tests::SHELL, tests::entry};

    /// Repackages an APK or AAB from its bytes, as `omni-build repackage`
    /// does.
    fn repackage(shell: &[u8], app: &App, key: &SigningKey) -> Result<Vec<u8>> {
        let entries = zip::read(shell).unwrap();
        let layout = layout(&entries).unwrap();
        super::repackage(entries, layout, app, key)
    }

    #[test]
    fn tells_an_apk_from_an_aab() {
        let manifest = |shell: &[u8]| layout(&zip::read(shell).unwrap()).unwrap().manifest;

        assert_eq!(manifest(SHELL), "AndroidManifest.xml");
        assert_eq!(manifest(SHELL_AAB), "base/manifest/AndroidManifest.xml");
        assert!(layout(&[entry("classes.dex", Method::Deflated, b"dex")]).is_none());
    }

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
    fn rejects_an_apk_it_cant_rewrite() {
        let bad_manifest = repackage(
            &zip::write(&[entry("AndroidManifest.xml", Method::Stored, b"<manifest/>")]).bytes,
            &App::default(),
            &KEY,
        )
        .unwrap_err();
        let bad_id = App {
            id: Some("app".to_owned()),
            ..App::default()
        };
        let unrewritable = repackage(SHELL, &bad_id, &KEY).unwrap_err();
        let corrupt_entry = repackage(
            &zip::write(&[entry("AndroidManifest.xml", Method::Deflated, &[0xff; 8])]).bytes,
            &App::default(),
            &KEY,
        )
        .unwrap_err();

        assert_eq!(
            format!("{bad_manifest:#}"),
            "couldn't rewrite the shell's AndroidManifest.xml: it isn't binary XML"
        );
        assert_eq!(corrupt_entry.to_string(), "AndroidManifest.xml is corrupt");
        assert!(
            format!("{unrewritable:#}").starts_with(
                "couldn't rewrite the shell's AndroidManifest.xml: \"app\" isn't a valid Android application ID"
            )
        );
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
                "couldn't replace the shell's icon res/o-.png: it isn't a PNG, and OmniBuild only replaces PNG icons so far",
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

    fn aab_with(others: Vec<Entry>) -> Vec<u8> {
        let mut entries = zip::read(SHELL_AAB).unwrap();
        entries.extend(others);
        zip::write(&entries).bytes
    }

    #[test]
    fn gives_a_bundle_the_apps_values_and_a_jar_signature() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(512, 512, [0xe8, 0x59, 0x0c, 0xff])).unwrap();
        let payload = dir.path().join("payload");
        fs::create_dir_all(&payload).unwrap();
        fs::write(payload.join("payload.txt"), "the app's payload").unwrap();
        let app = App {
            id: Some("dev.example.app".to_owned()),
            name: Some("Repackaged".to_owned()),
            version: Some("2.5.0".to_owned()),
            build: Some(42),
            icon: Some(icon),
            payload: Some(payload),
        };

        let aab = zip::read(&repackage(SHELL_AAB, &app, &KEY).unwrap()).unwrap();

        let file = |name: &str| {
            aab.iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .contents()
                .unwrap()
        };
        // OmniBuild's signature replaces the shell's.
        let names: Vec<&str> = aab.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names[..3],
            [
                "META-INF/MANIFEST.MF",
                "META-INF/CERT.SF",
                "META-INF/CERT.RSA"
            ]
        );
        assert!(
            !names
                .iter()
                .any(|name| name.starts_with("META-INF/ANDROIDD"))
        );
        let manifest = ProtoXml::parse(&file("base/manifest/AndroidManifest.xml")).unwrap();
        let value = |name: &str| {
            manifest
                .attributes()
                .iter()
                .find(|attribute| attribute.name == name)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(
            value("package"),
            Value::String("dev.example.app".to_owned())
        );
        assert_eq!(value("versionName"), Value::String("2.5.0".to_owned()));
        assert_eq!(value("versionCode"), Value::Int(42));
        let resources = ProtoResources::parse(&file("base/resources.pb")).unwrap();
        assert_eq!(resources.strings(0x7f02_0000).unwrap(), ["Repackaged"]);
        let icon = image::load_from_memory(&file("base/res/mipmap-xxxhdpi-v4/ic_launcher.png"))
            .unwrap()
            .into_rgba8();
        assert_eq!(icon.get_pixel(96, 96).0, [0xe8, 0x59, 0x0c, 0xff]);
        assert_eq!(file("base/assets/payload.txt"), b"the app's payload");
    }

    #[test]
    fn reports_a_bundle_it_cant_repackage() {
        let without_manifest = zip::write(&[entry("BundleConfig.pb", Method::Stored, b"")]).bytes;
        let corrupt_manifest = zip::write(&[
            entry("BundleConfig.pb", Method::Stored, b""),
            entry("base/manifest/AndroidManifest.xml", Method::Stored, &[0xff]),
        ])
        .bytes;
        let mut corrupt_table = zip::read(SHELL_AAB).unwrap();
        corrupt_table[11] = Entry::new("base/resources.pb", Method::Stored, &[0xff]);
        let corrupt_table = zip::write(&corrupt_table).bytes;
        let mut corrupt_entry = Entry::new("base/dex/classes.dex", Method::Deflated, b"dex");
        corrupt_entry.data = vec![0xff; 8];
        let named = App {
            name: Some("Repackaged".to_owned()),
            ..App::default()
        };

        for (shell, app, message) in [
            (
                without_manifest,
                App::default(),
                "the shell has no base/manifest/AndroidManifest.xml",
            ),
            (
                corrupt_manifest,
                App::default(),
                "couldn't rewrite the shell's base/manifest/AndroidManifest.xml: it's corrupt or cut short",
            ),
            (
                corrupt_table,
                named,
                "couldn't read the shell's base/resources.pb: it's corrupt or cut short",
            ),
            (
                aab_with(vec![corrupt_entry]),
                App::default(),
                "base/dex/classes.dex is corrupt: corrupt deflate stream",
            ),
        ] {
            assert_eq!(
                format!("{:#}", repackage(&shell, &app, &KEY).unwrap_err()),
                message
            );
        }
    }
}
