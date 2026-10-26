//! Native modules: npm packages with native code built ahead of time, for a
//! shell's runtime, which OmniBuild adds to an app with no compiler. A
//! prebuilt shell can't compile a module's source, so a module ships its
//! binaries, and says what they are in its `package.json`, under
//! `omniBuild`:
//!
//! ```json
//! "omniBuild": {
//!   "runtime": "example@1",
//!   "ios": { "frameworks": ["ios/Haptics.xcframework"], "modules": ["Haptics"] },
//!   "android": {
//!     "dex": ["android/classes.dex"],
//!     "jniLibs": "android/jni",
//!     "modules": ["dev.example.haptics.Haptics"],
//!     "permissions": ["VIBRATE"]
//!   }
//! }
//! ```
//!
//! The app config's `packageJson` names the app's own, and OmniBuild finds
//! the modules among its dependencies, and theirs, where npm installed
//! them: in `node_modules`, as Node finds a package. The shell registers
//! each module by name when it starts, as its runtime does. A binary works
//! only with the runtime it was built against, which the shell names, so a
//! module built for another is refused rather than crash the app.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

/// What npm installs for a package in `node_modules`, besides the app's
/// own devDependencies: a package's own are for developing it.
const DEPENDENCIES: [&str; 3] = ["dependencies", "optionalDependencies", "peerDependencies"];

#[derive(Debug, PartialEq, Eq)]
pub struct Module {
    /// Its package's name and version.
    pub name: String,
    pub version: String,
    /// The runtime its binaries were built against, as shells name theirs,
    /// such as `example@1`.
    pub runtime: String,
    pub ios: Option<Ios>,
    pub android: Option<Android>,
}

impl fmt::Display for Module {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{} {}", self.name, self.version)
    }
}

/// What a module adds to an iOS app.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ios {
    /// `.xcframework` bundles, each with a dynamic framework for devices
    /// and one for the Simulator, which go in the app's `Frameworks/`.
    #[serde(default)]
    pub frameworks: Vec<PathBuf>,
    /// What the shell registers, by name: for OmniBuild's shells, the
    /// Objective-C names of classes, which `NSClassFromString` finds.
    #[serde(default)]
    pub modules: Vec<String>,
}

/// What a module adds to an Android app.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Android {
    /// Its code, compiled for Android (DEX files, as d8 makes them), which
    /// goes beside the shell's.
    #[serde(default)]
    pub dex: Vec<PathBuf>,
    /// A folder of native libraries, in a folder for each ABI
    /// (`arm64-v8a/libhaptics.so`), as Gradle's `jniLibs` are.
    pub jni_libs: Option<PathBuf>,
    /// What the shell registers, by name: for OmniBuild's shells, the
    /// names of classes, which `Class.forName` finds.
    #[serde(default)]
    pub modules: Vec<String>,
    /// The permissions it asks for, as the app config's
    /// `android.permissions` names them, which the app asks for too.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// A module's `omniBuild`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    runtime: String,
    ios: Option<Ios>,
    android: Option<Android>,
}

/// The native modules among the dependencies of the app's `package.json`,
/// and theirs, as npm installed them, by name.
pub fn find(package_json: &Path) -> Result<Vec<Module>> {
    let path = fs::canonicalize(package_json).with_context(|| {
        format!(
            "couldn't read the app's package.json, {}",
            package_json.display()
        )
    })?;
    let app = read(&path)?;
    let root = path.parent().expect("a file is in a folder");
    // What the app depends on has to be installed, or it would be left out
    // without a word. Its optional dependencies and devDependencies needn't
    // be: npm leaves those out of some installs.
    let mut folders = VecDeque::new();
    for name in dependencies(&app, &["dependencies"]) {
        folders.push_back(resolve(root, name).with_context(|| {
            format!(
                "{name}, which {} depends on, isn't installed in node_modules: run npm install",
                package_json.display()
            )
        })?);
    }
    folders.extend(
        dependencies(&app, &["optionalDependencies", "devDependencies"])
            .filter_map(|name| resolve(root, name)),
    );
    // Breadth first, so that of the copies of one version, the module has
    // the one nearest the app.
    let mut seen = HashSet::new();
    let mut modules = Vec::new();
    while let Some(folder) = folders.pop_front() {
        if !seen.insert(folder.clone()) {
            continue;
        }
        let package = read(&folder.join("package.json"))?;
        folders.extend(
            dependencies(&package, &DEPENDENCIES).filter_map(|name| resolve(&folder, name)),
        );
        if let Some(manifest) = package.get("omniBuild") {
            modules.push(module(&package, manifest, &folder)?);
        }
    }
    modules.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    // npm installs a package once for every version something asks for. One
    // version's copies are the same module, but two versions' binaries
    // would define the same classes.
    modules.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    if let Some(pair) = modules.windows(2).find(|pair| pair[0].name == pair[1].name) {
        bail!(
            "{} is installed at two versions, {} and {}, and an app can only have one of a native module: have npm install one, with an override in package.json",
            pair[0].name,
            pair[0].version,
            pair[1].version
        );
    }
    Ok(modules)
}

