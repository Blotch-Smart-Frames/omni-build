//! The binary `AndroidManifest.xml` in an APK: Android's binary XML
//! (`ResXMLTree` in AOSP's `ResourceTypes.h`), as aapt2 compiles it.
//!
//! Binary XML is a string pool, a resource map giving the resource ID of
//! each attribute name, and the nodes, whose attributes hold string pool
//! indexes or typed values. OmniBuild changes attribute values in place and
//! adds the strings they need to the end of the pool, so every other node,
//! index and string stays as aapt2 wrote it.

use anyhow::{Context, Result, bail};

use super::chunk::chunks;
use super::string_pool::StringPool;
use crate::bytes::Reader;
use crate::config::App;

const XML: u16 = 0x0003;
const RESOURCE_MAP: u16 = 0x0180;
const START_ELEMENT: u16 = 0x0102;
/// A string pool index meaning "no string".
const NO_STRING: u32 = u32::MAX;
/// `Res_value` data types.
const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;

// The android: attributes OmniBuild reads or rewrites, by resource ID, as
// Android itself identifies them (android.R.attr).
const LABEL: u32 = 0x0101_0001;
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

/// Android 7.0, the first to verify APK Signature Scheme v2. Earlier
/// versions only verify a v1 (JAR) signature, which OmniBuild doesn't write.
const MIN_SDK: u32 = 24;
/// The highest version code Google Play accepts.
const MAX_VERSION_CODE: u32 = 2_100_000_000;

/// The rewritten manifest, and where else the app's values go.
#[derive(Debug)]
pub struct Rewritten {
    pub manifest: Vec<u8>,
    /// The string resource the app's name goes in, when the shell's label
    /// is one rather than text in the manifest.
    pub name_resource: Option<u32>,
}

/// Gives the manifest the app's ID, name, version and build number.
pub fn rewrite(data: &[u8], app: &App) -> Result<Rewritten> {
    let mut manifest = Manifest::parse(data)?;
    let min_sdk = manifest
        .find("uses-sdk", MIN_SDK_VERSION)
        .and_then(|attribute| manifest.int(attribute))
        .unwrap_or(1);
    if min_sdk < MIN_SDK {
        bail!(
            "the shell runs on Android versions before 7.0 (its minSdkVersion is {min_sdk}), which need a v1 signature. OmniBuild only signs with v2 and v3, for Android 7.0 (API 24) and later"
        );
    }
    if let Some(id) = &app.id {
        manifest.set_package(id)?;
    }
    if let Some(version) = &app.version {
        let attribute = manifest
            .find("manifest", VERSION_NAME)
            .context("it has no android:versionName to change")?;
        manifest.set_string(attribute, version);
    }
    if let Some(build) = app.build {
        if !(1..=MAX_VERSION_CODE).contains(&build) {
            bail!(
                "the build number {build} isn't between 1 and {MAX_VERSION_CODE}, which Google Play requires of a version code"
            );
        }
        let attribute = manifest
            .find("manifest", VERSION_CODE)
            .context("it has no android:versionCode to change")?;
        manifest.set_int(attribute, build);
    }
    // The application's label is the name under the icon, unless the
    // launcher activity has a label of its own: shells shouldn't.
    let mut name_resource = None;
    if let Some(name) = &app.name {
        let attribute = manifest
            .find("application", LABEL)
            .context("it has no android:label to change")?;
        match manifest.typed(attribute) {
            (TYPE_REFERENCE, id) => name_resource = Some(id),
            _ => manifest.set_string(attribute, name),
        }
    }
    Ok(Rewritten {
        manifest: manifest.to_bytes(),
        name_resource,
    })
}

struct Manifest {
    pool: StringPool,
    /// Every chunk after the string pool: the resource map and the nodes,
    /// which rewriting changes in place.
    nodes: Vec<u8>,
    attributes: Vec<Attribute>,
}

struct Attribute {
    /// The name of the element it's on.
    element: String,
    name: String,
    /// The resource ID of an android: attribute. Others, like `package`,
    /// have none.
    resource_id: Option<u32>,
    /// Where its record starts in `nodes`.
    offset: usize,
}

