//! Adding native modules to an iOS app (see `crate::modules`). A module's
//! frameworks come in `.xcframework` bundles, with a framework for each
//! platform. The one for the app's, devices or the Simulator, goes in the
//! app's `Frameworks/`, where the signer signs it as it signs the shell's
//! own, and the app's Info.plist lists the frameworks and the modules for
//! the shell. Nothing compiled the
//! shell's executable against them, so nothing loads them for it: the
//! shell loads each framework when it starts (`Bundle.load()`), then
//! registers each module by name (`NSClassFromString`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

use super::info_plist::InfoPlist;
use super::macho::{self, MH_DYLIB};
use crate::modules::{Ios, Module, check_runtime};
use crate::payload;
use crate::zip::{Entry, Method};

/// The runtime a shell that registers native modules names in its
/// Info.plist, which each module has to be built for.
const RUNTIME: &str = "OmniBuildRuntime";
/// What the app's Info.plist lists for the shell: the modules' frameworks,
/// by their folders' names in `Frameworks/`, and the modules, by name.
const MODULE_FRAMEWORKS: &str = "OmniBuildModuleFrameworks";
const MODULES: &str = "OmniBuildModules";
/// What Xcode leaves out of a framework it embeds in an app: what code
/// compiled against it reads, and its signature, which signing replaces.
const NOT_EMBEDDED: [&str; 4] = ["Headers/", "Modules/", "PrivateHeaders/", "_CodeSignature/"];
/// A framework's executable's mode, as Xcode leaves it.
const EXECUTABLE_MODE: u32 = 0o100_755;

/// Adds the modules that have something for iOS to the app in `bundle`, a
/// build for the Simulator if `simulator`, or for devices, whose Info.plist
/// is `info`.
pub fn add(
    entries: &mut Vec<Entry>,
    bundle: &str,
    info: &mut InfoPlist,
    modules: &[Module],
    simulator: bool,
) -> Result<()> {
    let modules: Vec<(&Module, &Ios)> = modules
        .iter()
        .filter_map(|module| Some((module, module.ios.as_ref()?)))
        .collect();
    if modules.is_empty() {
        return Ok(());
    }
    check_runtime(
        modules.iter().map(|&(module, _)| module),
        info.string(RUNTIME),
        &format!("its Info.plist, as {RUNTIME}"),
    )?;
    // The architectures the app runs on, which each framework needs code
    // for. One that can't be read is the signer's to report.
    let program = info.executable().map(|name| format!("{bundle}{name}"));
    let cpus: Vec<u32> = entries
        .iter()
        .find(|entry| Some(&entry.name) == program.as_ref())
        .and_then(|entry| entry.contents().ok())
        .and_then(|code| macho::architectures(&code).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(cpu, _)| cpu)
        .collect();
    let app = App {
        bundle,
        simulator,
        cpus: &cpus,
        minimum: info.string("MinimumOSVersion").map(str::to_owned),
    };
    let (mut frameworks, mut names) = (Vec::new(), Vec::new());
    for (module, ios) in modules {
        for xcframework in &ios.frameworks {
            frameworks.push(
                app.add_framework(entries, xcframework).with_context(|| {
                    format!("couldn't add {module}'s {}", xcframework.display())
                })?,
            );
        }
        names.extend(ios.modules.iter().cloned());
    }
    info.append(MODULE_FRAMEWORKS, &frameworks);
    info.append(MODULES, &names);
    Ok(())
}

/// What a module's framework has to fit.
struct App<'a> {
    bundle: &'a str,
    simulator: bool,
    /// The CPU types of the app's executable's architectures.
    cpus: &'a [u32],
    /// The oldest iOS the app runs on, `MinimumOSVersion`.
    minimum: Option<String>,
}

