//! Repackaging iOS app shells, which are zipped: a `.app` is a folder, and
//! its executable's mode wouldn't survive being copied around as one.
//!
//! Only builds for the iOS Simulator, so far. Changing an app's files
//! breaks the code signature Xcode gave it, which the Simulator doesn't
//! check but a device does, and OmniBuild can't sign iOS apps yet.

mod info_plist;

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::App;
use crate::payload;
use crate::zip::{self, Entry, Method};
use info_plist::InfoPlist;

/// The folder in the shell's entries that holds the app, if it's an iOS
/// app: `Name.app/`, in a zipped `.app`, or `Payload/Name.app/`, in an
/// `.ipa`. It's the `.app` with an Info.plist, and not one inside another
/// (a watch app, say).
pub fn bundle(entries: &[Entry]) -> Option<String> {
    entries.iter().find_map(|entry| {
        let folder = entry.name.strip_suffix("Info.plist")?;
        let app = folder.strip_prefix("Payload/").unwrap_or(folder);
        (app.ends_with(".app/") && app.matches('/').count() == 1).then(|| folder.to_owned())
    })
}

/// Repackages an iOS app shell, given its entries and the folder in them
/// that holds the app, into the app `app` describes.
pub fn repackage(mut entries: Vec<Entry>, bundle: &str, app: &App) -> Result<Vec<u8>> {
    let name = format!("{bundle}Info.plist");
    let index = entries
        .iter()
        .position(|entry| entry.name == name)
        .expect("the bundle is where an Info.plist is");
    let mut info = entries[index]
        .contents()
        .and_then(|contents| InfoPlist::parse(&contents))
        .with_context(|| format!("couldn't read the shell's {name}"))?;
    let platforms = info.platforms();
    if !platforms.contains(&"iPhoneSimulator") {
        bail!(
            "the shell isn't built for the iOS Simulator: its CFBundleSupportedPlatforms is [{}]. OmniBuild can't sign iOS apps yet, so it only repackages Simulator builds, which run without a valid signature",
            platforms.join(", ")
        );
    }
    let shell = info.clone();
    info.rewrite(app)
        .with_context(|| format!("couldn't rewrite the shell's {name}"))?;
    // Left as it was when nothing changed, rather than written again.
    if info != shell {
        entries[index] = entries[index].with_contents(&info.to_bytes());
    }
    if let Some(payload) = &app.payload {
        add_payload(&mut entries, bundle, payload)?;
    }
    Ok(zip::write(&entries).bytes)
}

