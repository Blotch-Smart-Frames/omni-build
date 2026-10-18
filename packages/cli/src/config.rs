//! The app config: the values that turn a shell into an app.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// What the config gives is changed in the shell. What it leaves out stays
/// as the shell has it.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct App {
    /// The application ID on Android, the bundle ID on iOS.
    pub id: Option<String>,
    /// The name under the app's icon.
    pub name: Option<String>,
    /// The version people see: `versionName` on Android,
    /// `CFBundleShortVersionString` on iOS.
    pub version: Option<String>,
    /// The build number stores compare: `versionCode` on Android,
    /// `CFBundleVersion` on iOS.
    pub build: Option<u32>,
    /// The app's icon: a square PNG, which OmniBuild resizes to each size
    /// the shell has its icon in. Android only, so far: an iOS app keeps
    /// the shell's.
    pub icon: Option<PathBuf>,
    /// A folder of files the app reads at run time, such as its Lynx
    /// bundle: they go in the APK's `assets/`, or in the iOS app's bundle,
    /// replacing the shell's files of the same name.
    pub payload: Option<PathBuf>,
}

impl App {
    /// Reads an app config from a JSON file. The paths in it are relative
    /// to the file's folder.
    pub fn read(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path)
            .with_context(|| format!("couldn't read the app config {}", path.display()))?;
        let app: Self = serde_json::from_str(&json)
            .with_context(|| format!("the app config {} isn't valid", path.display()))?;
        let folder = path.parent().unwrap_or(path);
        Ok(Self {
            icon: app.icon.map(|icon| folder.join(icon)),
            payload: app.payload.map(|payload| folder.join(payload)),
            ..app
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::App;

    #[test]
    fn reads_an_app_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(
            &path,
            r#"{ "id": "dev.example.app", "name": "Example", "version": "2.5.0", "build": 42,
                "icon": "icon.png", "payload": "bundle" }"#,
        )
        .unwrap();

        assert_eq!(
            App::read(&path).unwrap(),
            App {
                id: Some("dev.example.app".to_owned()),
                name: Some("Example".to_owned()),
                version: Some("2.5.0".to_owned()),
                build: Some(42),
                // Relative to the config's folder.
                icon: Some(dir.path().join("icon.png")),
                payload: Some(dir.path().join("bundle")),
            }
        );
    }

    #[test]
    fn reports_a_config_it_cant_read_or_understand() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(&path, r#"{ "id": "dev.example.app", "colour": "blue" }"#).unwrap();

        let missing = App::read(&dir.path().join("missing.json")).unwrap_err();
        let unknown_field = App::read(&path).unwrap_err();

        assert!(
            missing
                .to_string()
                .starts_with("couldn't read the app config ")
        );
        assert!(format!("{unknown_field:#}").contains("unknown field `colour`"));
    }
}