impl App<'_> {
    fn platform(&self) -> &'static str {
        if self.simulator {
            "the iOS Simulator"
        } else {
            "iOS devices"
        }
    }

    /// Adds the framework in `xcframework` for the app's platform to the
    /// app's `Frameworks/`, and gives its folder's name there.
    fn add_framework(&self, entries: &mut Vec<Entry>, xcframework: &Path) -> Result<String> {
        let folder = self.library(xcframework)?;
        let name = folder
            .file_name()
            .expect("the library's path ends with its folder")
            .to_string_lossy()
            .into_owned();
        let prefix = format!("{}Frameworks/{name}/", self.bundle);
        if entries.iter().any(|entry| entry.name.starts_with(&prefix)) {
            bail!("the app has a {name} already, from the shell or another module");
        }
        let files: Vec<(String, Vec<u8>)> = payload::read_folder(&folder, "the framework")?
            .into_iter()
            .filter(|(file, _)| !NOT_EMBEDDED.iter().any(|skip| file.starts_with(skip)))
            .collect();
        let contents = |wanted: &str| {
            files
                .iter()
                .find(|(file, _)| file == wanted)
                .map(|(_, contents)| contents)
        };
        let info = contents("Info.plist")
            .with_context(|| format!("{name} has no Info.plist"))
            .and_then(|contents| InfoPlist::parse(contents))
            .with_context(|| format!("couldn't read {name}'s Info.plist"))?;
        let executable = info.executable().with_context(|| {
            format!("{name}'s Info.plist names no executable (CFBundleExecutable)")
        })?;
        let code = contents(executable).with_context(|| {
            format!("{name} has no {executable}, the executable its Info.plist names")
        })?;
        let architectures = macho::architectures(code)
            .ok()
            .filter(|architectures| architectures.iter().all(|&(_, kind)| kind == MH_DYLIB))
            .with_context(|| {
                format!(
                    "{name}/{executable} isn't a dynamic library: build the module as a dynamic framework (MACH_O_TYPE = mh_dylib), which an app loads as it runs, where a static library needs a linker"
                )
            })?;
        let has = || {
            architectures
                .iter()
                .map(|&(cpu, _)| macho::cpu_name(cpu))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if let Some(&cpu) = self
            .cpus
            .iter()
            .find(|&&cpu| !architectures.iter().any(|&(has, _)| has == cpu))
        {
            bail!(
                "{name} has no {} code for {}, which the app has: it has {}",
                macho::cpu_name(cpu),
                self.platform(),
                has()
            );
        }
        if let (Some(needs), Some(runs)) = (info.string("MinimumOSVersion"), &self.minimum)
            && later(needs, runs)
        {
            bail!(
                "{name} needs iOS {needs} or later, and the app runs on iOS {runs} too: build it for iOS {runs} (IPHONEOS_DEPLOYMENT_TARGET)"
            );
        }
        for (file, contents) in &files {
            let mut entry = Entry::new(&format!("{prefix}{file}"), Method::Deflated, contents);
            if file == executable {
                entry.attributes = EXECUTABLE_MODE << 16;
            }
            entries.push(entry);
        }
        Ok(name)
    }

    /// The folder of the framework in an `.xcframework` for the app's
    /// platform, which its Info.plist lists among its libraries.
    fn library(&self, xcframework: &Path) -> Result<PathBuf> {
        let path = xcframework.join("Info.plist");
        let info =
            Value::from_file(&path).with_context(|| format!("couldn't read {}", path.display()))?;
        let libraries = info
            .as_dictionary()
            .and_then(|info| info.get("AvailableLibraries"))
            .and_then(Value::as_array)
            .context("it isn't an XCFramework: its Info.plist lists no AvailableLibraries")?;
        let variant = self.simulator.then_some("simulator");
        let library = libraries
            .iter()
            .filter_map(Value::as_dictionary)
            .find(|library| {
                string(library, "SupportedPlatform") == Some("ios")
                    && string(library, "SupportedPlatformVariant") == variant
            })
            .with_context(|| format!("it has no library for {}", self.platform()))?;
        let (Some(identifier), Some(path)) = (
            string(library, "LibraryIdentifier"),
            string(library, "LibraryPath"),
        ) else {
            bail!(
                "its Info.plist doesn't say where its library for {} is (LibraryIdentifier, LibraryPath)",
                self.platform()
            );
        };
        if !path.ends_with(".framework") {
            bail!(
                "its library for {} is {path}, which isn't a framework: build the module as a dynamic framework (MACH_O_TYPE = mh_dylib), which an app loads as it runs, where a static library needs a linker",
                self.platform()
            );
        }
        Ok(xcframework.join(identifier).join(path))
    }
}