/// Adds the payload's files to the app's bundle, replacing the shell's own
/// files of the same name. The app finds them as resources of its main
/// bundle, as it would files Xcode copied there.
fn add_payload(entries: &mut Vec<Entry>, bundle: &str, folder: &Path) -> Result<()> {
    for (name, contents) in payload::read(folder)? {
        let name = format!("{bundle}{name}");
        match entries.iter().position(|entry| entry.name == name) {
            Some(index) => entries[index] = entries[index].with_contents(&contents),
            None => entries.push(Entry::new(&name, Method::Deflated, &contents)),
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::io::Cursor;

    use plist::Value;

    use super::{InfoPlist, bundle, repackage};
    use crate::config::App;
    use crate::zip::{self, Entry, Method, tests::SHELL, tests::entry};

    /// The iOS E2E shell, built for the Simulator: see
    /// tests/fixtures/README.md.
    pub const SHELL_APP: &[u8] = include_bytes!("../../tests/fixtures/shell.app.zip");
    const INFO_PLIST: &str = "OmniBuildShell.app/Info.plist";

    fn file<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        entries.iter().find(|entry| entry.name == name).unwrap()
    }

    /// The shell's Info.plist, as Xcode wrote it.
    pub fn shell_info_plist() -> Vec<u8> {
        file(&zip::read(SHELL_APP).unwrap(), INFO_PLIST)
            .contents()
            .unwrap()
    }

    /// The shell with its Info.plist replaced.
    fn with_info_plist(info: &Entry) -> Vec<Entry> {
        zip::read(SHELL_APP)
            .unwrap()
            .into_iter()
            .map(|entry| {
                if entry.name == INFO_PLIST {
                    info.clone()
                } else {
                    entry
                }
            })
            .collect()
    }

    /// The file's Unix mode, from its zip attributes.
    fn mode(entry: &Entry) -> u32 {
        entry.attributes >> 16
    }

    #[test]
    fn finds_the_app_in_a_zipped_app_or_an_ipa() {
        let app = zip::read(SHELL_APP).unwrap();
        let ipa: Vec<Entry> = app
            .iter()
            .map(|entry| Entry {
                name: format!("Payload/{}", entry.name),
                ..entry.clone()
            })
            .collect();
        let nested = [
            entry("Watch.app/Info.plist", Method::Stored, b""),
            entry("Payload/Info.plist", Method::Stored, b""),
            entry(
                "Shell.app/PlugIns/Share.appex/Info.plist",
                Method::Stored,
                b"",
            ),
            entry("Shell.app/Watch/Watch.app/Info.plist", Method::Stored, b""),
        ];

        assert_eq!(bundle(&app).unwrap(), "OmniBuildShell.app/");
        assert_eq!(bundle(&ipa).unwrap(), "Payload/OmniBuildShell.app/");
        assert_eq!(bundle(&nested).unwrap(), "Watch.app/");
        assert_eq!(bundle(&nested[1..]), None);
        assert_eq!(bundle(&zip::read(SHELL).unwrap()), None);
    }

    #[test]
    fn leaves_the_shell_as_it_was_without_changes() {
        let shell = zip::read(SHELL_APP).unwrap();

        let app = repackage(shell.clone(), "OmniBuildShell.app/", &App::default()).unwrap();

        let app = zip::read(&app).unwrap();
        assert_eq!(app, shell);
        // ditto's modes, which the executable needs to run.
        assert_eq!(
            mode(file(&app, "OmniBuildShell.app/OmniBuildShell")),
            0o100_755
        );
        assert_eq!(mode(file(&app, "OmniBuildShell.app/")), 0o40_755);
    }

    #[test]
    fn gives_the_app_its_values_and_payload() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("bundle")).unwrap();
        fs::write(dir.path().join("payload.txt"), "the app's payload").unwrap();
        fs::write(dir.path().join("bundle/main.lynx.bundle"), "bundle").unwrap();
        let app = App {
            id: Some("dev.example.app".to_owned()),
            name: Some("Repackaged".to_owned()),
            version: Some("2.5.0".to_owned()),
            build: Some(42),
            payload: Some(dir.path().to_owned()),
            ..App::default()
        };
        let shell = zip::read(SHELL_APP).unwrap();

        let repackaged = repackage(shell.clone(), "OmniBuildShell.app/", &app).unwrap();

        let entries = zip::read(&repackaged).unwrap();
        let mut expected = InfoPlist::parse(&shell_info_plist()).unwrap();
        expected.rewrite(&app).unwrap();
        let info = file(&entries, INFO_PLIST);
        assert_eq!(
            InfoPlist::parse(&info.contents().unwrap()).unwrap(),
            expected
        );
        assert_eq!(mode(info), 0o100_644);
        let payload = file(&entries, "OmniBuildShell.app/payload.txt");
        assert_eq!(payload.contents().unwrap(), b"the app's payload");
        let bundle = file(&entries, "OmniBuildShell.app/bundle/main.lynx.bundle");
        assert_eq!(bundle.contents().unwrap(), b"bundle");
        assert_eq!(mode(bundle), 0o100_644);
        // Every other file is as it was.
        let names = |entries: &[Entry]| -> Vec<String> {
            entries.iter().map(|entry| entry.name.clone()).collect()
        };
        assert_eq!(names(&entries)[..7], names(&shell));
        assert_eq!(
            file(&entries, "OmniBuildShell.app/OmniBuildShell"),
            file(&shell, "OmniBuildShell.app/OmniBuildShell")
        );
    }

    #[test]
    fn refuses_a_build_for_devices() {
        let mut info = Value::from_reader(Cursor::new(shell_info_plist())).unwrap();
        info.as_dictionary_mut().unwrap().insert(
            "CFBundleSupportedPlatforms".to_owned(),
            Value::Array(vec![Value::String("iPhoneOS".to_owned())]),
        );
        let mut device = Vec::new();
        info.to_writer_binary(&mut device).unwrap();
        let shell = with_info_plist(&Entry::new(INFO_PLIST, Method::Deflated, &device));

        let error = repackage(shell, "OmniBuildShell.app/", &App::default()).unwrap_err();

        assert!(
            error.to_string().starts_with(
                "the shell isn't built for the iOS Simulator: its CFBundleSupportedPlatforms is [iPhoneOS]. "
            ),
            "{error}"
        );
    }

    #[test]
    fn reports_an_app_it_cant_repackage() {
        let mut corrupt = Entry::new(INFO_PLIST, Method::Deflated, b"");
        corrupt.data = vec![0xff; 8];
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (
                with_info_plist(&Entry::new(INFO_PLIST, Method::Deflated, b"not a plist")),
                App::default(),
                "couldn't read the shell's OmniBuildShell.app/Info.plist: it isn't a property list",
            ),
            (
                with_info_plist(&corrupt),
                App::default(),
                "couldn't read the shell's OmniBuildShell.app/Info.plist: OmniBuildShell.app/Info.plist is corrupt: corrupt deflate stream",
            ),
            (
                zip::read(SHELL_APP).unwrap(),
                App {
                    id: Some("dev.my_app".to_owned()),
                    ..App::default()
                },
                "couldn't rewrite the shell's OmniBuildShell.app/Info.plist: \"dev.my_app\" isn't a valid iOS bundle ID: it can hold only letters, digits, hyphens and dots",
            ),
        ];

        for (shell, app, message) in cases {
            let error = repackage(shell, "OmniBuildShell.app/", &app).unwrap_err();

            // The first ends with the plist parser's own description.
            assert!(format!("{error:#}").starts_with(message), "{error:#}");
        }
        let missing_payload = App {
            payload: Some(dir.path().join("missing")),
            ..App::default()
        };
        let error = repackage(
            zip::read(SHELL_APP).unwrap(),
            "OmniBuildShell.app/",
            &missing_payload,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("couldn't read the payload folder ")
        );
    }
}
