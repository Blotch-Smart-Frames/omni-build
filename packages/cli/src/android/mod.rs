//! Repackaging Android app shells: APKs, which install on a device, and
//! Android App Bundles (AABs), which Google Play turns into APKs.

mod binary_xml;
mod chunk;
mod icons;
mod jar_signing;
mod manifest;
mod proto_resources;
mod proto_xml;
mod protobuf;
mod resources;
mod signing;
mod string_pool;

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::App;
use crate::key::SigningKey;
use crate::payload;
use crate::zip::{self, Entry, Method};
use binary_xml::BinaryXml;
use manifest::{Changes, Document};
use proto_resources::ProtoResources;
use proto_xml::ProtoXml;
use protobuf::Wire;
use resources::{ResourceTable, Resources};

/// Where a package keeps what OmniBuild rewrites. An APK keeps it at its
/// root, compiled for a device; an AAB keeps it in its base module, in
/// protobuf, for bundletool to compile for each device.
pub struct Layout {
    manifest: &'static str,
    resources: &'static str,
    /// What the resource table's file paths, and `assets/`, are under.
    base: &'static str,
    /// Whether it's an AAB, which only Google Play installs from.
    pub bundle: bool,
}

const APK: Layout = Layout {
    manifest: "AndroidManifest.xml",
    resources: "resources.arsc",
    base: "",
    bundle: false,
};

/// The path field of a `TargetedNativeDirectory`, in an app bundle's
/// `native.pb` (bundletool's `files.proto`).
const NATIVE_PATH: u32 = 1;
/// Android 7.0, the first to verify APK Signature Scheme v2. An APK for
/// earlier versions needs a v1 (JAR) signature too, which is all they
/// verify.
const V2_MIN_SDK: u32 = 24;
/// Android 4.3, the first to verify a v1 signature made with SHA-256, as
/// OmniBuild's are. Earlier versions need SHA-1, which OmniBuild doesn't
/// sign with: no Android the AndroidX libraries support is that old.
const V1_MIN_SDK: u32 = 18;

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
/// and v3, for devices, and v1 too if it runs on Android before 7.0, and an
/// AAB with a JAR signature, as Google Play checks an upload.
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
    // Google Play signs the APKs it makes from an app bundle itself, for
    // whichever versions the app runs on.
    if !layout.bundle && changes.min_sdk < V1_MIN_SDK {
        bail!(
            "the shell runs on Android before 4.3 (its minSdkVersion is {}), whose v1 signatures need SHA-1: OmniBuild signs APKs for Android 4.3 (API 18) and later",
            changes.min_sdk
        );
    }
    // Left as it was when nothing changed, rather than compressed again.
    if manifest != contents {
        entries[index] = entries[index].with_contents(&manifest);
    }
    if !changes.name_resources.is_empty() || !changes.icon_resources.is_empty() {
        set_resources(&mut entries, layout, app, &changes)?;
    }
    if let Some(abis) = &app.android.abis {
        keep_abis(&mut entries, layout, abis)?;
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, layout, payload)?;
    }
    if layout.bundle {
        return Ok(zip::write(&jar_signing::sign_bundle(entries, key)?).bytes);
    }
    // v1 first, so that v2 and v3, which sign the whole file, sign it too.
    if changes.min_sdk < V2_MIN_SDK {
        entries = jar_signing::sign_apk(entries, key)?;
    }
    Ok(signing::sign(&zip::write(&entries), key))
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
    entries: &mut Vec<Entry>,
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
    if let Some(name) = &app.name {
        for &resource in &changes.name_resources {
            resources.set_string(resource, name).with_context(|| {
                format!("couldn't give the app its name in {}", layout.resources)
            })?;
        }
    }
    icons::set(
        entries,
        layout,
        resources.as_mut(),
        &changes.icon_resources,
        app,
    )?;
    // By its name again: giving the app its icon may add and remove files.
    let entry = entries
        .iter_mut()
        .find(|entry| entry.name == layout.resources)
        .expect("the table was read from it");
    let contents = resources.to_bytes();
    if contents != entry.contents().expect("the table was read from it") {
        *entry = entry.with_contents(&contents);
    }
    Ok(())
}