/// The string at `key` in an `.xcframework`'s description of a library.
fn string<'a>(library: &'a Dictionary, key: &str) -> Option<&'a str> {
    library.get(key).and_then(Value::as_string)
}

/// Whether the version `a` is later than `b`, each numbers between dots,
/// however many (`17`, `17.0`, `17.0.1`).
fn later(a: &str, b: &str) -> bool {
    let numbers = |version: &str| {
        let mut numbers: Vec<u32> = version
            .split('.')
            .map(|number| number.parse().unwrap_or(0))
            .collect();
        while numbers.last() == Some(&0) {
            numbers.pop();
        }
        numbers
    };
    numbers(a) > numbers(b)
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use plist::{Dictionary, Value};

    use super::{InfoPlist, add, later};
    use crate::ios::tests::{SHELL_APP, SHELL_IPA};
    use crate::modules::Ios;
    use crate::modules::tests::example;
    use crate::zip::{self, Entry};

    /// A property list, as XML.
    fn plist(value: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes).unwrap();
        bytes
    }

    /// The libraries of an `.xcframework`'s Info.plist: their identifier,
    /// platform variant and path.
    pub fn libraries(libraries: &[(&str, Option<&str>, &str)]) -> Vec<u8> {
        let libraries = libraries
            .iter()
            .map(|&(identifier, variant, path)| {
                let mut library = Dictionary::new();
                library.insert("LibraryIdentifier".to_owned(), identifier.into());
                library.insert("LibraryPath".to_owned(), path.into());
                library.insert("SupportedPlatform".to_owned(), "ios".into());
                if let Some(variant) = variant {
                    library.insert("SupportedPlatformVariant".to_owned(), variant.into());
                }
                Value::Dictionary(library)
            })
            .collect();
        let mut info = Dictionary::new();
        info.insert("AvailableLibraries".to_owned(), Value::Array(libraries));
        info.insert("CFBundlePackageType".to_owned(), "XFWK".into());
        info.insert("XCFrameworkFormatVersion".to_owned(), "1.0".into());
        plist(&Value::Dictionary(info))
    }

    /// An `.xcframework` in `dir`, `<name>.xcframework`, made of the
    /// shells' own framework, `ShellKit`, for devices and the Simulator, renamed
    /// `name`, with a header Xcode doesn't embed.
    pub fn xcframework(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.xcframework"));
        for (shell, folder, identifier) in [
            (SHELL_IPA, "Payload/OmniBuildShell.app", "ios-arm64"),
            (
                SHELL_APP,
                "OmniBuildShell.app",
                "ios-arm64_x86_64-simulator",
            ),
        ] {
            let prefix = format!("{folder}/Frameworks/ShellKit.framework/");
            let framework = path.join(identifier).join(format!("{name}.framework"));
            for entry in zip::read(shell).unwrap() {
                let Some(file) = entry
                    .name
                    .strip_prefix(&prefix)
                    .filter(|file| !file.is_empty() && !file.ends_with('/'))
                else {
                    continue;
                };
                let mut contents = entry.contents().unwrap();
                let file = match file {
                    "ShellKit" => name.to_owned(),
                    "Info.plist" => {
                        let mut info = Value::from_reader(Cursor::new(contents)).unwrap();
                        let info_dictionary = info.as_dictionary_mut().unwrap();
                        info_dictionary.insert("CFBundleExecutable".to_owned(), name.into());
                        info_dictionary.insert(
                            "CFBundleIdentifier".to_owned(),
                            format!("dev.example.{name}").into(),
                        );
                        contents = plist(&info);
                        file.to_owned()
                    }
                    _ => file.to_owned(),
                };
                fs::create_dir_all(framework.join(&file).parent().unwrap()).unwrap();
                fs::write(framework.join(&file), contents).unwrap();
            }
            fs::create_dir_all(framework.join("Headers")).unwrap();
            fs::write(
                framework.join(format!("Headers/{name}.h")),
                "@import Foundation;",
            )
            .unwrap();
        }
        fs::write(
            path.join("Info.plist"),
            libraries(&[
                ("ios-arm64", None, &format!("{name}.framework")),
                (
                    "ios-arm64_x86_64-simulator",
                    Some("simulator"),
                    &format!("{name}.framework"),
                ),
            ]),
        )
        .unwrap();
        path
    }

    /// A shell's entries, the folder of its app, and its Info.plist, with
    /// the runtime `example@1` and a module of its own.
    fn shell(archive: &[u8]) -> (Vec<Entry>, String, InfoPlist) {
        let entries = zip::read(archive).unwrap();
        let folder = crate::ios::bundle(&entries).unwrap();
        let info = entries
            .iter()
            .find(|entry| entry.name == format!("{folder}Info.plist"))
            .unwrap();
        let mut value = Value::from_reader(Cursor::new(info.contents().unwrap())).unwrap();
        let dictionary = value.as_dictionary_mut().unwrap();
        dictionary.insert("OmniBuildRuntime".to_owned(), "example@1".into());
        dictionary.insert(
            "OmniBuildModules".to_owned(),
            Value::Array(vec!["ShellModule".into()]),
        );
        (entries, folder, InfoPlist::parse(&plist(&value)).unwrap())
    }

    /// A module, haptics 1.0.0, for the runtime `example@1`, with what it
    /// adds to an iOS app.
    pub fn module(frameworks: Vec<PathBuf>, modules: &[&str]) -> crate::modules::Module {
        crate::modules::Module {
            ios: Some(Ios {
                frameworks,
                modules: modules.iter().map(|&module| module.to_owned()).collect(),
            }),
            ..example("haptics", "example@1")
        }
    }

    fn strings(info: &InfoPlist, key: &str) -> Vec<String> {
        let value = Value::from_reader(Cursor::new(info.to_bytes())).unwrap();
        value.as_dictionary().unwrap()[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_string().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn adds_each_frameworks_library_for_the_apps_platform() {
        let dir = tempfile::tempdir().unwrap();
        let modules = [
            module(
                vec![
                    xcframework(dir.path(), "Haptics"),
                    xcframework(dir.path(), "HapticsKit"),
                ],
                &["Haptics", "Haptics.Feedback"],
            ),
            crate::modules::Module {
                ios: None,
                ..example("android-only", "example@2")
            },
        ];
        for (simulator, archive) in [(true, SHELL_APP), (false, SHELL_IPA)] {
            let (mut entries, bundle, mut info) = shell(archive);
            let shell_entries = entries.len();

            add(&mut entries, &bundle, &mut info, &modules, simulator).unwrap();

            let added: Vec<(String, u32)> = entries[shell_entries..]
                .iter()
                .map(|entry| (entry.name.clone(), entry.attributes >> 16))
                .collect();
            assert_eq!(
                added,
                [
                    ("Haptics.framework/Haptics", 0o100_755),
                    ("Haptics.framework/Info.plist", 0o100_644),
                    ("HapticsKit.framework/HapticsKit", 0o100_755),
                    ("HapticsKit.framework/Info.plist", 0o100_644),
                ]
                .map(|(name, mode)| (format!("{bundle}Frameworks/{name}"), mode))
            );
            // The library for the app's platform: the Simulator's has
            // x86_64 code too.
            let shell_kit = zip::read(archive)
                .unwrap()
                .into_iter()
                .find(|entry| {
                    entry.name == format!("{bundle}Frameworks/ShellKit.framework/ShellKit")
                })
                .unwrap();
            assert_eq!(
                entries[shell_entries].contents().unwrap(),
                shell_kit.contents().unwrap()
            );
            assert_eq!(
                strings(&info, "OmniBuildModuleFrameworks"),
                ["Haptics.framework", "HapticsKit.framework"]
            );
            // After the shell's own.
            assert_eq!(
                strings(&info, "OmniBuildModules"),
                ["ShellModule", "Haptics", "Haptics.Feedback"]
            );
        }
    }

    #[test]
    fn leaves_the_app_as_it_was_without_modules_for_ios() {
        let (mut entries, bundle, mut info) = shell(SHELL_APP);
        let (shell_entries, shell_info) = (entries.clone(), info.clone());
        let modules = [crate::modules::Module {
            ios: None,
            ..example("android-only", "example@2")
        }];

        add(&mut entries, &bundle, &mut info, &modules, true).unwrap();

        assert_eq!((entries, info), (shell_entries, shell_info));
    }

    /// The error from adding the `.xcframework` at `path`, changed by `change`,
    /// to the Simulator shell, as haptics 1.0.0 has it.
    fn error_for(dir: &Path, change: impl FnOnce(&Path)) -> String {
        let path = xcframework(dir, "Haptics");
        change(&path);
        let (mut entries, bundle, mut info) = shell(SHELL_APP);
        let error = add(
            &mut entries,
            &bundle,
            &mut info,
            &[module(vec![path], &[])],
            true,
        )
        .unwrap_err();
        format!("{error:#}")
    }

    /// A file in the Simulator's framework, changed by `change`.
    fn changing(file: &'static str, change: impl Fn(&mut Vec<u8>) + 'static) -> impl FnOnce(&Path) {
        move |path: &Path| {
            let path = path
                .join("ios-arm64_x86_64-simulator/Haptics.framework")
                .join(file);
            let mut contents = fs::read(&path).unwrap();
            change(&mut contents);
            fs::write(path, contents).unwrap();
        }
    }

    /// The framework's Info.plist with `change` made to it.
    fn framework_info(change: impl Fn(&mut Dictionary) + 'static) -> impl FnOnce(&Path) {
        changing("Info.plist", move |contents| {
            let mut info = Value::from_reader(Cursor::new(&contents[..])).unwrap();
            change(info.as_dictionary_mut().unwrap());
            *contents = plist(&info);
        })
    }

    /// Where a universal file has the code of its architecture `index`.
    fn slice(file: &[u8], index: usize) -> usize {
        let at = 8 + 20 * index + 8;
        u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize
    }

    /// A change to an `.xcframework`, at the path it's given.
    type Change = Box<dyn FnOnce(&Path)>;

    /// Checks that adding the `.xcframework`, with each change, gives its
    /// error: `couldn't`, with the message for the error at the path.
    fn check_errors(cases: impl FnOnce(&Path, &dyn Fn(&str) -> String) -> Vec<(Change, String)>) {
        let dir = tempfile::tempdir().unwrap();
        let xcframework = dir.path().join("Haptics.xcframework");
        let couldnt = |message: &str| {
            format!(
                "couldn't add haptics 1.0.0's {}: {message}",
                xcframework.display()
            )
        };
        for (change, message) in cases(&xcframework, &couldnt) {
            let error = error_for(dir.path(), change);
            assert!(error.starts_with(&message), "{error}\n{message}");
            fs::remove_dir_all(dir.path().join("Haptics.xcframework")).unwrap();
        }
    }

    #[test]
    fn says_why_it_cant_find_the_framework_for_the_app() {
        check_errors(|xcframework, couldnt| {
            let info_path = xcframework.join("Info.plist");
            let write_info = |contents: Vec<u8>| {
                move |path: &Path| fs::write(path.join("Info.plist"), contents).unwrap()
            };
            vec![
                (
                    Box::new(|path: &Path| fs::remove_file(path.join("Info.plist")).unwrap()),
                    couldnt(&format!("couldn't read {}: ", info_path.display())),
                ),
                (
                    Box::new(write_info(plist(&Value::Dictionary(Dictionary::new())))),
                    couldnt("it isn't an XCFramework: its Info.plist lists no AvailableLibraries"),
                ),
                (
                    Box::new(write_info(libraries(&[(
                        "ios-arm64",
                        None,
                        "Haptics.framework",
                    )]))),
                    couldnt("it has no library for the iOS Simulator"),
                ),
                (
                    Box::new(write_info({
                        let mut info = Dictionary::new();
                        let mut library = Dictionary::new();
                        library.insert("SupportedPlatform".to_owned(), "ios".into());
                        library.insert("SupportedPlatformVariant".to_owned(), "simulator".into());
                        info.insert(
                            "AvailableLibraries".to_owned(),
                            Value::Array(vec![Value::Dictionary(library)]),
                        );
                        plist(&Value::Dictionary(info))
                    })),
                    couldnt(
                        "its Info.plist doesn't say where its library for the iOS Simulator is (LibraryIdentifier, LibraryPath)",
                    ),
                ),
                (
                    Box::new(write_info(libraries(&[(
                        "ios-arm64_x86_64-simulator",
                        Some("simulator"),
                        "libHaptics.a",
                    )]))),
                    couldnt(
                        "its library for the iOS Simulator is libHaptics.a, which isn't a framework: build the module as a dynamic framework (MACH_O_TYPE = mh_dylib), which an app loads as it runs, where a static library needs a linker",
                    ),
                ),
                (
                    Box::new(|path: &Path| {
                        fs::remove_dir_all(path.join("ios-arm64_x86_64-simulator")).unwrap();
                    }),
                    couldnt(&format!(
                        "couldn't read the framework {}: ",
                        xcframework
                            .join("ios-arm64_x86_64-simulator/Haptics.framework")
                            .display()
                    )),
                ),
            ]
        });
    }

    #[test]
    fn says_why_a_framework_doesnt_fit_the_app() {
        let (_, _, info) = shell(SHELL_APP);
        let minimum = info.string("MinimumOSVersion").unwrap().to_owned();
        check_errors(|_, couldnt| {
            vec![
                (
                    Box::new(|path: &Path| {
                        fs::remove_file(
                            path.join("ios-arm64_x86_64-simulator/Haptics.framework/Info.plist"),
                        )
                        .unwrap();
                    }),
                    couldnt(
                        "couldn't read Haptics.framework's Info.plist: Haptics.framework has no Info.plist",
                    ),
                ),
                (
                    Box::new(changing("Info.plist", |contents| {
                        *contents = b"not a plist".to_vec();
                    })),
                    couldnt(
                        "couldn't read Haptics.framework's Info.plist: it isn't a property list: ",
                    ),
                ),
                (
                    Box::new(framework_info(|info| {
                        info.remove("CFBundleExecutable");
                    })),
                    couldnt(
                        "Haptics.framework's Info.plist names no executable (CFBundleExecutable)",
                    ),
                ),
                (
                    Box::new(framework_info(|info| {
                        info.insert("CFBundleExecutable".to_owned(), "Gone".into());
                    })),
                    couldnt("Haptics.framework has no Gone, the executable its Info.plist names"),
                ),
                (
                    Box::new(changing("Haptics", |contents| {
                        *contents = b"!<arch>\n".to_vec();
                    })),
                    couldnt(
                        "Haptics.framework/Haptics isn't a dynamic library: build the module as a dynamic framework (MACH_O_TYPE = mh_dylib), which an app loads as it runs, where a static library needs a linker",
                    ),
                ),
                (
                    // Its first architecture's code made the program's file type.
                    Box::new(changing("Haptics", |contents| {
                        let at = slice(contents, 0) + 12;
                        contents[at] = 2;
                    })),
                    couldnt(
                        "Haptics.framework/Haptics isn't a dynamic library: build the module as a dynamic framework (MACH_O_TYPE = mh_dylib), which an app loads as it runs, where a static library needs a linker",
                    ),
                ),
                (
                    // Its arm64 code, after its x86_64 code, made code for
                    // another CPU.
                    Box::new(changing("Haptics", |contents| {
                        let at = slice(contents, 1) + 4;
                        contents[at..at + 4].copy_from_slice(&0x12_u32.to_le_bytes());
                    })),
                    couldnt(
                        "Haptics.framework has no arm64 code for the iOS Simulator, which the app has: it has x86_64, CPU type 0x12",
                    ),
                ),
                (
                    Box::new(framework_info(|info| {
                        info.insert("MinimumOSVersion".to_owned(), "99.0".into());
                    })),
                    couldnt(&format!(
                        "Haptics.framework needs iOS 99.0 or later, and the app runs on iOS {minimum} too: build it for iOS {minimum} (IPHONEOS_DEPLOYMENT_TARGET)"
                    )),
                ),
            ]
        });
    }

    #[test]
    fn refuses_a_framework_the_app_has_already_or_a_runtime_it_hasnt() {
        let dir = tempfile::tempdir().unwrap();
        let (mut entries, bundle, mut info) = shell(SHELL_APP);
        let shell_kit = module(vec![xcframework(dir.path(), "ShellKit")], &[]);

        let error = add(&mut entries, &bundle, &mut info, &[shell_kit], true).unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            format!(
                "couldn't add haptics 1.0.0's {}: the app has a ShellKit.framework already, from the shell or another module",
                dir.path().join("ShellKit.xcframework").display()
            )
        );
        // A build for devices needs the library for devices.
        let devices = dir.path().join("Devices");
        let simulator_only = xcframework(&devices, "Haptics");
        fs::write(
            simulator_only.join("Info.plist"),
            libraries(&[(
                "ios-arm64_x86_64-simulator",
                Some("simulator"),
                "Haptics.framework",
            )]),
        )
        .unwrap();
        let (mut entries, bundle, mut info) = shell(SHELL_IPA);
        let error = add(
            &mut entries,
            &bundle,
            &mut info,
            &[module(vec![simulator_only], &[])],
            false,
        )
        .unwrap_err();
        assert!(format!("{error:#}").ends_with(": it has no library for iOS devices"));
        let (mut entries, bundle, mut info) = shell(SHELL_APP);
        let other = crate::modules::Module {
            ios: Some(Ios::default()),
            ..example("haptics", "example@2")
        };
        assert_eq!(
            add(&mut entries, &bundle, &mut info, &[other], true)
                .unwrap_err()
                .to_string(),
            "haptics 1.0.0 is built for the runtime example@2, and the shell's is example@1: install a version of it built for example@1"
        );
        let mut entries = zip::read(SHELL_APP).unwrap();
        let info = entries
            .iter()
            .find(|entry| entry.name == format!("{bundle}Info.plist"))
            .unwrap();
        let mut info = InfoPlist::parse(&info.contents().unwrap()).unwrap();
        assert_eq!(
            add(
                &mut entries,
                &bundle,
                &mut info,
                &[example("haptics", "example@1")],
                true
            )
            .unwrap_err()
            .to_string(),
            "haptics 1.0.0 is a native module, and the shell takes none: a shell that registers native modules names its runtime in its Info.plist, as OmniBuildRuntime"
        );
    }

    #[test]
    fn compares_versions_number_by_number() {
        assert!(later("18.0", "17.4"));
        assert!(later("17.10", "17.9"));
        assert!(later("17.0.1", "17"));
        assert!(!later("17.0", "17"));
        assert!(!later("17", "17.0.0"));
        assert!(!later("16.4", "17.0"));
    }
}
