//! The app's values in a shell's manifest. An APK's manifest is binary
//! XML and an AAB's is XML in protobuf (`binary_xml.rs`, `proto_xml.rs`),
//! but which attributes change, and how, is the same for both.

use anyhow::{Context, Result, bail};

use crate::config::{App, check_url_scheme};

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
const SCHEME: u32 = 0x0101_0027;

/// The elements that start an app's components. The intent filters
/// inside one come after it in document order, before the next one.
const COMPONENTS: [&str; 5] = [
    "activity",
    "activity-alias",
    "service",
    "receiver",
    "provider",
];
/// The intent of the activity a launcher shows.
const MAIN: &str = "android.intent.action.MAIN";
const LAUNCHER: &str = "android.intent.category.LAUNCHER";
/// The intent a link opens an app with: to view what the URL names, from
/// a browser (`BROWSABLE`), or from another app, whose intent Android gives
/// the `DEFAULT` category.
const VIEW: &str = "android.intent.action.VIEW";
const DEFAULT: &str = "android.intent.category.DEFAULT";
const BROWSABLE: &str = "android.intent.category.BROWSABLE";
/// The elements that ask for a permission, the second only on Android 6.0
/// and later.
const PERMISSION_REQUESTS: [&str; 2] = ["uses-permission", "uses-permission-sdk-23"];

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
    /// Which element it's on: its place in document order, counting every
    /// element, with attributes or without.
    pub node: usize,
    pub name: String,
    /// The resource ID of an android: attribute. Others, like `package`,
    /// have none.
    pub resource_id: Option<u32>,
    pub value: Value,
}

/// An element to add to a document: its name, its android: attributes,
/// and the elements in it.
#[derive(Debug, Default)]
pub struct Element<'a> {
    pub name: &'a str,
    /// Each attribute's name and resource ID, such as `android:name`'s, and
    /// its value, a string.
    pub attributes: Vec<(&'a str, u32, &'a str)>,
    pub children: Vec<Element<'a>>,
}

impl<'a> Element<'a> {
    /// An element with one android: attribute, and nothing in it.
    pub fn with(name: &'a str, attribute: (&'a str, u32), value: &'a str) -> Self {
        Self {
            name,
            attributes: vec![(attribute.0, attribute.1, value)],
            children: Vec::new(),
        }
    }
}

/// Compiled XML whose attributes can be changed, and elements added and
/// removed, in either format: a manifest, or a resource such as an adaptive
/// icon.
pub trait Document {
    /// Every attribute, in document order.
    fn attributes(&self) -> &[Attribute];
    fn set_string(&mut self, index: usize, value: &str);
    fn set_int(&mut self, index: usize, value: u32);
    /// Removes the root's children named `name` whose attributes `remove`
    /// picks, with everything in them.
    fn remove_children(&mut self, name: &str, remove: &dyn Fn(&[&Attribute]) -> bool);
    /// Adds `element`, and the elements in it, to the element `parent`, by
    /// its place in document order: the root is 0. Where among the
    /// parent's children it goes is each format's own.
    fn add_element(&mut self, parent: usize, element: &Element) -> Result<()>;
    fn to_bytes(&self) -> Vec<u8>;
}

/// Where else the app's values go, besides the manifest.
#[derive(Debug)]
pub struct Changes {
    /// The oldest Android version the app runs on, by API level: its
    /// `minSdkVersion`, which decides how an APK is signed.
    pub min_sdk: u32,
    /// The string resources the app's name goes in, where the shell's
    /// labels are ones rather than text in the manifest: the application's,
    /// and any the launcher shows in its place.
    pub name_resources: Vec<u32>,
    /// The resources the app's icon goes in: the application's icon, and
    /// its round icon, which some launchers show instead.
    pub icon_resources: Vec<u32>,
}

