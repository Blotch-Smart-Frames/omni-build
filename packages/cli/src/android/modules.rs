//! Adding native modules to an Android app (see `crate::modules`). Each
//! module's DEX files go beside the shell's, as `classes2.dex` and on,
//! which Android 5.0 and later load as one app's code, its native libraries
//! go in `lib/`, in each ABI's folder, and the manifest asks for its
//! permissions. The manifest lists the modules for the shell too, which
//! registers each by its class's name when it starts (`Class.forName`).

use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result, bail};

use super::Layout;
use super::dex::Dex;
use super::manifest::{self, Document};
use super::protobuf::{self, Field, Wire};
use crate::modules::{Android, Module, check_runtime};
use crate::payload;
use crate::zip::{Entry, Method};

/// The `<meta-data>` a shell that registers native modules names its
/// runtime in, which each module has to be built for.
const RUNTIME: &str = "dev.omnibuild.runtime";
/// The `<meta-data>` that lists the modules for the shell, by name, between
/// commas.
const MODULES: &str = "dev.omnibuild.modules";
/// Android 5.0, the first to load every DEX file in an APK. Before it, an
/// app loaded the rest itself, with AndroidX's `MultiDex`.
const MULTIDEX_MIN_SDK: u32 = 21;
/// The ABIs Android has, as Gradle and the NDK name their folders, and as
/// bundletool's `AbiAlias` numbers them, in an app bundle's `native.pb`.
const ABIS: [(&str, u64); 8] = [
    ("armeabi", 1),
    ("armeabi-v7a", 2),
    ("arm64-v8a", 3),
    ("x86", 4),
    ("x86_64", 5),
    ("mips", 6),
    ("mips64", 7),
    ("riscv64", 8),
];
/// The fields of bundletool's `NativeLibraries`, `TargetedNativeDirectory`,
/// `NativeDirectoryTargeting` and `Abi` OmniBuild writes: a directory, its
/// path and targeting, an ABI, and its alias.
const DIRECTORY: u32 = 1;
const PATH: u32 = 1;
const TARGETING: u32 = 2;
const ABI: u32 = 1;
const ALIAS: u32 = 1;

/// A module's native library: its ABI, and its file's name and contents.
pub struct Library {
    pub abi: String,
    file: String,
    contents: Vec<u8>,
}

/// The modules that have something for Android, and what.
pub fn android(modules: &[Module]) -> Vec<(&Module, &Android)> {
    modules
        .iter()
        .filter_map(|module| Some((module, module.android.as_ref()?)))
        .collect()
}

/// Checks the modules are built for the runtime the shell's manifest
/// names.
pub fn check(manifest: &dyn Document, modules: &[(&Module, &Android)]) -> Result<()> {
    check_runtime(
        modules.iter().map(|&(module, _)| module),
        manifest::meta_data(manifest, RUNTIME).as_deref(),
        &format!("its manifest, as <meta-data android:name=\"{RUNTIME}\">"),
    )
}

/// Asks for the modules' permissions in the manifest, and lists the
/// modules there, after any the shell lists itself.
pub fn list(manifest: &mut dyn Document, modules: &[(&Module, &Android)]) -> Result<()> {
    let permissions: Vec<String> = modules
        .iter()
        .flat_map(|(_, android)| android.permissions.iter().cloned())
        .collect();
    manifest::add_permissions(manifest, &permissions)?;
    let mut names: Vec<String> = manifest::meta_data(manifest, MODULES)
        .into_iter()
        .filter(|names| !names.is_empty())
        .collect();
    names.extend(
        modules
            .iter()
            .flat_map(|(_, android)| android.modules.iter().cloned()),
    );
    manifest::set_meta_data(manifest, MODULES, &names.join(","))
}

