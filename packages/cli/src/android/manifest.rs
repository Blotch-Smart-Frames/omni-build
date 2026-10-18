//! The app's values in a shell's manifest. An APK's manifest is binary
//! XML and an AAB's is XML in protobuf (`binary_xml.rs`, `proto_xml.rs`),
//! but which attributes change, and how, is the same for both.

use anyhow::{Context, Result, bail};

use crate::config::App;

// The android: attributes OmniBuild reads or rewrites, by resource ID, as
// Android itself identifies them (android.R.attr).
const LABEL: u32 = 0x0101_0001;
const ICON: u32 = 0x0101_0002;
const NAME: u32 = 0x0101_0003;
const MANAGE_SPACE_ACTIVITY: u32 = 0x0101_0004;
const PERMISSION: u32 = 0x0101_0006;
const READ_PERMISSION: u32 = 0x0101_0007;
const WRITE_PERMISSION: u32 = 0x0101_0008;
const AUTHORITIES: u32 = 0x0101_0018;
const TARGET_ACTIVITY: u32 = 0x0101_0202;
const MIN_SDK_VERSION: u32 = 0x0101_020c;
const VERSION_CODE: u32 = 0x0101_021b;
const VERSION_NAME: u32 = 0x0101_021c;
const BACKUP_AGENT: u32 = 0x0101_027f;
const PARENT_ACTIVITY_NAME: u32 = 0x0101_03a7;
const ROUND_ICON: u32 = 0x0101_052c;

/// Android 7.0, the first to verify APK Signature Scheme v2. Earlier
/// versions only verify a v1 (JAR) signature, which OmniBuild doesn't write
/// for APKs.
const MIN_SDK: u32 = 24;
/// The highest version code Google Play accepts.
const MAX_VERSION_CODE: u32 = 2_100_000_000;

/// An attribute's value, typed as both formats type it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    String(String),
    Int(u32),
    /// A resource's ID, like `@string/app_name`'s.
    Reference(u32),
    Other,
}

#[derive(Debug)]
pub struct Attribute {
    /// The name of the element it's on.
    pub element: String,
    pub name: String,
    /// The resource ID of an android: attribute. Others, like `package`,
    /// have none.
    pub resource_id: Option<u32>,
    pub value: Value,
}

/// A manifest whose attributes can be changed, in either format.
pub trait Document {
    /// Every attribute, in document order.
    fn attributes(&self) -> &[Attribute];
    fn set_string(&mut self, index: usize, value: &str);
    fn set_int(&mut self, index: usize, value: u32);
    fn to_bytes(&self) -> Vec<u8>;
}

/// Where else the app's values go, besides the manifest.
#[derive(Debug)]
pub struct Changes {
    /// The string resource the app's name goes in, when the shell's label
    /// is one rather than text in the manifest.
    pub name_resource: Option<u32>,
    /// The resources the app's icon goes in: the application's icon, and
    /// its round icon, which some launchers show instead.
    pub icon_resources: Vec<u32>,
}

/// Gives the manifest the app's ID, name, version and build number.
pub fn rewrite(manifest: &mut dyn Document, app: &App) -> Result<Changes> {
    let min_sdk = find(manifest, "uses-sdk", MIN_SDK_VERSION)
        .and_then(|index| match manifest.attributes()[index].value {
            Value::Int(value) => Some(value),
            _ => None,
        })
        .unwrap_or(1);
    if min_sdk < MIN_SDK {
        bail!(
            "the shell runs on Android versions before 7.0 (its minSdkVersion is {min_sdk}), which need a v1 signature. OmniBuild only signs with v2 and v3, for Android 7.0 (API 24) and later"
        );
    }
    if let Some(id) = &app.id {
        set_package(manifest, id)?;
    }
    if let Some(version) = &app.version {
        let index = find(manifest, "manifest", VERSION_NAME)
            .context("it has no android:versionName to change")?;
        manifest.set_string(index, version);
    }
    if let Some(build) = app.build {
        if !(1..=MAX_VERSION_CODE).contains(&build) {
            bail!(
                "the build number {build} isn't between 1 and {MAX_VERSION_CODE}, which Google Play requires of a version code"
            );
        }
        let index = find(manifest, "manifest", VERSION_CODE)
            .context("it has no android:versionCode to change")?;
        manifest.set_int(index, build);
    }
    // The application's label is the name under the icon, unless the
    // launcher activity has a label of its own: shells shouldn't.
    let mut name_resource = None;
    if let Some(name) = &app.name {
        let index =
            find(manifest, "application", LABEL).context("it has no android:label to change")?;
        match manifest.attributes()[index].value {
            Value::Reference(id) => name_resource = Some(id),
            _ => manifest.set_string(index, name),
        }
    }
    let mut icon_resources = Vec::new();
    if app.icon.is_some() {
        icon_resources = [ICON, ROUND_ICON]
            .into_iter()
            .filter_map(|icon| find(manifest, "application", icon))
            .filter_map(|index| match manifest.attributes()[index].value {
                Value::Reference(id) => Some(id),
                _ => None,
            })
            .collect();
        if icon_resources.is_empty() {
            bail!("it has no android:icon to change");
        }
    }
    Ok(Changes {
        name_resource,
        icon_resources,
    })
}