/// Checks that each module is built for the shell's runtime, `runtime`,
/// which a shell that registers modules names in `place`.
pub fn check_runtime<'a>(
    modules: impl IntoIterator<Item = &'a Module>,
    runtime: Option<&str>,
    place: &str,
) -> Result<()> {
    for module in modules {
        match runtime {
            None => bail!(
                "{module} is a native module, and the shell takes none: a shell that registers native modules names its runtime in {place}"
            ),
            Some(runtime) if runtime != module.runtime => bail!(
                "{module} is built for the runtime {}, and the shell's is {runtime}: install a version of it built for {runtime}",
                module.runtime
            ),
            Some(_) => {}
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<Value> {
    let json =
        fs::read_to_string(path).with_context(|| format!("couldn't read {}", path.display()))?;
    serde_json::from_str(&json).with_context(|| format!("{} isn't valid JSON", path.display()))
}

/// The names of what a package depends on, of each of `kinds`.
fn dependencies<'a>(package: &'a Value, kinds: &[&str]) -> impl Iterator<Item = &'a String> {
    kinds
        .iter()
        .filter_map(|kind| package.get(kind)?.as_object())
        .flat_map(serde_json::Map::keys)
}

/// The folder of the package `name`, as Node finds it from a package in
/// `folder`: in `node_modules` in the folder, or in the nearest one in a
/// folder above it, but never in `node_modules/node_modules`. Followed
/// through links, as pnpm installs packages, whose own dependencies are in
/// the folder above the one it links to.
fn resolve(folder: &Path, name: &str) -> Option<PathBuf> {
    folder
        .ancestors()
        .filter(|folder| folder.file_name() != Some(OsStr::new("node_modules")))
        .map(|folder| folder.join("node_modules").join(name))
        .find(|package| package.join("package.json").is_file())
        .and_then(|package| fs::canonicalize(package).ok())
}

/// The module a package's `omniBuild`, `manifest`, describes, with its
/// paths in its folder.
fn module(package: &Value, manifest: &Value, folder: &Path) -> Result<Module> {
    let path = folder.join("package.json");
    let field = |name: &str| package.get(name).and_then(Value::as_str).map(str::to_owned);
    let (Some(name), Some(version)) = (field("name"), field("version")) else {
        bail!(
            "{} has an omniBuild, and no name and version, which every package npm installs has",
            path.display()
        );
    };
    let manifest = Manifest::deserialize(manifest)
        .with_context(|| format!("{name}'s omniBuild, in {}, isn't valid", path.display()))?;
    Ok(Module {
        name,
        version,
        runtime: manifest.runtime,
        ios: manifest.ios.map(|ios| Ios {
            frameworks: ios
                .frameworks
                .iter()
                .map(|framework| folder.join(framework))
                .collect(),
            ..ios
        }),
        android: manifest.android.map(|android| Android {
            dex: android.dex.iter().map(|dex| folder.join(dex)).collect(),
            jni_libs: android.jni_libs.map(|libraries| folder.join(libraries)),
            ..android
        }),
    })
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::{Android, Ios, Module, check_runtime, find};

    /// Writes a package's `package.json` in `folder`.
    pub fn package(folder: &Path, json: &Value) {
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join("package.json"), json.to_string()).unwrap();
    }

    /// A module's `package.json`, for the runtime `example@1`.
    pub fn module_json(name: &str, version: &str, dependencies: &Value) -> Value {
        json!({
            "name": name,
            "version": version,
            "dependencies": dependencies,
            "omniBuild": {
                "runtime": "example@1",
                "ios": { "frameworks": ["ios/Haptics.xcframework"], "modules": ["Haptics"] },
                "android": {
                    "dex": ["android/classes.dex"],
                    "jniLibs": "android/jni",
                    "modules": ["dev.example.Haptics"],
                    "permissions": ["VIBRATE"]
                }
            }
        })
    }

    pub fn example(name: &str, runtime: &str) -> Module {
        Module {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            runtime: runtime.to_owned(),
            ios: Some(Ios::default()),
            android: Some(Android::default()),
        }
    }

    fn names(modules: &[Module]) -> Vec<String> {
        modules.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn finds_the_modules_among_the_apps_dependencies_and_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let modules = app.join("node_modules");
        package(
            &app,
            &json!({
                "name": "tides",
                "dependencies": { "@example/haptics": "^1.0.0", "ui-kit": "^2.0.0" },
                "devDependencies": { "camera": "^3.0.0", "uninstalled": "^1.0.0" },
                "optionalDependencies": { "maps": "^1.0.0" }
            }),
        );
        package(
            &modules.join("@example/haptics"),
            &module_json("@example/haptics", "1.0.0", &json!({ "ui-kit": "^2.0.0" })),
        );
        // A library that uses a module, at two versions: the app's, and an
        // older one of its own for the camera. Each depends on a module.
        package(
            &modules.join("ui-kit"),
            &json!({ "name": "ui-kit", "version": "2.0.0", "dependencies": { "@example/haptics": "^1.0.0" } }),
        );
        package(
            &modules.join("camera"),
            &json!({
                "name": "camera", "version": "3.0.0",
                "dependencies": { "ui-kit": "^1.0.0" },
                "peerDependencies": { "absent-peer": "*" },
                "omniBuild": { "runtime": "example@1", "ios": { "modules": ["Camera"] } }
            }),
        );
        package(
            &modules.join("camera/node_modules/ui-kit"),
            &json!({ "name": "ui-kit", "version": "1.0.0", "dependencies": { "@example/haptics": "^1.0.0", "flash": "*" } }),
        );
        // The same version again, where ui-kit 1 has it of its own.
        package(
            &modules.join("camera/node_modules/ui-kit/node_modules/@example/haptics"),
            &module_json("@example/haptics", "1.0.0", &json!({})),
        );
        package(
            &modules.join("flash"),
            &json!({ "name": "flash", "version": "0.1.0", "omniBuild": { "runtime": "example@1" } }),
        );
        package(
            &modules.join("maps"),
            &json!({ "name": "maps", "version": "1.0.0" }),
        );
        // Not a dependency of the app or anything it depends on.
        package(
            &modules.join("unused"),
            &module_json("unused", "1.0.0", &json!({})),
        );

        let found = find(&app.join("package.json")).unwrap();

        assert_eq!(
            names(&found),
            ["@example/haptics 1.0.0", "camera 3.0.0", "flash 0.1.0"]
        );
        let folder = fs::canonicalize(modules.join("@example/haptics")).unwrap();
        assert_eq!(
            found[0],
            Module {
                name: "@example/haptics".to_owned(),
                version: "1.0.0".to_owned(),
                runtime: "example@1".to_owned(),
                ios: Some(Ios {
                    frameworks: vec![folder.join("ios/Haptics.xcframework")],
                    modules: vec!["Haptics".to_owned()],
                }),
                android: Some(Android {
                    dex: vec![folder.join("android/classes.dex")],
                    jni_libs: Some(folder.join("android/jni")),
                    modules: vec!["dev.example.Haptics".to_owned()],
                    permissions: vec!["VIBRATE".to_owned()],
                }),
            }
        );
        assert_eq!(
            found[1].ios.as_ref().unwrap().frameworks,
            Vec::<PathBuf>::new()
        );
        assert_eq!(found[1].android, None);
        assert_eq!((&found[2].ios, &found[2].android), (&None, &None));
    }

    #[cfg(unix)]
    #[test]
    fn finds_modules_through_links_as_pnpm_installs_them() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let store = app.join("node_modules/.pnpm");
        package(&app, &json!({ "dependencies": { "ui-kit": "2.0.0" } }));
        package(
            &store.join("ui-kit@2.0.0/node_modules/ui-kit"),
            &json!({ "name": "ui-kit", "version": "2.0.0", "dependencies": { "haptics": "1.0.0" } }),
        );
        package(
            &store.join("ui-kit@2.0.0/node_modules/haptics"),
            &module_json("haptics", "1.0.0", &json!({})),
        );
        std::os::unix::fs::symlink(
            store.join("ui-kit@2.0.0/node_modules/ui-kit"),
            app.join("node_modules/ui-kit"),
        )
        .unwrap();

        let found = find(&app.join("package.json")).unwrap();

        assert_eq!(names(&found), ["haptics 1.0.0"]);
    }

    #[test]
    fn says_why_it_cant_find_the_modules() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("package.json");
        let display = app.display();
        let modules = dir.path().join("node_modules");
        let installed = |json: &Value| {
            package(dir.path(), &json!({ "dependencies": { "haptics": "*" } }));
            package(&modules.join("haptics"), json);
            find(&app).unwrap_err()
        };
        let messages =
            |error: anyhow::Error| error.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            find(&app).unwrap_err().to_string(),
            format!("couldn't read the app's package.json, {display}")
        );
        // A folder, by the name of the file.
        fs::create_dir(&app).unwrap();
        assert_eq!(
            find(&app).unwrap_err().to_string(),
            format!(
                "couldn't read {}",
                fs::canonicalize(&app).unwrap().display()
            )
        );
        fs::remove_dir(&app).unwrap();
        fs::write(&app, "{").unwrap();
        assert_eq!(
            find(&app).unwrap_err().to_string(),
            format!(
                "{} isn't valid JSON",
                fs::canonicalize(&app).unwrap().display()
            )
        );
        package(dir.path(), &json!({ "dependencies": { "haptics": "*" } }));
        assert_eq!(
            find(&app).unwrap_err().to_string(),
            format!(
                "haptics, which {display} depends on, isn't installed in node_modules: run npm install"
            )
        );
        let folder = || fs::canonicalize(modules.join("haptics")).unwrap();
        package(&modules.join("haptics"), &json!({}));
        fs::write(modules.join("haptics/package.json"), "{").unwrap();
        assert_eq!(
            find(&app).unwrap_err().to_string(),
            format!(
                "{} isn't valid JSON",
                folder().join("package.json").display()
            )
        );
        assert_eq!(
            installed(&json!({ "omniBuild": { "runtime": "example@1" } })).to_string(),
            format!(
                "{} has an omniBuild, and no name and version, which every package npm installs has",
                folder().join("package.json").display()
            )
        );
        assert_eq!(
            messages(installed(
                &json!({ "name": "haptics", "version": "1.0.0", "omniBuild": { "runtime": "example@1", "watchos": {} } })
            )),
            [
                format!(
                    "haptics's omniBuild, in {}, isn't valid",
                    folder().join("package.json").display()
                ),
                "unknown field `watchos`, expected one of `runtime`, `ios`, `android`".to_owned(),
            ]
        );
        package(
            &modules.join("ui-kit"),
            &json!({ "name": "ui-kit", "version": "1.0.0", "dependencies": { "haptics": "2.0.0" } }),
        );
        package(
            &modules.join("ui-kit/node_modules/haptics"),
            &module_json("haptics", "2.0.0", &json!({})),
        );
        package(
            dir.path(),
            &json!({ "dependencies": { "haptics": "*", "ui-kit": "*" } }),
        );
        package(
            &modules.join("haptics"),
            &module_json("haptics", "1.0.0", &json!({})),
        );
        assert_eq!(
            find(&app).unwrap_err().to_string(),
            "haptics is installed at two versions, 1.0.0 and 2.0.0, and an app can only have one of a native module: have npm install one, with an override in package.json"
        );
    }

    #[test]
    fn checks_the_modules_are_built_for_the_shells_runtime() {
        let place = "its Info.plist (OmniBuildRuntime)";
        let modules = [
            example("haptics", "example@1"),
            example("camera", "example@2"),
        ];

        check_runtime(&modules[..1], Some("example@1"), place).unwrap();
        check_runtime([], None, place).unwrap();
        assert_eq!(
            check_runtime(&modules, Some("example@1"), place)
                .unwrap_err()
                .to_string(),
            "camera 1.0.0 is built for the runtime example@2, and the shell's is example@1: install a version of it built for example@1"
        );
        assert_eq!(
            check_runtime(&modules, None, place)
                .unwrap_err()
                .to_string(),
            "haptics 1.0.0 is a native module, and the shell takes none: a shell that registers native modules names its runtime in its Info.plist (OmniBuildRuntime)"
        );
    }
}