/// Gives the manifest the app's ID, name, version, build number and
/// permissions.
pub fn rewrite(manifest: &mut dyn Document, app: &App) -> Result<Changes> {
    let min_sdk = match find(manifest, "uses-sdk", MIN_SDK_VERSION)
        .map(|index| &manifest.attributes()[index].value)
    {
        Some(Value::Int(level)) => *level,
        // A preview's codename, such as "Baklava": the app runs only on
        // that preview, which is newer than every Android released.
        Some(Value::String(_)) => u32::MAX,
        // Without one, Android's default: every version.
        _ => 1,
    };
    if let Some(id) = &app.id {
        set_package(manifest, id)?;
    }
    if let Some(permissions) = &app.android.permissions {
        set_permissions(manifest, permissions)?;
    }
    if let Some(schemes) = &app.url_schemes {
        add_url_schemes(manifest, schemes)?;
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
    // launcher activity, or its intent filter, has a label of its own.
    let mut name_resources = Vec::new();
    if let Some(name) = &app.name {
        let index =
            find(manifest, "application", LABEL).context("it has no android:label to change")?;
        let launcher_labels = launchers(manifest)
            .into_iter()
            .flat_map(|launcher| launcher.labels);
        for index in [index].into_iter().chain(launcher_labels) {
            match manifest.attributes()[index].value {
                Value::Reference(id) if name_resources.contains(&id) => {}
                Value::Reference(id) => name_resources.push(id),
                _ => manifest.set_string(index, name),
            }
        }
    }
    if app.android.adaptive_icon.is_some() && app.icon.is_none() {
        bail!(
            "an adaptive icon needs the app's icon too, which Android shows before 8.0: give an icon"
        );
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
        min_sdk,
        name_resources,
        icon_resources,
    })
}

/// An activity, or an alias of one, and whether a launcher shows it: if
/// one of its intent filters has the `MAIN` action and the `LAUNCHER`
/// category.
#[derive(Default)]
struct Component {
    /// Its place in document order.
    node: usize,
    /// The labels a launcher shows for it in place of the application's:
    /// its own, and its intent filters'.
    labels: Vec<usize>,
    main: bool,
    launcher: bool,
}

/// The activities and aliases a launcher shows, in document order. An
/// element's children follow it in document order, and components don't
/// nest, so the elements after a component, up to the next, are in it.
fn launchers(manifest: &dyn Document) -> Vec<Component> {
    let mut launchers = Vec::new();
    let mut current: Option<Component> = None;
    let mut finish = |component: Option<Component>| {
        launchers.extend(component.filter(|component| component.main && component.launcher));
    };
    for (index, attribute) in manifest.attributes().iter().enumerate() {
        let element = attribute.element.as_str();
        let is_new = current
            .as_ref()
            .is_none_or(|component| component.node != attribute.node);
        if is_new && COMPONENTS.contains(&element) {
            finish(current.take());
            current = matches!(element, "activity" | "activity-alias").then(|| Component {
                node: attribute.node,
                ..Component::default()
            });
        }
        let Some(component) = current.as_mut() else {
            continue;
        };
        match (element, attribute.resource_id, &attribute.value) {
            ("activity" | "activity-alias" | "intent-filter", Some(LABEL), _) => {
                component.labels.push(index);
            }
            ("action", Some(NAME), Value::String(action)) if action == MAIN => {
                component.main = true;
            }
            ("category", Some(NAME), Value::String(category)) if category == LAUNCHER => {
                component.launcher = true;
            }
            _ => {}
        }
    }
    finish(current);
    launchers
}

/// Has the launcher activity open the app's URLs with these schemes, as
/// deep links: an intent filter for them, for those the manifest doesn't
/// have already.
fn add_url_schemes(manifest: &mut dyn Document, schemes: &[String]) -> Result<()> {
    for scheme in schemes {
        check_url_scheme(scheme)?;
    }
    let has = |scheme: &str| {
        manifest.attributes().iter().any(|attribute| {
            attribute.element == "data"
                && attribute.resource_id == Some(SCHEME)
                && attribute.value == Value::String(scheme.to_owned())
        })
    };
    let mut data: Vec<Element> = Vec::new();
    for scheme in schemes {
        if !has(scheme) && !data.iter().any(|element| element.attributes[0].2 == scheme) {
            data.push(Element::with("data", ("scheme", SCHEME), scheme));
        }
    }
    if data.is_empty() {
        return Ok(());
    }
    let launcher = launchers(manifest)
        .first()
        .map(|launcher| launcher.node)
        .context("it has no launcher activity to open the app's URLs")?;
    let filter = Element {
        name: "intent-filter",
        attributes: Vec::new(),
        children: [
            Element::with("action", ("name", NAME), VIEW),
            Element::with("category", ("name", NAME), DEFAULT),
            Element::with("category", ("name", NAME), BROWSABLE),
        ]
        .into_iter()
        .chain(data)
        .collect(),
    };
    manifest.add_element(launcher, &filter)
}