/// The index of the android: attribute with this resource ID on the first
/// element with this name.
fn find(manifest: &dyn Document, element: &str, resource_id: u32) -> Option<usize> {
    manifest.attributes().iter().position(|attribute| {
        attribute.element == element && attribute.resource_id == Some(resource_id)
    })
}

/// Gives the app a new ID: the manifest's `package`, and the values AGP
/// derives from the application ID through `${applicationId}`. Those have
/// to change with it, or two apps from one shell would clash on a device:
/// content provider authorities (FileProvider's, AndroidX Startup's), and
/// the permissions an app declares for itself (AndroidX's
/// `DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION`). Class names stay, since the
/// code doesn't move, but a relative one would now resolve against the new
/// package, so it's made absolute.
fn set_package(manifest: &mut dyn Document, id: &str) -> Result<()> {
    if !is_application_id(id) {
        bail!(
            "\"{id}\" isn't a valid Android application ID: it needs two or more parts separated by dots, each starting with a letter and holding only letters, digits and underscores"
        );
    }
    let package = manifest
        .attributes()
        .iter()
        .find_map(|attribute| match &attribute.value {
            Value::String(package)
                if attribute.element == "manifest"
                    && attribute.resource_id.is_none()
                    && attribute.name == "package" =>
            {
                Some(package.clone())
            }
            _ => None,
        })
        .context("it has no package to change")?;
    for index in 0..manifest.attributes().len() {
        let Attribute {
            element,
            name,
            resource_id,
            value,
        } = &manifest.attributes()[index];
        let Value::String(value) = value else {
            continue;
        };
        let changed = match (element.as_str(), *resource_id) {
            ("manifest", None) if name == "package" => id.to_owned(),
            (_, Some(AUTHORITIES)) => value
                .split(';')
                .map(|authority| derived(authority, &package, id))
                .collect::<Vec<_>>()
                .join(";"),
            (
                "permission"
                | "permission-group"
                | "permission-tree"
                | "uses-permission"
                | "uses-permission-sdk-23",
                Some(NAME),
            )
            | (_, Some(PERMISSION | READ_PERMISSION | WRITE_PERMISSION)) => {
                derived(value, &package, id)
            }
            (
                "application" | "activity" | "activity-alias" | "service" | "receiver" | "provider",
                Some(NAME),
            )
            | (
                _,
                Some(TARGET_ACTIVITY | BACKUP_AGENT | MANAGE_SPACE_ACTIVITY | PARENT_ACTIVITY_NAME),
            ) => absolute(value, &package),
            _ => continue,
        };
        if changed != *value {
            manifest.set_string(index, &changed);
        }
    }
    Ok(())
}

/// Whether `id` is a valid Android application ID: a Java package name of
/// at least two parts.
fn is_application_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            part.starts_with(|first: char| first.is_ascii_alphabetic())
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
}

/// The value with the old package swapped for the new ID, if it's derived
/// from the package: the package itself, or a name under it.
fn derived(value: &str, package: &str, id: &str) -> String {
    match value.strip_prefix(package) {
        Some(rest) if rest.is_empty() || rest.starts_with('.') => format!("{id}{rest}"),
        _ => value.to_owned(),
    }
}