/// Keeps only the native libraries for the ABIs the app asks for: each
/// ABI's are in `lib/<abi>/`. An app bundle lists the folders in
/// `native.pb` too, as `TargetedNativeDirectory` messages: their paths,
/// and which ABI each is for.
fn keep_abis(entries: &mut Vec<Entry>, layout: &Layout, abis: &[String]) -> Result<()> {
    let libraries = format!("{}lib/", layout.base);
    let abi_of = |name: &str| -> Option<String> {
        let (abi, _) = name.strip_prefix(&libraries)?.split_once('/')?;
        Some(abi.to_owned())
    };
    let mut shell_abis: Vec<String> = entries
        .iter()
        .filter_map(|entry| abi_of(&entry.name))
        .collect();
    shell_abis.sort();
    shell_abis.dedup();
    for abi in abis {
        if !shell_abis.contains(abi) {
            bail!(
                "the shell has no native libraries for {abi}: {}",
                if shell_abis.is_empty() {
                    "it has none at all".to_owned()
                } else {
                    format!("it has them for {}", shell_abis.join(", "))
                }
            );
        }
    }
    entries.retain(|entry| abi_of(&entry.name).is_none_or(|abi| abis.contains(&abi)));
    let native = format!("{}native.pb", layout.base);
    if let Some(entry) = entries.iter_mut().find(|entry| entry.name == native) {
        let mut directories = protobuf::decode(&entry.contents()?)
            .with_context(|| format!("couldn't read the shell's {native}"))?;
        directories.retain(|directory| match &directory.value {
            Wire::Bytes(message) => protobuf::decode(message)
                .ok()
                .and_then(|fields| protobuf::string(&fields, NATIVE_PATH))
                .and_then(|path| path.strip_prefix("lib/").map(str::to_owned))
                .is_none_or(|abi| abis.contains(&abi)),
            _ => true,
        });
        *entry = entry.with_contents(&protobuf::encode(&directories));
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

    use image::RgbaImage;

    use super::chunk::chunks;
    use super::protobuf::{self, Field, Wire};
    use super::{
        ProtoResources, ProtoXml, ResourceTable, Resources, SigningKey, is_jar_signature, layout,
        read_resources,
    };
    use crate::android::binary_xml::BinaryXml;
    use crate::android::binary_xml::tests::{
        Value::{Ref, Str},
        binary_xml, names, texts,
    };
    use crate::android::manifest::{Document, Value};
    use crate::android::proto_xml::tests::SHELL_AAB;
    use crate::android::resources::Value as ResourceValue;
    use crate::android::resources::tests::strings;
    use crate::config::{AdaptiveIcon, Android, App, Background};
    use crate::icon::tests::solid_png;
    use crate::key::tests::KEY;
    use crate::zip::{self, Entry, Method, tests::SHELL, tests::entry, tests::named};

    const ORANGE: [u8; 4] = [0xe8, 0x59, 0x0c, 0xff];
    const GREEN: [u8; 4] = [0, 0xff, 0, 0xff];
    const BLUE: [u8; 4] = [0, 0, 0xff, 0xff];

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

    /// The shell, APK or AAB, with `min_sdk` as its minSdkVersion.
    fn shell_for(shell: &[u8], min_sdk: u32) -> Vec<u8> {
        let entries = zip::read(shell).unwrap();
        let layout = layout(&entries).unwrap();
        let contents = file(&entries, layout.manifest);
        let mut manifest: Box<dyn Document> = if layout.bundle {
            Box::new(ProtoXml::parse(&contents).unwrap())
        } else {
            Box::new(BinaryXml::parse(&contents).unwrap())
        };
        let index = manifest
            .attributes()
            .iter()
            .position(|attribute| attribute.name == "minSdkVersion")
            .unwrap();
        manifest.set_int(index, min_sdk);
        changed(shell, &[(layout.manifest, manifest.to_bytes())], &[])
    }

    #[test]
    fn signs_an_apk_for_android_before_7_with_v1_too() {
        let shell = shell_for(SHELL, 21);

        let apk = repackage(&shell, &App::default(), &KEY).unwrap();

        let entries = zip::read(&apk).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names[..3],
            [
                "META-INF/MANIFEST.MF",
                "META-INF/CERT.SF",
                "META-INF/CERT.RSA"
            ]
        );
        assert_eq!(entries[3..], zip::read(&shell).unwrap());
        let signature_file = String::from_utf8(file(&entries, "META-INF/CERT.SF")).unwrap();
        assert!(signature_file.contains("\r\nX-Android-APK-Signed: 2, 3\r\n"));
        // And v2 and v3 after it, over the whole file.
        assert!(apk.windows(16).any(|window| window == b"APK Sig Block 42"));
    }

    #[test]
    fn reports_an_entry_it_cant_sign_for_android_before_7() {
        let mut entries = zip::read(&shell_for(SHELL, 21)).unwrap();
        let mut corrupt = entry("assets/corrupt.bin", Method::Deflated, b"data");
        corrupt.data = vec![0xff; 8];
        entries.push(corrupt);
        let layout = layout(&entries).unwrap();

        let error = super::repackage(entries, layout, &App::default(), &KEY).unwrap_err();

        assert_eq!(error.to_string(), "assets/corrupt.bin is corrupt");
    }

    #[test]
    fn refuses_an_apk_for_android_before_4_3() {
        let error = repackage(&shell_for(SHELL, 17), &App::default(), &KEY).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the shell runs on Android before 4.3 (its minSdkVersion is 17), whose v1 signatures need SHA-1: OmniBuild signs APKs for Android 4.3 (API 18) and later"
        );
        // Google Play signs the APKs it makes from an app bundle itself.
        repackage(&shell_for(SHELL_AAB, 17), &App::default(), &KEY).unwrap();
    }

    /// The contents of the entry `name`.
    fn file(entries: &[Entry], name: &str) -> Vec<u8> {
        named(entries, name).contents().unwrap()
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
        let utf16: Vec<u8> = "dev.example.app"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(
            file(&apk, "AndroidManifest.xml")
                .windows(utf16.len())
                .any(|window| window == utf16)
        );
        // The application's label, and the launcher activity's.
        let table = ResourceTable::parse(&file(&apk, "resources.arsc")).unwrap();
        assert_eq!(strings(&table, 0x7f03_0000).unwrap(), ["Repackaged"]);
        assert_eq!(strings(&table, 0x7f03_0001).unwrap(), ["Repackaged"]);
        // The table stays uncompressed, as Android requires.
        assert_eq!(named(&apk, "resources.arsc").method, Method::Stored);
        // Every other entry is as it was.
        assert_eq!(apk.len(), shell.len());
        for entry in &shell {
            if !["AndroidManifest.xml", "resources.arsc"].contains(&entry.name.as_str()) {
                assert_eq!(named(&apk, &entry.name), entry);
            }
        }
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
        let mut table = zip::tests::file(SHELL, "resources.arsc");
        // The package's size, after the table's header and its string pool.
        let pool = u32::from_le_bytes(table[16..20].try_into().unwrap()) as usize;
        table[12 + pool + 4..12 + pool + 8].copy_from_slice(&4u32.to_le_bytes());
        Entry::new("resources.arsc", Method::Stored, &table)
    }

    fn with_icon(path: &Path) -> App {
        App {
            icon: Some(path.to_owned()),
            ..App::default()
        }
    }

    /// An image file, of either kind.
    fn image(bytes: &[u8]) -> RgbaImage {
        image::load_from_memory(bytes).unwrap().into_rgba8()
    }

    /// The elements of an adaptive icon in an APK.
    fn layers(xml: &[u8]) -> Vec<String> {
        BinaryXml::parse(xml)
            .unwrap()
            .attributes()
            .iter()
            .map(|attribute| attribute.element.clone())
            .collect()
    }

    #[test]
    fn gives_the_shell_the_apps_icon_and_payload() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(512, 512, ORANGE)).unwrap();
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

        // The icon and round icon for Android before 8.0: a WebP for each
        // density, at its size, as the shell has them.
        for (bitmap, size) in [
            ("res/d2.webp", 48),
            ("res/sK.webp", 192),
            ("res/yw.webp", 48),
            ("res/-6.webp", 192),
        ] {
            let entry = named(&apk, bitmap);
            assert_eq!(&entry.contents().unwrap()[8..12], b"WEBP");
            let icon = image(&entry.contents().unwrap());
            assert_eq!(icon.dimensions(), (size, size));
            assert_eq!(icon.get_pixel(size / 2, size / 2).0, ORANGE);
            assert_eq!(entry.method, Method::Stored);
        }
        // The adaptive icon's layers, both vectors in the shell: the app's
        // icon whole as the background, and nothing in front of it.
        let table = ResourceTable::parse(&file(&apk, "resources.arsc")).unwrap();
        let background = "res/drawable/omnibuild_7f010000.png";
        let foreground = "res/drawable/omnibuild_7f010001.png";
        assert_eq!(strings(&table, 0x7f01_0000).unwrap(), [background]);
        assert_eq!(strings(&table, 0x7f01_0001).unwrap(), [foreground]);
        let background = image(&file(&apk, background));
        assert_eq!(background.dimensions(), (432, 432));
        assert_eq!(background.get_pixel(216, 216).0, ORANGE);
        assert_eq!(image(&file(&apk, foreground)), RgbaImage::new(432, 432));
        // No monochrome layer, as the app has none.
        for adaptive_icon in ["res/BW.xml", "res/0K.xml"] {
            assert_eq!(
                layers(&file(&apk, adaptive_icon)),
                ["background", "foreground"]
            );
        }
        assert_eq!(file(&apk, "assets/payload.txt"), b"the app's payload");
        let bundle = named(&apk, "assets/bundle/main.lynx.bundle");
        assert_eq!(bundle.contents().unwrap(), b"bundle");
        assert_eq!(bundle.method, Method::Deflated);
        assert!(!apk.iter().any(|entry| entry.name.ends_with(".DS_Store")));
    }

    /// The shell with the files `replaced`, and without those `removed`.
    fn changed(shell: &[u8], replaced: &[(&str, Vec<u8>)], removed: &[&str]) -> Vec<u8> {
        let mut entries = zip::read(shell).unwrap();
        entries.retain(|entry| !removed.contains(&entry.name.as_str()));
        for (name, contents) in replaced {
            let index = entries
                .iter()
                .position(|entry| entry.name == *name)
                .unwrap();
            entries[index] = entries[index].with_contents(contents);
        }
        zip::write(&entries).bytes
    }

    /// The shell's resources.arsc with the type chunks of one type, by its
    /// ID, broken: their entries said to start past their end.
    fn broken_type(type_id: u8) -> Vec<u8> {
        let mut table = zip::tests::file(SHELL, "resources.arsc");
        let pool = u32::from_le_bytes(table[16..20].try_into().unwrap()) as usize;
        let package = chunks(&table, 12 + pool, table.len()).unwrap().remove(0);
        for chunk in chunks(&table, package.body(), package.end()).unwrap() {
            if chunk.kind == 0x0201 && table[chunk.start + 8] == type_id {
                table[chunk.start + 16..chunk.start + 20].copy_from_slice(&u32::MAX.to_le_bytes());
            }
        }
        table
    }

    /// The shell's resource table, in either format, with `change` made.
    fn table_with(shell: &[u8], change: impl FnOnce(&mut dyn Resources)) -> (String, Vec<u8>) {
        let entries = zip::read(shell).unwrap();
        let layout = layout(&entries).unwrap();
        let mut table = read_resources(&file(&entries, layout.resources), layout).unwrap();
        change(table.as_mut());
        (layout.resources.to_owned(), table.to_bytes())
    }

    #[test]
    fn gives_an_adaptive_icon_the_apps_layers() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, color: [u8; 4]| {
            let path = dir.path().join(name);
            fs::write(&path, solid_png(108, 108, color)).unwrap();
            path
        };
        let app = |background, monochrome| App {
            icon: Some(write("icon.png", ORANGE)),
            android: Android {
                adaptive_icon: Some(AdaptiveIcon {
                    foreground: write("front.png", GREEN),
                    background,
                    monochrome,
                }),
                ..Android::default()
            },
            ..App::default()
        };
        // A shell whose background is a colour, and whose foreground is a
        // bitmap per density, as Android Studio makes from an image.
        let (table, colored) = table_with(SHELL_AAB, |table| {
            table.set_color(0x7f01_0000, 0xff2f_6feb).unwrap();
            table
                .set_file(0x7f01_0001, "res/drawable-xxxhdpi/front.png")
                .unwrap();
        });
        let mut entries = zip::read(&changed(SHELL_AAB, &[(&table, colored)], &[])).unwrap();
        entries.push(Entry::new(
            "base/res/drawable-xxxhdpi/front.png",
            Method::Stored,
            &solid_png(432, 432, [0; 4]),
        ));
        let shell = zip::write(&entries).bytes;

        let colors = zip::read(
            &repackage(&shell, &app(Background::Color([0, 0, 0xff]), None), &KEY).unwrap(),
        )
        .unwrap();
        // The colour's resource names a PNG of the icon, without a colour
        // of the app's.
        let default =
            zip::read(&repackage(&shell, &with_icon(&write("icon.png", ORANGE)), &KEY).unwrap())
                .unwrap();
        let images = zip::read(
            &repackage(
                SHELL_AAB,
                &app(Background::Image(write("back.png", BLUE)), None),
                &KEY,
            )
            .unwrap(),
        )
        .unwrap();

        let values = |aab: &[Entry], id| {
            ProtoResources::parse(&file(aab, "base/resources.pb"))
                .unwrap()
                .values(id)
                .unwrap()
        };
        // A colour stays one, and each bitmap is redrawn at its size.
        assert_eq!(
            values(&colors, 0x7f01_0000),
            [ResourceValue::Color(0xff00_00ff)]
        );
        assert_eq!(
            values(&default, 0x7f01_0000),
            [ResourceValue::File(
                "res/drawable/omnibuild_7f010000.png".to_owned()
            )]
        );
        let front = image(&file(&colors, "base/res/drawable-xxxhdpi/front.png"));
        assert_eq!(
            (front.dimensions(), front.get_pixel(0, 0).0),
            ((432, 432), GREEN)
        );
        // In place of the shell's vectors, a PNG of each layer. The
        // foreground's drawable is the monochrome layer's too, which keeps
        // it.
        let background = "res/drawable/omnibuild_7f010000.png";
        assert_eq!(
            values(&images, 0x7f01_0000),
            [ResourceValue::File(background.to_owned())]
        );
        assert_eq!(
            image(&file(&images, &format!("base/{background}")))
                .get_pixel(9, 9)
                .0,
            BLUE
        );
        let foreground = image(&file(&images, "base/res/drawable/omnibuild_7f010001.png"));
        assert_eq!(foreground.get_pixel(9, 9).0, GREEN);
        let adaptive_icon =
            ProtoXml::parse(&file(&images, "base/res/mipmap-anydpi-v26/ic_launcher.xml")).unwrap();
        assert_eq!(
            adaptive_icon
                .attributes()
                .iter()
                .map(|attribute| attribute.element.as_str())
                .collect::<Vec<_>>(),
            ["background", "foreground", "monochrome"]
        );
        // The vectors go, as nothing names them.
        assert!(
            !images
                .iter()
                .any(|entry| entry.name.starts_with("base/res/drawable/ic_launcher_"))
        );
    }

    #[test]
    fn gives_the_icon_to_a_shell_without_an_adaptive_icon() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(64, 64, ORANGE)).unwrap();
        // A bitmap for the icon and the round icon, and nothing for
        // Android 8.0 and later.
        let (table, bitmaps) = table_with(SHELL, |table| {
            table.set_file(0x7f02_0000, "res/d2.webp").unwrap();
            table.set_file(0x7f02_0001, "res/yw.webp").unwrap();
        });
        let shell = changed(SHELL, &[(&table, bitmaps)], &[]);

        let apk = zip::read(&repackage(&shell, &with_icon(&icon), &KEY).unwrap()).unwrap();

        assert_eq!(image(&file(&apk, "res/d2.webp")).get_pixel(9, 9).0, ORANGE);
        // The adaptive icons' layers are as they were.
        assert!(!apk.iter().any(|entry| entry.name.contains("omnibuild_")));
    }

    #[test]
    fn fills_only_the_layers_an_adaptive_icon_draws_with() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(64, 64, ORANGE)).unwrap();
        let drawable = |id| ("drawable", Some(0x0101_0199), Ref(id));
        // A monochrome layer of its own, which the app has none for, and
        // drawables in other elements, and other attributes in the layers.
        let adaptive_icon = binary_xml(&[
            ("adaptive-icon", vec![]),
            ("background", vec![drawable(0x7f01_0000)]),
            (
                "foreground",
                vec![
                    drawable(0x7f01_0001),
                    ("inset", Some(0x0101_04b5), Str("16%")),
                ],
            ),
            ("monochrome", vec![drawable(0x7f03_0001)]),
            ("inset", vec![drawable(0x7f03_0000)]),
        ]);
        let shell = changed(
            SHELL,
            &[
                ("res/BW.xml", adaptive_icon.clone()),
                ("res/0K.xml", adaptive_icon),
            ],
            &[],
        );

        let apk = zip::read(&repackage(&shell, &with_icon(&icon), &KEY).unwrap()).unwrap();

        let table = ResourceTable::parse(&file(&apk, "resources.arsc")).unwrap();
        assert_eq!(
            strings(&table, 0x7f01_0001).unwrap(),
            ["res/drawable/omnibuild_7f010001.png"]
        );
        // The strings the monochrome layer and the inset name stay.
        assert_eq!(strings(&table, 0x7f03_0001).unwrap(), ["OmniBuild Shell"]);
        assert_eq!(strings(&table, 0x7f03_0000).unwrap(), ["OmniBuild Shell"]);
    }

    #[test]
    fn reports_a_shell_whose_icon_it_cant_replace() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(64, 64, [0; 4])).unwrap();
        let (_, not_a_file) = table_with(SHELL, |table| table.set_color(0x7f02_0000, 0).unwrap());
        let (_, missing_layer) = table_with(SHELL, |table| {
            table.set_file(0x7f01_0001, "res/front.png").unwrap();
        });
        let mut compressed = zip::read(SHELL).unwrap();
        let adaptive_icon = compressed
            .iter_mut()
            .find(|entry| entry.name == "res/BW.xml")
            .unwrap();
        (adaptive_icon.method, adaptive_icon.data) = (Method::Deflated, vec![0xff; 8]);
        let unreadable_icon = zip::write(&compressed).bytes;
        let not_layers = binary_xml(&[("adaptive-icon", vec![])]);
        let cases = [
            (
                shell_with(&["AndroidManifest.xml"], vec![]),
                with_icon(&icon),
                "the shell has no resources.arsc",
            ),
            (
                changed(SHELL, &[], &["res/d2.webp"]),
                with_icon(&icon),
                "the shell has no res/d2.webp, which its icon names",
            ),
            (
                changed(SHELL, &[("res/d2.webp", b"not an image".to_vec())], &[]),
                with_icon(&icon),
                "couldn't replace the shell's icon res/d2.webp: it isn't a PNG or a WebP, which are the icons OmniBuild replaces",
            ),
            (
                changed(SHELL, &[], &["res/BW.xml"]),
                with_icon(&icon),
                "the shell has no res/BW.xml, which its icon names",
            ),
            (
                changed(SHELL, &[("res/BW.xml", b"not XML".to_vec())], &[]),
                with_icon(&icon),
                "couldn't read the shell's res/BW.xml: it isn't binary XML",
            ),
            (
                changed(SHELL, &[("res/BW.xml", not_layers)], &[]),
                with_icon(&icon),
                "OmniBuild can't give the app its icon in the shell's res/BW.xml: it isn't an adaptive icon whose foreground and background each name a drawable",
            ),
            (
                changed(SHELL, &[("resources.arsc", not_a_file)], &[]),
                with_icon(&icon),
                "the shell's icon 0x7f020000 isn't a file in its package",
            ),
            (
                changed(SHELL, &[("resources.arsc", missing_layer)], &[]),
                with_icon(&icon),
                "the shell has no res/front.png, which its icon names",
            ),
            (
                unreadable_icon,
                with_icon(&icon),
                "res/BW.xml is corrupt: corrupt deflate stream",
            ),
            (
                // The drawables, which the layers are.
                changed(SHELL, &[("resources.arsc", broken_type(1))], &[]),
                with_icon(&icon),
                "it's corrupt or cut short",
            ),
            (
                // The strings, which only listing every file reads.
                changed(SHELL, &[("resources.arsc", broken_type(3))], &[]),
                with_icon(&icon),
                "it's corrupt or cut short",
            ),
            (
                shell_with(&["AndroidManifest.xml"], vec![broken_table()]),
                with_icon(&icon),
                "couldn't find the shell's icon in resources.arsc: it's corrupt or cut short",
            ),
        ];

        for (shell, app, message) in cases {
            let error = repackage(&shell, &app, &KEY).unwrap_err();

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
    fn reports_layers_it_cant_give_an_adaptive_icon() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.png");
        fs::write(&icon, solid_png(64, 64, [0; 4])).unwrap();
        let monochrome = App {
            android: Android {
                adaptive_icon: Some(AdaptiveIcon {
                    foreground: icon.clone(),
                    background: Background::Color([0; 3]),
                    monochrome: Some(icon.clone()),
                }),
                ..Android::default()
            },
            ..with_icon(&icon)
        };

        let error = repackage(SHELL, &monochrome, &KEY).unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "the shell's adaptive icon draws its foreground and monochrome layers with one drawable, so they can't differ: leave out the monochrome layer"
        );
        for layer in ["front", "back", "mono"] {
            let missing = |name: &str| {
                if name == layer {
                    dir.path().join("missing.png")
                } else {
                    icon.clone()
                }
            };
            let app = App {
                android: Android {
                    adaptive_icon: Some(AdaptiveIcon {
                        foreground: missing("front"),
                        background: Background::Image(missing("back")),
                        monochrome: Some(missing("mono")),
                    }),
                    ..Android::default()
                },
                ..with_icon(&icon)
            };
            let error = repackage(SHELL, &app, &KEY).unwrap_err();
            assert!(
                error.to_string().starts_with("couldn't read the icon "),
                "{error:#}"
            );
        }
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
            ..App::default()
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
        assert_eq!(strings(&resources, 0x7f03_0000).unwrap(), ["Repackaged"]);
        assert_eq!(strings(&resources, 0x7f03_0001).unwrap(), ["Repackaged"]);
        let icon = image(&file("base/res/mipmap-xxxhdpi-v4/ic_launcher.webp"));
        assert_eq!(icon.get_pixel(96, 96).0, ORANGE);
        // An app bundle names its files by where they are, and the layers
        // are PNGs, as aapt2 marks them.
        let background = image(&file("base/res/drawable/omnibuild_7f010000.png"));
        assert_eq!(background.get_pixel(216, 216).0, ORANGE);
        let adaptive_icon =
            ProtoXml::parse(&file("base/res/mipmap-anydpi-v26/ic_launcher_round.xml")).unwrap();
        assert_eq!(
            adaptive_icon
                .attributes()
                .iter()
                .map(|attribute| attribute.element.as_str())
                .collect::<Vec<_>>(),
            ["background", "foreground"]
        );
        assert_eq!(file("base/assets/payload.txt"), b"the app's payload");
    }

    #[test]
    fn asks_for_the_apps_permissions_in_a_bundle() {
        let app = App {
            android: Android {
                permissions: Some(vec!["VIBRATE".to_owned()]),
                ..Android::default()
            },
            ..App::default()
        };

        let aab = zip::read(&repackage(SHELL_AAB, &app, &KEY).unwrap()).unwrap();

        let manifest = ProtoXml::parse(&file(&aab, "base/manifest/AndroidManifest.xml")).unwrap();
        assert_eq!(
            names(&manifest, "uses-permission"),
            texts(&[
                "dev.omnibuild.shell.permission.SHELL",
                "android.permission.VIBRATE"
            ])
        );
    }

    #[test]
    fn opens_the_apps_urls_in_a_bundle_and_an_apk() {
        let app = App {
            url_schemes: Some(vec!["myapp".to_owned()]),
            ..App::default()
        };
        let schemes = |manifest: &dyn Document| -> Vec<Value> {
            manifest
                .attributes()
                .iter()
                .filter(|attribute| attribute.element == "data")
                .map(|attribute| attribute.value.clone())
                .collect()
        };

        let aab = zip::read(&repackage(SHELL_AAB, &app, &KEY).unwrap()).unwrap();
        let apk = zip::read(&repackage(SHELL, &app, &KEY).unwrap()).unwrap();

        let bundle = ProtoXml::parse(&file(&aab, "base/manifest/AndroidManifest.xml")).unwrap();
        let binary = BinaryXml::parse(&file(&apk, "AndroidManifest.xml")).unwrap();
        assert_eq!(schemes(&bundle), texts(&["myapp"]));
        assert_eq!(schemes(&binary), texts(&["myapp"]));
    }

    #[test]
    fn keeps_the_native_libraries_for_the_apps_abis() {
        let abis = |abis: &[&str]| App {
            android: Android {
                abis: Some(abis.iter().map(|&abi| abi.to_owned()).collect()),
                ..Android::default()
            },
            ..App::default()
        };
        let libraries = |entries: &[Entry], prefix: &str| -> Vec<String> {
            entries
                .iter()
                .filter_map(|entry| entry.name.strip_prefix(prefix).map(str::to_owned))
                .collect()
        };

        let apk =
            zip::read(&repackage(SHELL, &abis(&["x86_64", "arm64-v8a"]), &KEY).unwrap()).unwrap();
        let aab = zip::read(&repackage(SHELL_AAB, &abis(&["x86_64"]), &KEY).unwrap()).unwrap();

        assert_eq!(
            libraries(&apk, "lib/"),
            ["arm64-v8a/libshell.so", "x86_64/libshell.so"]
        );
        assert_eq!(libraries(&aab, "base/lib/"), ["x86_64/libshell.so"]);
        // An app bundle lists its native folders, and which ABI each is for.
        let native = protobuf::decode(&file(&aab, "base/native.pb")).unwrap();
        assert_eq!(native.len(), 1);
        assert!(
            matches!(&native[0].value, Wire::Bytes(directory) if directory.windows(10).any(|window| window == b"lib/x86_64"))
        );
    }

    #[test]
    fn says_why_it_cant_keep_an_abi() {
        let abis = |abis: &[&str]| App {
            android: Android {
                abis: Some(abis.iter().map(|&abi| abi.to_owned()).collect()),
                ..Android::default()
            },
            ..App::default()
        };
        let without_libraries = changed(
            SHELL,
            &[],
            &[
                "lib/arm64-v8a/libshell.so",
                "lib/armeabi-v7a/libshell.so",
                "lib/x86/libshell.so",
                "lib/x86_64/libshell.so",
            ],
        );
        // A folder bundletool wouldn't write, outside lib/, and a field
        // that isn't a folder, which stay as they are.
        let mut native = protobuf::decode(&zip::tests::file(SHELL_AAB, "base/native.pb")).unwrap();
        native.push(Field::bytes(
            1,
            protobuf::encode(&[Field::bytes(1, "other/x86")]),
        ));
        native.push(Field::varint(2, 7));
        let odd_native = changed(
            SHELL_AAB,
            &[("base/native.pb", protobuf::encode(&native))],
            &[],
        );
        let corrupt_native = changed(SHELL_AAB, &[("base/native.pb", vec![0xff])], &[]);
        let mut unreadable_native = zip::read(SHELL_AAB).unwrap();
        let native = unreadable_native
            .iter_mut()
            .find(|entry| entry.name == "base/native.pb")
            .unwrap();
        (native.method, native.data) = (Method::Deflated, vec![0xff; 8]);
        let unreadable_native = zip::write(&unreadable_native).bytes;
        // A file in lib/ of no ABI's, which stays.
        let mut stray = zip::read(SHELL).unwrap();
        stray.push(Entry::new("lib/README", Method::Deflated, b"libraries"));
        let stray = zip::write(&stray).bytes;

        let missing = repackage(SHELL, &abis(&["riscv64"]), &KEY).unwrap_err();
        let none = repackage(&without_libraries, &abis(&["x86_64"]), &KEY).unwrap_err();
        let corrupt = repackage(&corrupt_native, &abis(&["x86_64"]), &KEY).unwrap_err();
        let unreadable = repackage(&unreadable_native, &abis(&["x86_64"]), &KEY).unwrap_err();
        let kept = zip::read(&repackage(&stray, &abis(&["x86_64"]), &KEY).unwrap()).unwrap();
        let odd = zip::read(&repackage(&odd_native, &abis(&["x86"]), &KEY).unwrap()).unwrap();

        assert_eq!(
            missing.to_string(),
            "the shell has no native libraries for riscv64: it has them for arm64-v8a, armeabi-v7a, x86, x86_64"
        );
        assert_eq!(
            none.to_string(),
            "the shell has no native libraries for x86_64: it has none at all"
        );
        assert_eq!(
            format!("{corrupt:#}"),
            "couldn't read the shell's base/native.pb: it's corrupt or cut short"
        );
        assert_eq!(
            protobuf::decode(&file(&odd, "base/native.pb"))
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            format!("{unreadable:#}"),
            "base/native.pb is corrupt: corrupt deflate stream"
        );
        assert_eq!(file(&kept, "lib/README"), b"libraries");
    }

    #[test]
    fn reports_a_bundle_it_cant_repackage() {
        let without_manifest = zip::write(&[entry("BundleConfig.pb", Method::Stored, b"")]).bytes;
        let corrupt_manifest = zip::write(&[
            entry("BundleConfig.pb", Method::Stored, b""),
            entry("base/manifest/AndroidManifest.xml", Method::Stored, &[0xff]),
        ])
        .bytes;
        let corrupt_table = changed(SHELL_AAB, &[("base/resources.pb", vec![0xff])], &[]);
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