/// Each module's native libraries, from its `jniLibs` folder, with the
/// module.
pub fn libraries<'a>(
    modules: &[(&'a Module, &Android)],
) -> Result<Vec<(&'a Module, Vec<Library>)>> {
    let mut libraries = Vec::new();
    for &(module, android) in modules {
        let Some(folder) = &android.jni_libs else {
            continue;
        };
        let mut own = Vec::new();
        for (path, contents) in payload::read_folder(folder, "the native libraries' folder")
            .with_context(|| format!("couldn't read {module}'s native libraries"))?
        {
            let Some((abi, file)) = path.split_once('/').filter(|(abi, file)| {
                !file.contains('/') && ABIS.iter().any(|&(name, _)| name == *abi)
            }) else {
                bail!(
                    "{module}'s native library {path} isn't in the folder of an ABI Android has, such as arm64-v8a"
                );
            };
            own.push(Library {
                abi: abi.to_owned(),
                file: file.to_owned(),
                contents,
            });
        }
        libraries.push((module, own));
    }
    Ok(libraries)
}

/// Adds the modules' native libraries for the app's ABIs, `abis`: stored,
/// for Android to map from an APK, unless it extracts them, and listed in
/// an app bundle's `native.pb`, for bundletool to split by ABI.
pub fn add_libraries(
    entries: &mut Vec<Entry>,
    layout: &Layout,
    libraries: &[(&Module, Vec<Library>)],
    abis: &[String],
    extract: bool,
) -> Result<()> {
    let method = if layout.bundle || extract {
        Method::Deflated
    } else {
        Method::Stored
    };
    for (module, own) in libraries {
        for library in own.iter().filter(|library| abis.contains(&library.abi)) {
            let name = format!("{}lib/{}/{}", layout.base, library.abi, library.file);
            if entries.iter().any(|entry| entry.name == name) {
                bail!(
                    "{module}'s {}/{} would replace the app's {name}, from the shell or another module",
                    library.abi,
                    library.file
                );
            }
            entries.push(Entry::new(&name, method, &library.contents));
        }
    }
    if layout.bundle {
        list_native_directories(entries, layout, abis)?;
    }
    Ok(())
}

/// Lists each ABI's folder of native libraries in an app bundle's
/// `native.pb`, which a shell with none may not have, as bundletool lists
/// them: a `TargetedNativeDirectory` with the folder's path and its ABI.
fn list_native_directories(
    entries: &mut Vec<Entry>,
    layout: &Layout,
    abis: &[String],
) -> Result<()> {
    let name = format!("{}native.pb", layout.base);
    let index = entries.iter().position(|entry| entry.name == name);
    let mut directories = match index {
        Some(index) => entries[index]
            .contents()
            .and_then(|contents| protobuf::decode(&contents))
            .with_context(|| format!("couldn't read the shell's {name}"))?,
        None => Vec::new(),
    };
    let listed: Vec<String> = directories
        .iter()
        .filter_map(|directory| match &directory.value {
            Wire::Bytes(message) => protobuf::string(&protobuf::decode(message).ok()?, PATH),
            _ => None,
        })
        .collect();
    for &(abi, alias) in ABIS
        .iter()
        .filter(|(abi, _)| abis.iter().any(|kept| kept == abi))
    {
        let path = format!("lib/{abi}");
        if listed.contains(&path) {
            continue;
        }
        let alias = protobuf::encode(&[Field::varint(ALIAS, alias)]);
        let targeting = protobuf::encode(&[Field::bytes(ABI, alias)]);
        directories.push(Field::bytes(
            DIRECTORY,
            protobuf::encode(&[Field::bytes(PATH, path), Field::bytes(TARGETING, targeting)]),
        ));
    }
    let contents = protobuf::encode(&directories);
    match index {
        Some(index) => entries[index] = entries[index].with_contents(&contents),
        None => entries.push(Entry::new(&name, Method::Deflated, &contents)),
    }
    Ok(())
}

/// Adds the modules' DEX files after the shell's, once each is checked to
/// run on the oldest Android the app does, `min_sdk`, and to define no
/// class the app has already.
pub fn add_dex(
    entries: &mut Vec<Entry>,
    layout: &Layout,
    modules: &[(&Module, &Android)],
    min_sdk: u32,
) -> Result<()> {
    let files: Vec<_> = modules
        .iter()
        .flat_map(|&(module, android)| android.dex.iter().map(move |path| (module, path)))
        .collect();
    let Some(&(first, _)) = files.first() else {
        return Ok(());
    };
    if min_sdk < MULTIDEX_MIN_SDK {
        bail!(
            "the shell runs on Android before 5.0 (its minSdkVersion is {min_sdk}), which loads only an app's first DEX file, and {first} has code of its own: OmniBuild adds native modules to apps for Android 5.0 (API 21) and later"
        );
    }
    let folder = if layout.bundle { "base/dex/" } else { "" };
    let is_dex = |name: &str| {
        name.strip_prefix(folder)
            .and_then(|file| file.strip_prefix("classes"))
            .and_then(|file| file.strip_suffix(".dex"))
            .is_some_and(|number| number.chars().all(|digit| digit.is_ascii_digit()))
    };
    // Who defines each class: the shell, or a module.
    let mut owners: HashMap<String, String> = HashMap::new();
    let mut count = 0;
    let mut method = Method::Deflated;
    for entry in entries.iter().filter(|entry| is_dex(&entry.name)) {
        let dex = entry
            .contents()
            .and_then(|contents| Dex::parse(&contents))
            .with_context(|| format!("couldn't read the shell's {}", entry.name))?;
        owners.extend(
            dex.classes
                .into_iter()
                .map(|class| (class, "the shell".to_owned())),
        );
        count += 1;
        method = entry.method;
    }
    for (module, path) in files {
        let what = format!("{module}'s {}", path.display());
        let contents = fs::read(path).with_context(|| format!("couldn't read {what}"))?;
        let dex = Dex::parse(&contents).with_context(|| format!("couldn't read {what}"))?;
        if dex.min_sdk > min_sdk {
            bail!(
                "{what} is for Android API {} and later (DEX {}), and the app runs on API {min_sdk} too: build it with d8 --min-api {min_sdk}",
                dex.min_sdk,
                dex.version
            );
        }
        let duplicates: Vec<&String> = dex
            .classes
            .iter()
            .filter(|class| owners.contains_key(*class))
            .collect();
        if let Some(class) = duplicates.first() {
            bail!(
                "{what} has {} classes the app has already, such as {class}, which {} has, and Android loads only one of a class: build the module's DEX from its own classes, with the libraries the shell has on d8's classpath",
                duplicates.len(),
                owners[*class]
            );
        }
        owners.extend(
            dex.classes
                .into_iter()
                .map(|class| (class, module.to_string())),
        );
        // Android loads classes.dex, then classes2.dex and on.
        count += 1;
        let number = if count == 1 {
            String::new()
        } else {
            count.to_string()
        };
        entries.push(Entry::new(
            &format!("{folder}classes{number}.dex"),
            method,
            &contents,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{MODULES, RUNTIME, add_dex, add_libraries, android, check, libraries, list};
    use crate::android::binary_xml::BinaryXml;
    use crate::android::binary_xml::tests::{names, shell_manifest};
    use crate::android::dex::tests::made_up;
    use crate::android::manifest::{self, Document, Value};
    use crate::android::proto_xml::ProtoXml;
    use crate::android::proto_xml::tests::SHELL_AAB;
    use crate::android::protobuf::{self, Field};
    use crate::android::{AAB as AAB_LAYOUT, APK as APK_LAYOUT};
    use crate::modules::tests::example;
    use crate::modules::{Android, Module};
    use crate::zip::{self, Entry, Method, tests::SHELL};

    /// A module, `name` 1.0.0, for the runtime `example@1`, with what it
    /// adds to an Android app.
    fn module(name: &str, android: Android) -> Module {
        Module {
            android: Some(android),
            ..example(name, "example@1")
        }
    }

    fn strings(strings: &[&str]) -> Vec<String> {
        strings.iter().map(|&string| string.to_owned()).collect()
    }

    #[test]
    fn lists_the_modules_and_asks_for_their_permissions() {
        let modules = [
            module(
                "haptics",
                Android {
                    modules: strings(&["dev.example.Haptics", "dev.example.Feedback"]),
                    permissions: strings(&["VIBRATE"]),
                    ..Android::default()
                },
            ),
            module(
                "camera",
                Android {
                    modules: strings(&["dev.example.Camera"]),
                    permissions: strings(&["CAMERA", "VIBRATE"]),
                    ..Android::default()
                },
            ),
            Module {
                android: None,
                ..example("ios-only", "example@2")
            },
        ];
        let modules = android(&modules);
        let bundle = zip::tests::file(SHELL_AAB, "base/manifest/AndroidManifest.xml");
        let documents: [(Box<dyn Document>, &str); 3] = [
            (Box::new(BinaryXml::parse(&shell_manifest()).unwrap()), ""),
            (Box::new(ProtoXml::parse(&bundle).unwrap()), ""),
            // A shell that registers modules of its own.
            (
                Box::new(BinaryXml::parse(&shell_manifest()).unwrap()),
                "dev.omnibuild.shell.ShellModule",
            ),
        ];

        assert_eq!(modules.len(), 2);
        for (mut document, own) in documents {
            manifest::set_meta_data(document.as_mut(), RUNTIME, "example@1").unwrap();
            manifest::set_meta_data(document.as_mut(), MODULES, own).unwrap();

            check(document.as_ref(), &modules).unwrap();
            list(document.as_mut(), &modules).unwrap();

            let listed = "dev.example.Haptics,dev.example.Feedback,dev.example.Camera";
            assert_eq!(
                manifest::meta_data(document.as_ref(), MODULES).unwrap(),
                if own.is_empty() {
                    listed.to_owned()
                } else {
                    format!("{own},{listed}")
                }
            );
            let permissions = names(document.as_ref(), "uses-permission");
            for permission in ["android.permission.VIBRATE", "android.permission.CAMERA"] {
                let requests = permissions
                    .iter()
                    .filter(|&name| *name == Value::String(permission.to_owned()))
                    .count();
                assert_eq!(requests, 1, "{permission}");
            }
        }
    }

    #[test]
    fn says_why_it_cant_list_the_modules() {
        let haptics = [module(
            "haptics",
            Android {
                permissions: strings(&["VIBRATE!"]),
                ..Android::default()
            },
        )];
        let modules = android(&haptics);
        let mut document = BinaryXml::parse(&shell_manifest()).unwrap();

        assert_eq!(
            check(&document, &modules).unwrap_err().to_string(),
            "haptics 1.0.0 is a native module, and the shell takes none: a shell that registers native modules names its runtime in its manifest, as <meta-data android:name=\"dev.omnibuild.runtime\">"
        );
        assert_eq!(
            list(&mut document, &modules).unwrap_err().to_string(),
            "\"VIBRATE!\" isn't the name of a permission, such as android.permission.CAMERA, or CAMERA for short"
        );
    }

    /// Writes a module's native libraries, by their paths in its `jniLibs`.
    fn jni_libs(folder: &Path, files: &[&str]) {
        for file in files {
            let path = folder.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("{file} code")).unwrap();
        }
    }

    #[test]
    fn reads_each_modules_native_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("jni");
        jni_libs(
            &folder,
            &["x86_64/libhaptics.so", "arm64-v8a/libhaptics.so"],
        );
        let modules = [
            module(
                "haptics",
                Android {
                    jni_libs: Some(folder.clone()),
                    ..Android::default()
                },
            ),
            module("plain", Android::default()),
        ];

        let read = libraries(&android(&modules)).unwrap();

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].0.name, "haptics");
        let files: Vec<(&str, &str, &[u8])> = read[0]
            .1
            .iter()
            .map(|library| {
                (
                    library.abi.as_str(),
                    library.file.as_str(),
                    &library.contents[..],
                )
            })
            .collect();
        assert_eq!(
            files,
            [
                (
                    "arm64-v8a",
                    "libhaptics.so",
                    &b"arm64-v8a/libhaptics.so code"[..]
                ),
                ("x86_64", "libhaptics.so", b"x86_64/libhaptics.so code"),
            ]
        );
        for file in [
            "libhaptics.so",
            "arm64/libhaptics.so",
            "x86/old/libhaptics.so",
        ] {
            let folder = dir.path().join(file.replace('/', "-"));
            jni_libs(&folder, &[file]);
            let modules = [module(
                "haptics",
                Android {
                    jni_libs: Some(folder),
                    ..Android::default()
                },
            )];
            assert_eq!(
                libraries(&android(&modules)).err().unwrap().to_string(),
                format!(
                    "haptics 1.0.0's native library {file} isn't in the folder of an ABI Android has, such as arm64-v8a"
                )
            );
        }
        let missing = [module(
            "haptics",
            Android {
                jni_libs: Some(dir.path().join("missing")),
                ..Android::default()
            },
        )];
        let error = libraries(&android(&missing)).err().unwrap();
        assert_eq!(
            format!("{error}: {}", error.source().unwrap()),
            format!(
                "couldn't read haptics 1.0.0's native libraries: couldn't read the native libraries' folder {}",
                dir.path().join("missing").display()
            )
        );
    }

    /// A module, `haptics`, with native libraries for arm64-v8a and
    /// `x86_64`.
    fn haptics_libraries(dir: &Path) -> Vec<Module> {
        let folder = dir.join("jni");
        jni_libs(
            &folder,
            &["arm64-v8a/libhaptics.so", "x86_64/libhaptics.so"],
        );
        vec![module(
            "haptics",
            Android {
                jni_libs: Some(folder),
                ..Android::default()
            },
        )]
    }

    /// The native libraries among the entries: each one's name and how
    /// it's stored.
    fn added(entries: &[Entry], prefix: &str) -> Vec<(String, Method)> {
        entries
            .iter()
            .filter(|entry| entry.name.starts_with(prefix) && entry.name.contains("libhaptics"))
            .map(|entry| (entry.name.clone(), entry.method))
            .collect()
    }

    #[test]
    fn adds_native_libraries_for_the_apps_abis() {
        let dir = tempfile::tempdir().unwrap();
        let modules = haptics_libraries(dir.path());
        let libraries = libraries(&android(&modules)).unwrap();
        let abis = strings(&["x86_64"]);

        // Stored, for Android to map from the APK, unless it extracts them.
        for (extract, method) in [(false, Method::Stored), (true, Method::Deflated)] {
            let mut entries = zip::read(SHELL).unwrap();
            add_libraries(&mut entries, &APK_LAYOUT, &libraries, &abis, extract).unwrap();
            assert_eq!(
                added(&entries, "lib/"),
                [("lib/x86_64/libhaptics.so".to_owned(), method)]
            );
        }
        // Already listed in native.pb, as the shell has x86_64 libraries.
        let mut entries = zip::read(SHELL_AAB).unwrap();
        let native = zip::tests::named(&entries, "base/native.pb")
            .contents()
            .unwrap();
        add_libraries(&mut entries, &AAB_LAYOUT, &libraries, &abis, false).unwrap();
        assert_eq!(
            added(&entries, "base/lib/"),
            [("base/lib/x86_64/libhaptics.so".to_owned(), Method::Deflated)]
        );
        assert_eq!(
            zip::tests::named(&entries, "base/native.pb")
                .contents()
                .unwrap(),
            native
        );
        let mut entries = zip::read(SHELL).unwrap();
        assert_eq!(
            add_libraries(&mut entries, &APK_LAYOUT, &libraries, &abis, false)
                .and_then(|()| add_libraries(&mut entries, &APK_LAYOUT, &libraries, &abis, false))
                .unwrap_err()
                .to_string(),
            "haptics 1.0.0's x86_64/libhaptics.so would replace the app's lib/x86_64/libhaptics.so, from the shell or another module"
        );
    }

    /// The paths and ABI aliases of the folders a `native.pb` lists.
    fn native_directories(native: &[u8]) -> Vec<(String, u64)> {
        protobuf::decode(native)
            .unwrap()
            .iter()
            .map(|directory| {
                let message = protobuf::bytes(std::slice::from_ref(directory), 1).unwrap();
                let fields = protobuf::decode(message).unwrap();
                let targeting = protobuf::message(&fields, 2);
                let abi = protobuf::message(&targeting, 1);
                (
                    protobuf::string(&fields, 1).unwrap(),
                    protobuf::uint(&abi, 1).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn lists_native_libraries_where_the_bundle_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let modules = haptics_libraries(dir.path());
        let libraries = libraries(&android(&modules)).unwrap();
        let abis = strings(&["arm64-v8a", "x86_64"]);
        let without_libraries = || -> Vec<Entry> {
            zip::read(SHELL_AAB)
                .unwrap()
                .into_iter()
                .filter(|entry| {
                    !entry.name.starts_with("base/lib/") && entry.name != "base/native.pb"
                })
                .collect()
        };
        let mut entries = without_libraries();

        add_libraries(&mut entries, &AAB_LAYOUT, &libraries, &abis, false).unwrap();

        let native = zip::tests::named(&entries, "base/native.pb")
            .contents()
            .unwrap();
        assert_eq!(
            native_directories(&native),
            [
                ("lib/arm64-v8a".to_owned(), 3),
                ("lib/x86_64".to_owned(), 5)
            ]
        );
        // As bundletool would write it: the same as the shell's lists them.
        assert_eq!(
            native_directories(&zip::tests::file(SHELL_AAB, "base/native.pb"))
                .into_iter()
                .filter(|(path, _)| path == "lib/x86_64")
                .collect::<Vec<_>>(),
            [("lib/x86_64".to_owned(), 5)]
        );
        // A native.pb of the shell's with fields that aren't folders, which
        // stay, and one that can't be read.
        for (native, error) in [
            (
                protobuf::encode(&[Field::varint(2, 7), Field::bytes(1, vec![0xff])]),
                None,
            ),
            (
                vec![0xff],
                Some("couldn't read the shell's base/native.pb: it's corrupt or cut short"),
            ),
        ] {
            let mut entries = without_libraries();
            entries.push(Entry::new("base/native.pb", Method::Deflated, &native));
            let result = add_libraries(&mut entries, &AAB_LAYOUT, &libraries, &abis, false);
            match error {
                None => {
                    result.unwrap();
                    let native = zip::tests::named(&entries, "base/native.pb")
                        .contents()
                        .unwrap();
                    assert_eq!(protobuf::decode(&native).unwrap().len(), 4);
                }
                Some(message) => assert_eq!(format!("{:#}", result.unwrap_err()), message),
            }
        }
    }

    /// The shell, with a DEX file of its own, as Gradle names it.
    fn with_dex(shell: &[u8], name: &str) -> Vec<Entry> {
        let mut entries = zip::read(shell).unwrap();
        let dex = made_up("035", &["dev.omnibuild.shell.MainActivity", "kotlin.Unit"]);
        entries.push(Entry::new(name, Method::Deflated, &dex));
        entries
    }

    /// A module with a DEX file of the classes, in the format's `version`,
    /// written in `dir`.
    fn with_classes(dir: &Path, name: &str, version: &str, classes: &[&str]) -> Module {
        let path = dir.join(format!("{name}.dex"));
        fs::write(&path, made_up(version, classes)).unwrap();
        module(
            name,
            Android {
                dex: vec![path],
                ..Android::default()
            },
        )
    }

    #[test]
    fn adds_the_modules_dex_files_after_the_shells() {
        let dir = tempfile::tempdir().unwrap();
        let modules = [
            with_classes(dir.path(), "camera", "035", &["dev.example.Camera"]),
            with_classes(dir.path(), "haptics", "037", &["dev.example.Haptics"]),
            module("plain", Android::default()),
        ];
        for (shell, layout, folder) in [
            (SHELL, &APK_LAYOUT, ""),
            (SHELL_AAB, &AAB_LAYOUT, "base/dex/"),
        ] {
            let mut entries = with_dex(shell, &format!("{folder}classes.dex"));

            add_dex(&mut entries, layout, &android(&modules), 24).unwrap();

            let added: Vec<(String, Method, Vec<u8>)> = entries
                .iter()
                .filter(|entry| entry.name.contains("classes"))
                .map(|entry| (entry.name.clone(), entry.method, entry.contents().unwrap()))
                .collect();
            let shell_dex = made_up("035", &["dev.omnibuild.shell.MainActivity", "kotlin.Unit"]);
            assert_eq!(
                added,
                [
                    ("classes.dex", shell_dex),
                    ("classes2.dex", made_up("035", &["dev.example.Camera"])),
                    ("classes3.dex", made_up("037", &["dev.example.Haptics"])),
                ]
                .map(|(name, contents)| (
                    format!("{folder}{name}"),
                    Method::Deflated,
                    contents
                ))
            );
        }
        // After every DEX file the shell has, and none else.
        let mut entries = with_dex(SHELL, "classes.dex");
        let shell_dex = made_up("035", &["dev.omnibuild.shell.Other"]);
        entries.push(Entry::new("classes2.dex", Method::Stored, &shell_dex));
        entries.push(Entry::new("classes-notes.dex", Method::Stored, b"not code"));
        add_dex(&mut entries, &APK_LAYOUT, &android(&modules[..1]), 24).unwrap();
        let added = entries.last().unwrap();
        assert_eq!(
            (added.name.as_str(), added.method),
            ("classes3.dex", Method::Stored)
        );
        // Without code, nothing changes, whatever Android the app runs on.
        let mut entries = zip::read(SHELL).unwrap();
        add_dex(&mut entries, &APK_LAYOUT, &android(&modules[2..]), 1).unwrap();
        assert_eq!(entries, zip::read(SHELL).unwrap());
    }

    #[test]
    fn says_why_it_cant_add_a_dex_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(format!("{name}.dex")).display().to_string();
        let error = |modules: &[Module], min_sdk: u32| {
            let mut entries = with_dex(SHELL, "classes.dex");
            format!(
                "{:#}",
                add_dex(&mut entries, &APK_LAYOUT, &android(modules), min_sdk).unwrap_err()
            )
        };
        let haptics = with_classes(dir.path(), "haptics", "035", &["dev.example.Haptics"]);

        assert_eq!(
            error(std::slice::from_ref(&haptics), 19),
            "the shell runs on Android before 5.0 (its minSdkVersion is 19), which loads only an app's first DEX file, and haptics 1.0.0 has code of its own: OmniBuild adds native modules to apps for Android 5.0 (API 21) and later"
        );
        assert_eq!(
            error(
                &[with_classes(
                    dir.path(),
                    "newer",
                    "038",
                    &["dev.example.Newer"]
                )],
                24
            ),
            format!(
                "{}'s {} is for Android API 26 and later (DEX 038), and the app runs on API 24 too: build it with d8 --min-api 24",
                "newer 1.0.0",
                path("newer")
            )
        );
        assert_eq!(
            error(
                &[with_classes(
                    dir.path(),
                    "copied",
                    "035",
                    &[
                        "kotlin.Unit",
                        "dev.example.Copied",
                        "dev.omnibuild.shell.MainActivity"
                    ]
                )],
                24
            ),
            format!(
                "copied 1.0.0's {} has 2 classes the app has already, such as kotlin.Unit, which the shell has, and Android loads only one of a class: build the module's DEX from its own classes, with the libraries the shell has on d8's classpath",
                path("copied")
            )
        );
        assert_eq!(
            error(
                &[
                    haptics,
                    with_classes(dir.path(), "twin", "035", &["dev.example.Haptics"])
                ],
                24
            ),
            format!(
                "twin 1.0.0's {} has 1 classes the app has already, such as dev.example.Haptics, which haptics 1.0.0 has, and Android loads only one of a class: build the module's DEX from its own classes, with the libraries the shell has on d8's classpath",
                path("twin")
            )
        );
    }

    #[test]
    fn says_why_it_cant_read_a_dex_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(format!("{name}.dex")).display().to_string();
        let error = |modules: &[Module], min_sdk: u32| {
            let mut entries = with_dex(SHELL, "classes.dex");
            format!(
                "{:#}",
                add_dex(&mut entries, &APK_LAYOUT, &android(modules), min_sdk).unwrap_err()
            )
        };
        let gone = module(
            "gone",
            Android {
                dex: vec![dir.path().join("gone.dex")],
                ..Android::default()
            },
        );
        assert!(
            error(&[gone], 24)
                .starts_with(&format!("couldn't read gone 1.0.0's {}: ", path("gone")))
        );
        fs::write(dir.path().join("text.dex"), "not code").unwrap();
        let text = module(
            "text",
            Android {
                dex: vec![dir.path().join("text.dex")],
                ..Android::default()
            },
        );
        assert_eq!(
            error(&[text], 24),
            format!(
                "couldn't read text 1.0.0's {}: it isn't a DEX file",
                path("text")
            )
        );
        let mut entries = zip::read(SHELL).unwrap();
        entries.push(Entry::new("classes.dex", Method::Deflated, b"not code"));
        assert_eq!(
            format!(
                "{:#}",
                add_dex(
                    &mut entries,
                    &APK_LAYOUT,
                    &android(&[with_classes(
                        dir.path(),
                        "haptics",
                        "035",
                        &["dev.example.Haptics"]
                    )]),
                    24
                )
                .unwrap_err()
            ),
            "couldn't read the shell's classes.dex: it isn't a DEX file"
        );
    }
}