/// A class name made absolute the way Android resolves it against the
/// package: `.Name`, and a name without a dot, are in the package.
fn absolute(class: &str, package: &str) -> String {
    if class.starts_with('.') {
        format!("{package}{class}")
    } else if class.contains('.') {
        class.to_owned()
    } else {
        format!("{package}.{class}")
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{
        AUTHORITIES, BACKUP_AGENT, Changes, Document, NAME, PERMISSION, TARGET_ACTIVITY,
        VERSION_CODE, Value as Typed, rewrite,
    };
    use crate::android::binary_xml::BinaryXml;
    use crate::android::binary_xml::tests::{
        Value::{Int, Ref, Str},
        binary_xml, shell_manifest,
    };
    use crate::config::App;

    /// The manifest rewritten, and the changes elsewhere.
    fn rewritten(data: &[u8], app: &App) -> Result<(Vec<u8>, Changes)> {
        let mut xml = BinaryXml::parse(data).unwrap();
        let changes = rewrite(&mut xml, app)?;
        Ok((xml.to_bytes(), changes))
    }

    /// Each attribute's element, name, and value as a string.
    fn values(data: &[u8]) -> Vec<(String, String, String)> {
        BinaryXml::parse(data)
            .unwrap()
            .attributes()
            .iter()
            .map(|attribute| {
                let value = match &attribute.value {
                    Typed::String(value) => value.clone(),
                    Typed::Int(value) => value.to_string(),
                    _ => String::new(),
                };
                (attribute.element.clone(), attribute.name.clone(), value)
            })
            .collect()
    }

    fn value(data: &[u8], element: &str, name: &str) -> String {
        values(data)
            .into_iter()
            .find(|(on, attribute, _)| on == element && attribute == name)
            .unwrap()
            .2
    }

    fn app(id: Option<&str>, version: Option<&str>, build: Option<u32>) -> App {
        App {
            id: id.map(str::to_owned),
            version: version.map(str::to_owned),
            build,
            ..App::default()
        }
    }

    fn named(name: &str) -> App {
        App {
            name: Some(name.to_owned()),
            ..App::default()
        }
    }

    #[test]
    fn leaves_the_manifest_as_it_was_without_changes() {
        let manifest = shell_manifest();

        assert_eq!(rewritten(&manifest, &App::default()).unwrap().0, manifest);
    }

    #[test]
    fn gives_the_shell_the_apps_id_version_and_build() {
        let manifest = shell_manifest();

        let rewritten = rewritten(
            &manifest,
            &app(Some("dev.example.app"), Some("2.5.0"), Some(42)),
        )
        .unwrap()
        .0;

        assert_eq!(value(&rewritten, "manifest", "package"), "dev.example.app");
        assert_eq!(value(&rewritten, "manifest", "versionName"), "2.5.0");
        assert_eq!(value(&rewritten, "manifest", "versionCode"), "42");
        // The activity was already absolute, and stays in the shell's package.
        assert_eq!(
            value(&rewritten, "activity", "name"),
            "dev.omnibuild.shell.MainActivity"
        );
        // Everything else is as it was.
        let unchanged = |values: Vec<(String, String, String)>| -> Vec<(String, String, String)> {
            values
                .into_iter()
                .filter(|(element, _, _)| element != "manifest")
                .collect()
        };
        assert_eq!(unchanged(values(&rewritten)), unchanged(values(&manifest)));
    }

    /// A manifest with every kind of value that derives from the package,
    /// and some that look like it but don't.
    fn manifest_with_derived_values() -> Vec<u8> {
        let package = "dev.omnibuild.shell";
        binary_xml(&[
            (
                "manifest",
                vec![
                    ("package", None, Str(package)),
                    ("versionCode", Some(VERSION_CODE), Int(1)),
                ],
            ),
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            (
                "permission",
                vec![(
                    "name",
                    Some(NAME),
                    Str("dev.omnibuild.shell.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION"),
                )],
            ),
            (
                "uses-permission",
                vec![(
                    "name",
                    Some(NAME),
                    Str("dev.omnibuild.shell.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION"),
                )],
            ),
            (
                "uses-permission",
                vec![("name", Some(NAME), Str("android.permission.INTERNET"))],
            ),
            (
                "application",
                vec![
                    ("name", Some(NAME), Str(".ShellApplication")),
                    ("backupAgent", Some(BACKUP_AGENT), Str("Backup")),
                ],
            ),
            (
                "activity",
                vec![("name", Some(NAME), Str("dev.omnibuild.shell.MainActivity"))],
            ),
            (
                "activity-alias",
                vec![
                    ("name", Some(NAME), Str(".Alias")),
                    (
                        "targetActivity",
                        Some(TARGET_ACTIVITY),
                        Str(".MainActivity"),
                    ),
                ],
            ),
            (
                "provider",
                vec![(
                    "authorities",
                    Some(AUTHORITIES),
                    Str(
                        "dev.omnibuild.shell.fileprovider;dev.omnibuild.shellfish;dev.omnibuild.shell",
                    ),
                )],
            ),
            (
                "receiver",
                vec![(
                    "permission",
                    Some(PERMISSION),
                    Str("dev.omnibuild.shell.SEND"),
                )],
            ),
            (
                "meta-data",
                vec![("name", Some(NAME), Str("dev.omnibuild.shell.Initializer"))],
            ),
        ])
    }

    #[test]
    fn changes_what_derives_from_the_application_id() {
        let manifest = manifest_with_derived_values();

        let rewritten = rewritten(&manifest, &app(Some("dev.example.app"), None, None))
            .unwrap()
            .0;

        let changed: Vec<String> = values(&rewritten)
            .into_iter()
            .map(|(_, _, value)| value)
            .collect();
        assert_eq!(
            changed,
            [
                "dev.example.app",
                "1",
                "24",
                "dev.example.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
                "dev.example.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
                "android.permission.INTERNET",
                "dev.omnibuild.shell.ShellApplication",
                "dev.omnibuild.shell.Backup",
                "dev.omnibuild.shell.MainActivity",
                "dev.omnibuild.shell.Alias",
                "dev.omnibuild.shell.MainActivity",
                "dev.example.app.fileprovider;dev.omnibuild.shellfish;dev.example.app",
                "dev.example.app.SEND",
                "dev.omnibuild.shell.Initializer",
            ]
        );
    }

    #[test]
    fn refuses_a_shell_for_android_before_7() {
        for uses_sdk in [
            vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(21))],
            // A preview's codename.
            vec![(
                "minSdkVersion",
                Some(super::MIN_SDK_VERSION),
                Str("Baklava"),
            )],
            vec![],
        ] {
            let manifest = binary_xml(&[("uses-sdk", uses_sdk)]);

            let error = rewritten(&manifest, &App::default()).unwrap_err();

            assert!(
                error
                    .to_string()
                    .starts_with("the shell runs on Android versions before 7.0"),
                "{error}"
            );
        }
    }

    #[test]
    fn rejects_values_android_wont_take() {
        let manifest = shell_manifest();
        for (app, message) in [
            (
                app(Some("app"), None, None),
                "\"app\" isn't a valid Android application ID",
            ),
            (
                app(Some("dev.1app"), None, None),
                "\"dev.1app\" isn't a valid Android application ID",
            ),
            (
                app(Some("dev.my-app"), None, None),
                "\"dev.my-app\" isn't a valid Android application ID",
            ),
            (
                app(None, None, Some(0)),
                "the build number 0 isn't between 1 and 2100000000",
            ),
            (
                app(None, None, Some(2_100_000_001)),
                "the build number 2100000001 isn't between 1 and 2100000000",
            ),
        ] {
            let error = rewritten(&manifest, &app).unwrap_err();

            assert!(error.to_string().starts_with(message), "{error}");
        }
    }

    #[test]
    fn puts_the_name_where_the_label_is() {
        let shell = shell_manifest();
        let literal = binary_xml(&[
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            (
                "application",
                vec![("label", Some(super::LABEL), Str("Shell"))],
            ),
        ]);

        let in_resources = rewritten(&shell, &named("Repackaged")).unwrap();
        let in_manifest = rewritten(&literal, &named("Repackaged")).unwrap();

        // The shell's label is @string/app_name.
        assert_eq!(in_resources.1.name_resource, Some(0x7f02_0000));
        assert_eq!(in_resources.0, shell);
        assert_eq!(in_manifest.1.name_resource, None);
        assert_eq!(value(&in_manifest.0, "application", "label"), "Repackaged");
    }

    #[test]
    fn finds_the_icon_and_round_icon() {
        let with_icon = App {
            icon: Some("icon.png".into()),
            ..App::default()
        };
        let both = binary_xml(&[
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            (
                "application",
                vec![
                    ("icon", Some(super::ICON), Ref(0x7f01_0000)),
                    ("roundIcon", Some(super::ROUND_ICON), Ref(0x7f01_0001)),
                ],
            ),
        ]);
        let not_a_reference = binary_xml(&[
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            (
                "application",
                vec![
                    ("icon", Some(super::ICON), Ref(0x7f01_0000)),
                    // A launcher couldn't load this, so it isn't changed.
                    ("roundIcon", Some(super::ROUND_ICON), Str("round.png")),
                ],
            ),
        ]);

        let shell = rewritten(&shell_manifest(), &with_icon).unwrap().1;
        let both = rewritten(&both, &with_icon).unwrap().1;
        let not_a_reference = rewritten(&not_a_reference, &with_icon).unwrap().1;
        let without = rewritten(&shell_manifest(), &App::default()).unwrap().1;

        assert_eq!(shell.icon_resources, [0x7f01_0000]);
        assert_eq!(both.icon_resources, [0x7f01_0000, 0x7f01_0001]);
        assert_eq!(not_a_reference.icon_resources, [0x7f01_0000]);
        assert_eq!(without.icon_resources, Vec::<u32>::new());
    }

    #[test]
    fn reports_a_manifest_without_what_it_changes() {
        let manifest = binary_xml(&[(
            "uses-sdk",
            vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
        )]);
        for (app, message) in [
            (
                app(Some("dev.example.app"), None, None),
                "it has no package to change",
            ),
            (
                app(None, Some("2.0"), None),
                "it has no android:versionName to change",
            ),
            (
                app(None, None, Some(2)),
                "it has no android:versionCode to change",
            ),
            (named("App"), "it has no android:label to change"),
            (
                App {
                    icon: Some("icon.png".into()),
                    ..App::default()
                },
                "it has no android:icon to change",
            ),
        ] {
            assert_eq!(rewritten(&manifest, &app).unwrap_err().to_string(), message);
        }
    }
}