/// Gives the app the permissions it asks for, `android.permission.` and
/// the rest of the name, or the whole name: the shell's others go, but
/// those under the app's own ID, which the shell's code declares and uses
/// itself (AndroidX's `DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION`).
fn set_permissions(manifest: &mut dyn Document, permissions: &[String]) -> Result<()> {
    let own = format!("{}.", package(manifest)?);
    let mut wanted = Vec::new();
    for permission in permissions {
        let name = if permission.contains('.') {
            permission.clone()
        } else {
            format!("android.permission.{permission}")
        };
        if !is_application_id(&name) {
            bail!(
                "\"{permission}\" isn't the name of a permission, such as android.permission.CAMERA, or CAMERA for short"
            );
        }
        wanted.push(name);
    }
    for element in PERMISSION_REQUESTS {
        manifest.remove_children(element, &|attributes| {
            attributes.iter().any(|attribute| {
                matches!(
                    (attribute.resource_id, &attribute.value),
                    (Some(NAME), Value::String(name)) if !wanted.contains(name) && !name.starts_with(&own)
                )
            })
        });
    }
    let requested: Vec<Value> = manifest
        .attributes()
        .iter()
        .filter(|attribute| {
            PERMISSION_REQUESTS.contains(&attribute.element.as_str())
                && attribute.resource_id == Some(NAME)
        })
        .map(|attribute| attribute.value.clone())
        .collect();
    for name in wanted
        .iter()
        .filter(|name| !requested.contains(&Value::String((*name).clone())))
    {
        manifest.add_element(0, &Element::with("uses-permission", ("name", NAME), name))?;
    }
    Ok(())
}