impl Manifest {
    fn parse(data: &[u8]) -> Result<Self> {
        let mut header = Reader::at(data, 0);
        let (kind, header_size, size) = (header.u16(), header.u16(), header.u32());
        if kind != XML || header_size != 8 || offset(size) != data.len() {
            bail!("it isn't binary XML");
        }
        let (pool, pool_size) = StringPool::parse(&data[8..])?;
        let nodes = data[8 + pool_size..].to_vec();

        let mut resource_map = Vec::new();
        let mut attributes = Vec::new();
        for chunk in chunks(&nodes, 0, nodes.len())? {
            match chunk.kind {
                RESOURCE_MAP => {
                    let mut ids = Reader::at(&nodes, chunk.body());
                    resource_map = (0..(chunk.size - chunk.header_size) / 4)
                        .map(|_| ids.u32())
                        .collect();
                }
                START_ELEMENT => {
                    // The element's name, then where its attributes start,
                    // each one's size, and how many there are.
                    let mut element = Reader::at(&nodes, chunk.body());
                    element.skip(4);
                    let name = element.u32();
                    let (start, attribute_size, count) = (
                        usize::from(element.u16()),
                        usize::from(element.u16()),
                        usize::from(element.u16()),
                    );
                    if attribute_size < 20
                        || chunk.header_size + start + count * attribute_size > chunk.size
                    {
                        bail!("it's corrupt or cut short");
                    }
                    let element = pool.get(name).unwrap_or_default();
                    for index in 0..count {
                        let offset = chunk.body() + start + index * attribute_size;
                        let name = Reader::at(&nodes, offset + 4).u32();
                        attributes.push(Attribute {
                            element: element.clone(),
                            name: pool.get(name).unwrap_or_default(),
                            resource_id: resource_map.get(self::offset(name)).copied(),
                            offset,
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            pool,
            nodes,
            attributes,
        })
    }

    /// The index of the android: attribute with this resource ID on the
    /// first element with this name.
    fn find(&self, element: &str, resource_id: u32) -> Option<usize> {
        self.attributes.iter().position(|attribute| {
            attribute.element == element && attribute.resource_id == Some(resource_id)
        })
    }

    /// An attribute's typed value: its data type and data.
    fn typed(&self, attribute: usize) -> (u8, u32) {
        let mut value = Reader::at(&self.nodes, self.attributes[attribute].offset + 15);
        (value.u8(), value.u32())
    }

    fn string(&self, attribute: usize) -> Option<String> {
        match self.typed(attribute) {
            (TYPE_STRING, index) => self.pool.get(index),
            _ => None,
        }
    }

    fn int(&self, attribute: usize) -> Option<u32> {
        match self.typed(attribute) {
            (TYPE_INT_DEC | TYPE_INT_HEX, value) => Some(value),
            _ => None,
        }
    }

    /// Sets an attribute's raw value (a string, or none) and typed value.
    fn set(&mut self, attribute: usize, raw: u32, data_type: u8, data: u32) {
        let offset = self.attributes[attribute].offset;
        self.nodes[offset + 8..offset + 12].copy_from_slice(&raw.to_le_bytes());
        self.nodes[offset + 15] = data_type;
        self.nodes[offset + 16..offset + 20].copy_from_slice(&data.to_le_bytes());
    }

    fn set_string(&mut self, attribute: usize, value: &str) {
        let index = self.pool.add(value);
        self.set(attribute, index, TYPE_STRING, index);
    }

    fn set_int(&mut self, attribute: usize, value: u32) {
        self.set(attribute, NO_STRING, TYPE_INT_DEC, value);
    }

    /// Gives the app a new ID: the manifest's `package`, and the values
    /// AGP derives from the application ID through `${applicationId}`.
    /// Those have to change with it, or two apps from one shell would clash
    /// on a device: content provider authorities (FileProvider's, AndroidX
    /// Startup's), and the permissions an app declares for itself (AndroidX's
    /// `DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION`). Class names stay, since
    /// the code doesn't move, but a relative one would now resolve against
    /// the new package, so it's made absolute.
    fn set_package(&mut self, id: &str) -> Result<()> {
        if !is_application_id(id) {
            bail!(
                "\"{id}\" isn't a valid Android application ID: it needs two or more parts separated by dots, each starting with a letter and holding only letters, digits and underscores"
            );
        }
        let package = self
            .attributes
            .iter()
            .position(|attribute| {
                attribute.element == "manifest"
                    && attribute.resource_id.is_none()
                    && attribute.name == "package"
            })
            .and_then(|attribute| self.string(attribute))
            .context("it has no package to change")?;
        for attribute in 0..self.attributes.len() {
            let Some(value) = self.string(attribute) else {
                continue;
            };
            let Attribute {
                element,
                name,
                resource_id,
                ..
            } = &self.attributes[attribute];
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
                    derived(&value, &package, id)
                }
                (
                    "application" | "activity" | "activity-alias" | "service" | "receiver"
                    | "provider",
                    Some(NAME),
                )
                | (
                    _,
                    Some(
                        TARGET_ACTIVITY
                        | BACKUP_AGENT
                        | MANAGE_SPACE_ACTIVITY
                        | PARENT_ACTIVITY_NAME,
                    ),
                ) => absolute(&value, &package),
                _ => continue,
            };
            if changed != value {
                self.set_string(attribute, &changed);
            }
        }
        Ok(())
    }

    fn to_bytes(&self) -> Vec<u8> {
        let pool = self.pool.to_bytes();
        let size =
            u32::try_from(8 + pool.len() + self.nodes.len()).expect("manifests are under 4 GiB");
        [
            &XML.to_le_bytes()[..],
            &8u16.to_le_bytes(),
            &size.to_le_bytes(),
            &pool,
            &self.nodes,
        ]
        .concat()
    }
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

fn offset(value: u32) -> usize {
    usize::try_from(value).expect("OmniBuild runs on 32 and 64-bit systems")
}

#[cfg(test)]
mod tests {
    use super::{
        AUTHORITIES, BACKUP_AGENT, Manifest, NAME, NO_STRING, PERMISSION, TARGET_ACTIVITY,
        TYPE_INT_DEC, TYPE_STRING, VERSION_CODE, VERSION_NAME, rewrite,
    };
    use crate::android::string_pool::tests::empty;
    use crate::config::App;
    use crate::zip::{self, tests::SHELL};

    enum Value<'a> {
        Str(&'a str),
        Int(u32),
    }
    use Value::{Int, Str};

    type Element<'a> = (&'a str, Vec<(&'a str, Option<u32>, Value<'a>)>);

    /// Binary XML with these elements, one after another: rewriting
    /// doesn't look at how they nest. Attributes with a resource ID are
    /// android: ones.
    fn binary_xml(elements: &[Element]) -> Vec<u8> {
        let mut pool = empty(0);
        // Attribute names with resource IDs come first in the pool, in the
        // resource map's order.
        let mut map: Vec<(&str, u32)> = Vec::new();
        for (_, attributes) in elements {
            for (name, id, _) in attributes {
                if let Some(id) = id
                    && !map.iter().any(|(mapped, _)| mapped == name)
                {
                    pool.add(name);
                    map.push((name, *id));
                }
            }
        }
        let mut nodes: Vec<u8> = [0x0180u16.to_le_bytes(), 8u16.to_le_bytes()].concat();
        nodes.extend(u32::try_from(8 + 4 * map.len()).unwrap().to_le_bytes());
        nodes.extend(map.iter().flat_map(|(_, id)| id.to_le_bytes()));
        for (element, attributes) in elements {
            let name = pool.add(element);
            let mut records = Vec::new();
            for (attribute, id, value) in attributes {
                let attribute_name = match id {
                    Some(_) => u32::try_from(
                        map.iter()
                            .position(|(mapped, _)| mapped == attribute)
                            .unwrap(),
                    )
                    .unwrap(),
                    None => pool.add(attribute),
                };
                let (raw, data_type, data) = match value {
                    Str(value) => {
                        let index = pool.add(value);
                        (index, TYPE_STRING, index)
                    }
                    Int(value) => (NO_STRING, TYPE_INT_DEC, *value),
                };
                records.extend(NO_STRING.to_le_bytes());
                records.extend(attribute_name.to_le_bytes());
                records.extend(raw.to_le_bytes());
                records.extend([8, 0, 0, data_type]);
                records.extend(data.to_le_bytes());
            }
            let size = u32::try_from(36 + records.len()).unwrap();
            let count = u16::try_from(attributes.len()).unwrap();
            for field in [
                &0x0102u16.to_le_bytes()[..],
                &16u16.to_le_bytes(),
                &size.to_le_bytes(),
                &1u32.to_le_bytes(),
                &NO_STRING.to_le_bytes(),
                &NO_STRING.to_le_bytes(),
                &name.to_le_bytes(),
                &20u16.to_le_bytes(),
                &20u16.to_le_bytes(),
                &count.to_le_bytes(),
                &[0; 6],
                &records,
            ] {
                nodes.extend(field);
            }
        }
        let pool = pool.to_bytes();
        let size = u32::try_from(8 + pool.len() + nodes.len()).unwrap();
        [
            &0x0003u16.to_le_bytes()[..],
            &8u16.to_le_bytes(),
            &size.to_le_bytes(),
            &pool,
            &nodes,
        ]
        .concat()
    }

    fn shell_manifest() -> Vec<u8> {
        zip::read(SHELL).unwrap()[2].contents().unwrap()
    }

    /// The string value of each attribute, or its integer as a string.
    fn values(data: &[u8]) -> Vec<(String, String, String)> {
        let manifest = Manifest::parse(data).unwrap();
        (0..manifest.attributes.len())
            .map(|index| {
                let attribute = &manifest.attributes[index];
                let value = manifest
                    .string(index)
                    .or_else(|| manifest.int(index).map(|value| value.to_string()))
                    .unwrap_or_default();
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

        assert_eq!(
            rewrite(&manifest, &App::default()).unwrap().manifest,
            manifest
        );
    }

    #[test]
    fn gives_the_shell_the_apps_id_version_and_build() {
        let manifest = shell_manifest();

        let rewritten = rewrite(
            &manifest,
            &app(Some("dev.example.app"), Some("2.5.0"), Some(42)),
        )
        .unwrap()
        .manifest;

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

        let rewritten = rewrite(&manifest, &app(Some("dev.example.app"), None, None))
            .unwrap()
            .manifest;

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
            vec![],
        ] {
            let manifest = binary_xml(&[("uses-sdk", uses_sdk)]);

            let error = rewrite(&manifest, &App::default()).unwrap_err();

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
            let error = rewrite(&manifest, &app).unwrap_err();

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

        let in_resources = rewrite(&shell, &named("Repackaged")).unwrap();
        let in_manifest = rewrite(&literal, &named("Repackaged")).unwrap();

        // The shell's label is @string/app_name.
        assert_eq!(in_resources.name_resource, Some(0x7f02_0000));
        assert_eq!(in_resources.manifest, shell);
        assert_eq!(in_manifest.name_resource, None);
        assert_eq!(
            value(&in_manifest.manifest, "application", "label"),
            "Repackaged"
        );
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
        ] {
            assert_eq!(rewrite(&manifest, &app).unwrap_err().to_string(), message);
        }
    }

    #[test]
    fn reads_a_version_code_in_hex() {
        let mut manifest = binary_xml(&[(
            "uses-sdk",
            vec![
                ("minSdkVersion", Some(super::MIN_SDK_VERSION), Int(24)),
                ("versionName", Some(VERSION_NAME), Str("1.0")),
            ],
        )]);
        // Make minSdkVersion's type hexadecimal.
        let at = manifest.len() - 2 * 20 + 15;
        manifest[at] = super::TYPE_INT_HEX;

        assert!(rewrite(&manifest, &App::default()).is_ok());
    }

    #[test]
    fn rejects_what_isnt_binary_xml() {
        let manifest = shell_manifest();
        let mut wrong_size = manifest.clone();
        wrong_size.push(0);
        let mut bad_pool = manifest.clone();
        bad_pool[8] = 0;
        let mut bad_node = manifest.clone();
        // The resource map's size, after the pool, made too small.
        let pool_size =
            usize::try_from(u32::from_le_bytes(manifest[12..16].try_into().unwrap())).unwrap();
        bad_node[8 + pool_size + 4..8 + pool_size + 8].copy_from_slice(&4u32.to_le_bytes());
        let mut bad_attributes = binary_xml(&[("manifest", vec![("package", None, Str("a.b"))])]);
        // The element's attribute size, made smaller than a record.
        let at = bad_attributes.len() - 20 - 20 + 10;
        bad_attributes[at] = 4;

        for (corrupt, message) in [
            (&b"<manifest/>"[..], "it isn't binary XML"),
            (&wrong_size, "it isn't binary XML"),
            (&bad_pool, "its string pool is corrupt"),
            (&bad_node, "it's corrupt or cut short"),
            (&bad_attributes, "it's corrupt or cut short"),
        ] {
            assert_eq!(
                rewrite(corrupt, &App::default()).unwrap_err().to_string(),
                message
            );
        }
    }
}