/// The manifest's package.
fn package(manifest: &dyn Document) -> Result<String> {
    manifest
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
        .context("it has no package to change")
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
    let package = package(manifest)?;
    for index in 0..manifest.attributes().len() {
        let Attribute {
            element,
            name,
            resource_id,
            value,
            ..
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
        AUTHORITIES, BACKUP_AGENT, Changes, Document, Element, NAME, PERMISSION, SCHEME,
        TARGET_ACTIVITY, VERSION_CODE, Value as Typed, rewrite,
    };
    use crate::android::binary_xml::BinaryXml;
    use crate::android::binary_xml::tests::{
        Value::{Int, Ref, Str},
        binary_xml, names, shell_manifest, texts,
    };
    use crate::config::{AdaptiveIcon, Android, App, Background};

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
        // What Gradle derived from the application ID moves with it.
        assert_eq!(
            value(&rewritten, "provider", "authorities"),
            "dev.example.app.provider"
        );
        assert_eq!(
            value(&rewritten, "permission", "name"),
            "dev.example.app.permission.SHELL"
        );
        // Everything else is as it was, but for those, and the permission
        // the shell asks for under its own.
        let moved = [
            "dev.omnibuild.shell.provider",
            "dev.omnibuild.shell.permission.SHELL",
        ];
        let others = |values: Vec<(String, String, String)>| -> Vec<(String, String, String)> {
            values
                .into_iter()
                .filter(|(element, _, _)| element != "manifest")
                .map(|(element, name, value)| {
                    if moved.contains(&value.as_str()) {
                        let value = value.replace("dev.omnibuild.shell", "dev.example.app");
                        (element, name, value)
                    } else {
                        (element, name, value)
                    }
                })
                .collect()
        };
        assert_eq!(others(values(&rewritten)), others(values(&manifest)));
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
    fn reads_the_oldest_android_the_shell_runs_on() {
        for (uses_sdk, min_sdk) in [
            (
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(21))],
                21,
            ),
            // A preview's codename.
            (
                vec![(
                    "minSdkVersion",
                    Some(super::MIN_SDK_VERSION),
                    Str("Baklava"),
                )],
                u32::MAX,
            ),
            (vec![], 1),
        ] {
            let manifest = binary_xml(&[("uses-sdk", uses_sdk)]);

            let changes = rewritten(&manifest, &App::default()).unwrap().1;

            assert_eq!(changes.min_sdk, min_sdk);
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

        // The shell's label is @string/app_name, and its launcher
        // activity's @string/launcher_name.
        assert_eq!(in_resources.1.name_resources, [0x7f03_0000, 0x7f03_0001]);
        assert_eq!(in_resources.0, shell);
        assert_eq!(in_manifest.1.name_resources, Vec::<u32>::new());
        assert_eq!(value(&in_manifest.0, "application", "label"), "Repackaged");
    }

    #[test]
    fn names_what_the_launcher_shows_in_place_of_the_application() {
        let name = |name| ("name", Some(NAME), Str(name));
        let label = |label| ("label", Some(super::LABEL), label);
        let manifest = binary_xml(&[
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            ("application", vec![label(Str("App"))]),
            // Not a launcher's: no LAUNCHER category.
            ("activity", vec![name(".Settings"), label(Str("Settings"))]),
            ("action", vec![name(super::MAIN)]),
            ("activity", vec![name(".Main"), label(Ref(0x7f03_0001))]),
            ("intent-filter", vec![label(Str("Open"))]),
            ("action", vec![name(super::MAIN)]),
            ("category", vec![name(super::LAUNCHER)]),
            // Another component ends the activity, and isn't one a
            // launcher shows.
            ("service", vec![name(".Sync"), label(Str("Sync"))]),
            ("category", vec![name(super::LAUNCHER)]),
            ("activity-alias", vec![name(".Alias"), label(Str("Alias"))]),
            ("action", vec![name(super::MAIN)]),
            ("category", vec![name(super::LAUNCHER)]),
        ]);

        let (rewritten, changes) = rewritten(&manifest, &named("Repackaged")).unwrap();
        // One resource for both labels, which is set once.
        let shared = binary_xml(&[
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            ("application", vec![label(Ref(0x7f03_0000))]),
            ("activity", vec![name(".Main"), label(Ref(0x7f03_0000))]),
            ("action", vec![name(super::MAIN)]),
            ("category", vec![name(super::LAUNCHER)]),
        ]);

        assert_eq!(changes.name_resources, [0x7f03_0001]);
        assert_eq!(
            super::tests::rewritten(&shared, &named("Repackaged"))
                .unwrap()
                .1
                .name_resources,
            [0x7f03_0000]
        );
        let labels: Vec<(String, String)> = values(&rewritten)
            .into_iter()
            .filter(|(_, attribute, _)| attribute == "label")
            .map(|(element, _, label)| (element, label))
            .collect();
        let label = |element: &str, label: &str| (element.to_owned(), label.to_owned());
        assert_eq!(
            labels,
            [
                label("application", "Repackaged"),
                label("activity", "Settings"),
                label("activity", ""),
                label("intent-filter", "Repackaged"),
                label("service", "Sync"),
                label("activity-alias", "Repackaged"),
            ]
        );
    }

    fn with_schemes(schemes: &[&str]) -> App {
        App {
            url_schemes: Some(schemes.iter().map(|&scheme| scheme.to_owned()).collect()),
            ..App::default()
        }
    }

    /// Each element in the manifest from the launcher activity on, and its
    /// android:name or android:scheme.
    fn from_the_activity(manifest: &[u8]) -> Vec<(String, Typed)> {
        let xml = BinaryXml::parse(manifest).unwrap();
        let activity = xml
            .attributes()
            .iter()
            .position(|attribute| attribute.element == "activity")
            .unwrap();
        xml.attributes()[activity..]
            .iter()
            .filter(|attribute| matches!(attribute.resource_id, Some(NAME | SCHEME)))
            .map(|attribute| (attribute.element.clone(), attribute.value.clone()))
            .collect()
    }

    #[test]
    fn opens_the_apps_urls_in_the_launcher_activity() {
        let shell = shell_manifest();

        let manifest = rewritten(&shell, &with_schemes(&["myapp", "other", "myapp"]))
            .unwrap()
            .0;
        // Again, as when an app is repackaged a second time.
        let again = rewritten(&manifest, &with_schemes(&["other"])).unwrap().0;
        let none = rewritten(&shell, &with_schemes(&[])).unwrap().0;

        let text =
            |element: &str, value: &str| (element.to_owned(), Typed::String(value.to_owned()));
        let elements = from_the_activity(&manifest);
        assert_eq!(
            elements[..7],
            [
                text("activity", "dev.omnibuild.shell.MainActivity"),
                text("action", "android.intent.action.VIEW"),
                text("category", "android.intent.category.DEFAULT"),
                text("category", "android.intent.category.BROWSABLE"),
                text("data", "myapp"),
                text("data", "other"),
                text("action", "android.intent.action.MAIN"),
            ]
        );
        assert_eq!(again, manifest);
        assert_eq!(none, shell);
    }

    #[test]
    fn says_why_it_cant_open_the_apps_urls() {
        let no_launcher = binary_xml(&[(
            "uses-sdk",
            vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
        )]);
        let cases = [
            (
                shell_manifest(),
                with_schemes(&["My App"]),
                "\"My App\" isn't a URL scheme",
            ),
            (
                no_launcher,
                with_schemes(&["myapp"]),
                "it has no launcher activity to open the app's URLs",
            ),
        ];

        for (manifest, app, message) in cases {
            let error = rewritten(&manifest, &app).unwrap_err().to_string();

            assert!(error.starts_with(message), "{error}");
        }
    }

    fn with_permissions(id: Option<&str>, permissions: &[&str]) -> App {
        App {
            id: id.map(str::to_owned),
            android: Android {
                permissions: Some(permissions.iter().map(|&name| name.to_owned()).collect()),
                ..Android::default()
            },
            ..App::default()
        }
    }

    #[test]
    fn asks_for_the_permissions_the_app_does() {
        let mut shell = BinaryXml::parse(&shell_manifest()).unwrap();
        shell
            .add_element(
                0,
                &Element::with(
                    "uses-permission-sdk-23",
                    ("name", NAME),
                    "android.permission.CAMERA",
                ),
            )
            .unwrap();
        let shell = shell.to_bytes();

        let kept = rewritten(
            &shell,
            &with_permissions(None, &["INTERNET", "android.permission.VIBRATE"]),
        )
        .unwrap()
        .0;
        let none = rewritten(&shell, &with_permissions(Some("dev.example.app"), &[]))
            .unwrap()
            .0;
        let unchanged = rewritten(&shell, &App::default()).unwrap().0;

        let requests = |manifest: &[u8]| {
            let xml = BinaryXml::parse(manifest).unwrap();
            [
                names(&xml, "uses-permission"),
                names(&xml, "uses-permission-sdk-23"),
            ]
            .concat()
        };
        // CAMERA goes, VIBRATE comes, and the shell's own stays.
        assert_eq!(
            requests(&kept),
            texts(&[
                "android.permission.VIBRATE",
                "android.permission.INTERNET",
                "dev.omnibuild.shell.permission.SHELL",
            ])
        );
        // Under the app's own ID, once it has one.
        assert_eq!(
            requests(&none),
            texts(&["dev.example.app.permission.SHELL"])
        );
        assert_eq!(unchanged, shell);
    }

    #[test]
    fn says_why_it_cant_ask_for_a_permission() {
        // A manifest without the android: namespace to add one in.
        let no_namespace = binary_xml(&[
            (
                "manifest",
                vec![("package", None, Str("dev.omnibuild.shell"))],
            ),
            (
                "uses-sdk",
                vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
            ),
            ("activity", vec![("name", Some(NAME), Str(".Main"))]),
        ]);

        let no_package = binary_xml(&[(
            "uses-sdk",
            vec![("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24))],
        )]);

        let bad_name = rewritten(&shell_manifest(), &with_permissions(None, &["CAMERA!"]));
        let not_added = rewritten(&no_namespace, &with_permissions(None, &["CAMERA"]));
        let unnamed = rewritten(&no_package, &with_permissions(None, &["CAMERA"]));

        assert_eq!(
            unnamed.unwrap_err().to_string(),
            "it has no package to change"
        );

        assert_eq!(
            bad_name.unwrap_err().to_string(),
            "\"CAMERA!\" isn't the name of a permission, such as android.permission.CAMERA, or CAMERA for short"
        );
        assert_eq!(
            not_added.unwrap_err().to_string(),
            "it declares no android: namespace"
        );
    }

    #[test]
    fn needs_the_apps_icon_with_an_adaptive_icon() {
        let app = App {
            android: Android {
                adaptive_icon: Some(AdaptiveIcon {
                    foreground: "front.png".into(),
                    background: Background::Color([0; 3]),
                    monochrome: None,
                }),
                ..Android::default()
            },
            ..App::default()
        };

        assert_eq!(
            rewritten(&shell_manifest(), &app).unwrap_err().to_string(),
            "an adaptive icon needs the app's icon too, which Android shows before 8.0: give an icon"
        );
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

        // The shell's @mipmap/ic_launcher and @mipmap/ic_launcher_round.
        assert_eq!(shell.icon_resources, [0x7f02_0000, 0x7f02_0001]);
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
