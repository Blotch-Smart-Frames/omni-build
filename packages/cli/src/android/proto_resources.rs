//! The resource table in an Android App Bundle, `resources.pb`: aapt2's
//! `ResourceTable` message (`Resources.proto`), which bundletool compiles
//! into each APK's `resources.arsc`, as Google Play does.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use super::protobuf::{self, Field, Wire};
use super::resources::{Resources, Value};
use crate::config::closest;

// Field numbers in Resources.proto, along the path from the table to a
// value: a package, its types, their entries, each entry's value in each
// configuration.
const TABLE_PACKAGE: u32 = 2;
const PACKAGE_TYPE: u32 = 3;
const TYPE_ENTRY: u32 = 3;
const ENTRY_CONFIG_VALUE: u32 = 6;
const CONFIG_VALUE_CONFIG: u32 = 1;
const CONFIG_VALUE_VALUE: u32 = 2;
/// A `Configuration`'s language, as a BCP 47 tag, such as `pt-BR`
/// (Configuration.proto).
const CONFIGURATION_LOCALE: u32 = 3;
const VALUE_ITEM: u32 = 4;
const VALUE_COMPOUND: u32 = 5;
const ITEM_STRING: u32 = 2;
const ITEM_FILE: u32 = 5;
const ITEM_PRIMITIVE: u32 = 7;
const STRING_VALUE: u32 = 1;
const FILE_PATH: u32 = 1;
const FILE_TYPE: u32 = 2;
/// `FileReference.Type`'s PNG.
const PNG: u64 = 1;
/// A primitive's colour, in each of the ways aapt2 writes one: ARGB8, RGB8,
/// ARGB4 and RGB4, each kept as 32-bit ARGB.
const PRIMITIVE_COLORS: [u32; 4] = [9, 10, 11, 12];
const PRIMITIVE_ARGB8: u32 = 9;

pub struct ProtoResources {
    data: Vec<u8>,
}

impl ProtoResources {
    pub fn parse(data: &[u8]) -> Result<Self> {
        protobuf::decode(data)?;
        Ok(Self {
            data: data.to_vec(),
        })
    }

    /// Calls `visit` with the resource's `Value` in each configuration it
    /// has one in. Returns the table with the values as `visit` left them.
    fn visit(
        &self,
        id: u32,
        visit: &mut dyn FnMut(&mut Vec<Field>) -> Result<()>,
    ) -> Result<Vec<u8>> {
        // A resource ID is its package's ID, its type's, and its entry's:
        // 0xPPTTEEEE.
        let ids = [id >> 24, (id >> 16) & 0xff, id & 0xffff];
        edit(
            &self.data,
            &[
                TABLE_PACKAGE,
                PACKAGE_TYPE,
                TYPE_ENTRY,
                ENTRY_CONFIG_VALUE,
                CONFIG_VALUE_VALUE,
            ],
            &ids,
            visit,
        )
    }
}

/// Calls `visit` with each message at the end of `path`, a field number at
/// each level, and returns the message with them as `visit` left them.
/// Where `ids` has an ID for a level, only the messages with that ID are
/// followed: a package's, type's and entry's ID is in its field 1, as a
/// message of its own.
fn edit(
    message: &[u8],
    path: &[u32],
    ids: &[u32],
    visit: &mut dyn FnMut(&mut Vec<Field>) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut fields = protobuf::decode(message)?;
    if let Some((&number, path)) = path.split_first() {
        for field in &mut fields {
            if let (true, Wire::Bytes(child)) = (field.number == number, &mut field.value)
                && ids.first().is_none_or(|&id| id_of(child) == id)
            {
                *child = edit(child, path, ids.get(1..).unwrap_or_default(), visit)?;
            }
        }
    } else {
        visit(&mut fields)?;
    }
    Ok(protobuf::encode(&fields))
}

/// The ID of a package, type or entry. proto3 leaves out an ID of 0.
fn id_of(message: &[u8]) -> u32 {
    protobuf::decode(message)
        .ok()
        .and_then(|fields| protobuf::uint(&protobuf::message(&fields, 1), 1))
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or(0)
}

impl ProtoResources {
    /// Gives the resource this `Item` in every configuration it has a value
    /// in.
    fn set_item(&mut self, id: u32, item: &[u8]) -> Result<()> {
        let mut found = false;
        let data = self.visit(id, &mut |value| {
            if protobuf::bytes(value, VALUE_COMPOUND).is_some() {
                bail!("resource {id:#010x} holds several values, not one");
            }
            protobuf::set(value, Field::bytes(VALUE_ITEM, item));
            found = true;
            Ok(())
        })?;
        if !found {
            bail!("it has no resource {id:#010x}");
        }
        self.data = data;
        Ok(())
    }
}

/// An `Item` that's a string.
fn string_item(value: &str) -> Vec<u8> {
    let string = protobuf::encode(&[Field::bytes(STRING_VALUE, value)]);
    protobuf::encode(&[Field::bytes(ITEM_STRING, string)])
}

/// A `Value` that's this string.
fn string_value(mut value: Vec<Field>, string: &str) -> Result<Vec<u8>> {
    if protobuf::bytes(&value, VALUE_COMPOUND).is_some() {
        bail!("the resource holds several values, not one");
    }
    protobuf::set(&mut value, Field::bytes(VALUE_ITEM, string_item(string)));
    Ok(protobuf::encode(&value))
}

impl Resources for ProtoResources {
    fn values(&self, id: u32) -> Result<Vec<Value>> {
        let mut values = Vec::new();
        self.visit(id, &mut |value| {
            let item = protobuf::message(value, VALUE_ITEM);
            let primitive = protobuf::message(&item, ITEM_PRIMITIVE);
            let string = protobuf::string(&protobuf::message(&item, ITEM_STRING), STRING_VALUE);
            let file = protobuf::string(&protobuf::message(&item, ITEM_FILE), FILE_PATH);
            let color = PRIMITIVE_COLORS
                .iter()
                .find_map(|&field| protobuf::uint(&primitive, field));
            values.push(match (string, file, color) {
                (Some(string), _, _) => Value::String(string),
                (_, Some(path), _) => Value::File(path),
                (_, _, Some(color)) => Value::Color(u32::try_from(color).unwrap_or_default()),
                _ => Value::Other,
            });
            Ok(())
        })?;
        Ok(values)
    }

    fn files(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        edit(
            &self.data,
            &[
                TABLE_PACKAGE,
                PACKAGE_TYPE,
                TYPE_ENTRY,
                ENTRY_CONFIG_VALUE,
                CONFIG_VALUE_VALUE,
            ],
            &[],
            &mut |value| {
                let item = protobuf::message(value, VALUE_ITEM);
                files.extend(protobuf::string(
                    &protobuf::message(&item, ITEM_FILE),
                    FILE_PATH,
                ));
                Ok(())
            },
        )?;
        Ok(files)
    }

    fn set_string(&mut self, id: u32, value: &str) -> Result<()> {
        self.set_item(id, &string_item(value))
    }

    fn set_file(&mut self, id: u32, path: &str) -> Result<()> {
        let file =
            protobuf::encode(&[Field::bytes(FILE_PATH, path), Field::varint(FILE_TYPE, PNG)]);
        self.set_item(id, &protobuf::encode(&[Field::bytes(ITEM_FILE, file)]))
    }

    fn set_color(&mut self, id: u32, argb: u32) -> Result<()> {
        let primitive = protobuf::encode(&[Field::varint(PRIMITIVE_ARGB8, u64::from(argb))]);
        self.set_item(
            id,
            &protobuf::encode(&[Field::bytes(ITEM_PRIMITIVE, primitive)]),
        )
    }

    /// Each configuration is a `ConfigValue` in the resource's `Entry`, so
    /// a language gets one of its own there.
    fn translate(&mut self, id: u32, names: &BTreeMap<&str, &str>) -> Result<()> {
        let ids = [id >> 24, (id >> 16) & 0xff, id & 0xffff];
        let mut found = false;
        let data = edit(
            &self.data,
            &[TABLE_PACKAGE, PACKAGE_TYPE, TYPE_ENTRY],
            &ids,
            &mut |entry| {
                found = true;
                // The languages with a configuration of their own already.
                let mut plain = Vec::new();
                for config_value in entry.iter_mut().filter_map(|field| match &mut field.value {
                    Wire::Bytes(bytes) if field.number == ENTRY_CONFIG_VALUE => Some(bytes),
                    _ => None,
                }) {
                    let mut fields = protobuf::decode(config_value)?;
                    let config = protobuf::message(&fields, CONFIG_VALUE_CONFIG);
                    let tag = protobuf::string(&config, CONFIGURATION_LOCALE).unwrap_or_default();
                    if tag.is_empty() {
                        continue;
                    }
                    if let Some(name) = closest(names, &tag) {
                        let value = protobuf::message(&fields, CONFIG_VALUE_VALUE);
                        let value = string_value(value, name)?;
                        protobuf::set(&mut fields, Field::bytes(CONFIG_VALUE_VALUE, value));
                        *config_value = protobuf::encode(&fields);
                    }
                    // A language and nothing else.
                    if config.len() == 1 {
                        plain.push(tag);
                    }
                }
                for (&tag, &name) in names {
                    if !plain.iter().any(|plain| plain == tag) {
                        let config = [Field::bytes(CONFIGURATION_LOCALE, tag)];
                        let config_value = [
                            Field::bytes(CONFIG_VALUE_CONFIG, protobuf::encode(&config)),
                            Field::bytes(
                                CONFIG_VALUE_VALUE,
                                protobuf::encode(&[Field::bytes(VALUE_ITEM, string_item(name))]),
                            ),
                        ];
                        entry.push(Field::bytes(
                            ENTRY_CONFIG_VALUE,
                            protobuf::encode(&config_value),
                        ));
                    }
                }
                Ok(())
            },
        )?;
        if !found {
            bail!("it has no resource {id:#010x}");
        }
        self.data = data;
        Ok(())
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::ProtoResources;
    use crate::android::proto_xml::tests::SHELL_AAB;
    use crate::android::protobuf::{self, Field, encode};
    use crate::android::resources::tests::strings;
    use crate::android::resources::{Resources, Value};
    use crate::zip;

    const APP_NAME: u32 = 0x7f03_0000;
    const ICON: u32 = 0x7f02_0000;

    /// The shell's icon: a WebP for each density, and an adaptive icon.
    const SHELL_ICON: [&str; 3] = [
        "res/mipmap-mdpi-v4/ic_launcher.webp",
        "res/mipmap-xxxhdpi-v4/ic_launcher.webp",
        "res/mipmap-anydpi-v26/ic_launcher.xml",
    ];

    fn shell_table() -> Vec<u8> {
        zip::tests::file(SHELL_AAB, "base/resources.pb")
    }

    /// A table with package 0x7f, type 2, and entry 1 holding these values.
    fn table(values: &[Vec<u8>]) -> Vec<u8> {
        let id = |id: u64| encode(&[Field::varint(1, id)]);
        let mut entry = vec![Field::bytes(1, id(1))];
        entry.extend(
            values
                .iter()
                .map(|value| Field::bytes(6, encode(&[Field::bytes(2, value.clone())]))),
        );
        let kind = encode(&[Field::bytes(1, id(2)), Field::bytes(3, encode(&entry))]);
        let package = encode(&[Field::bytes(1, id(0x7f)), Field::bytes(3, kind)]);
        encode(&[Field::bytes(2, package)])
    }

    #[test]
    fn reads_the_shells_strings_and_files() {
        let table = ProtoResources::parse(&shell_table()).unwrap();

        assert_eq!(strings(&table, APP_NAME).unwrap(), ["OmniBuild Shell"]);
        assert_eq!(strings(&table, ICON).unwrap(), SHELL_ICON);
        assert_eq!(strings(&table, 0x7f04_0000).unwrap(), Vec::<String>::new());
        assert_eq!(table.to_bytes(), shell_table());
    }

    #[test]
    fn sets_the_shells_name() {
        let mut table = ProtoResources::parse(&shell_table()).unwrap();

        table.set_string(APP_NAME, "Repackaged").unwrap();

        let table = ProtoResources::parse(&table.to_bytes()).unwrap();
        assert_eq!(strings(&table, APP_NAME).unwrap(), ["Repackaged"]);
        assert_eq!(strings(&table, ICON).unwrap(), SHELL_ICON);
    }

    #[test]
    fn sets_every_configurations_value() {
        // A string, and a reference to another resource.
        let string = encode(&[Field::bytes(
            4,
            encode(&[Field::bytes(2, encode(&[Field::bytes(1, "Shell")]))]),
        )]);
        let reference = encode(&[Field::bytes(4, encode(&[Field::bytes(1, [])]))]);
        let mut table = ProtoResources::parse(&table(&[string, reference])).unwrap();

        table.set_string(0x7f02_0001, "Name").unwrap();

        assert_eq!(strings(&table, 0x7f02_0001).unwrap(), ["Name", "Name"]);
    }

    #[test]
    fn tells_files_strings_and_colours_apart() {
        let shell = ProtoResources::parse(&shell_table()).unwrap();
        let string = encode(&[Field::bytes(
            4,
            encode(&[Field::bytes(2, encode(&[Field::bytes(1, "Shell")]))]),
        )]);
        // A colour written as #2F6FEB, kept as ARGB, and a reference.
        let color = encode(&[Field::bytes(
            4,
            encode(&[Field::bytes(7, encode(&[Field::varint(10, 0xff2f_6feb)]))]),
        )]);
        let reference = encode(&[Field::bytes(4, encode(&[Field::bytes(1, [])]))]);
        let mut table = ProtoResources::parse(&table(&[string, color, reference])).unwrap();

        let values = table.values(0x7f02_0001).unwrap();
        let mut colored = ProtoResources::parse(&table.to_bytes()).unwrap();
        table.set_file(0x7f02_0001, "res/icon.png").unwrap();
        colored.set_color(0x7f02_0001, 0xff00_ff00).unwrap();

        assert_eq!(
            shell.values(0x7f01_0000).unwrap(),
            [Value::File(
                "res/drawable/ic_launcher_background.xml".to_owned()
            )]
        );
        assert_eq!(
            values,
            [
                Value::String("Shell".to_owned()),
                Value::Color(0xff2f_6feb),
                Value::Other,
            ]
        );
        assert_eq!(
            table.values(0x7f02_0001).unwrap(),
            vec![Value::File("res/icon.png".to_owned()); 3]
        );
        // A PNG, as aapt2 marks one.
        assert!(
            table
                .to_bytes()
                .windows(14)
                .any(|window| window == b"res/icon.png\x10\x01")
        );
        assert_eq!(
            colored.values(0x7f02_0001).unwrap(),
            vec![Value::Color(0xff00_ff00); 3]
        );
    }

    /// A `ConfigValue` for a language, perhaps with another qualifier too,
    /// and a value.
    fn config_value(tag: &str, landscape: bool, value: Vec<u8>) -> Vec<u8> {
        let mut config = vec![Field::bytes(3, tag)];
        if landscape {
            config.push(Field::varint(15, 2));
        }
        encode(&[Field::bytes(1, encode(&config)), Field::bytes(2, value)])
    }

    /// A `Value` that's a string.
    fn string_value(text: &str) -> Vec<u8> {
        encode(&[Field::bytes(
            4,
            encode(&[Field::bytes(2, encode(&[Field::bytes(1, text)]))]),
        )])
    }

    /// A table with package 0x7f, type 2, and entry 1 holding these
    /// `ConfigValue`s.
    fn table_of(config_values: &[Vec<u8>]) -> Vec<u8> {
        let id = |id: u64| encode(&[Field::varint(1, id)]);
        let mut entry = vec![Field::bytes(1, id(1))];
        entry.extend(
            config_values
                .iter()
                .map(|config_value| Field::bytes(6, config_value.clone())),
        );
        let kind = encode(&[Field::bytes(1, id(2)), Field::bytes(3, encode(&entry))]);
        let package = encode(&[Field::bytes(1, id(0x7f)), Field::bytes(3, kind)]);
        encode(&[Field::bytes(2, package)])
    }

    /// Each of the resource's values' language, if it has one, and its
    /// string.
    fn translations(table: &ProtoResources, id: u32) -> Vec<(Option<String>, String)> {
        let mut found = Vec::new();
        super::edit(
            &table.data,
            &[2, 3, 3, 6],
            &[id >> 24, (id >> 16) & 0xff, id & 0xffff],
            &mut |config_value| {
                let config = protobuf::message(config_value, 1);
                let item = protobuf::message(&protobuf::message(config_value, 2), 4);
                let string = protobuf::string(&protobuf::message(&item, 2), 1);
                found.push((protobuf::string(&config, 3), string.unwrap()));
                Ok(())
            },
        )
        .unwrap();
        found
    }

    #[test]
    fn translates_the_shells_name_into_languages_it_hasnt() {
        let mut table = ProtoResources::parse(&shell_table()).unwrap();

        table
            .translate(
                APP_NAME,
                &BTreeMap::from([("fr", "Marées"), ("zh-Hans", "潮汐")]),
            )
            .unwrap();

        let table = ProtoResources::parse(&table.to_bytes()).unwrap();
        // Each in a configuration with the language and nothing else, as
        // aapt2 writes one for values-fr.
        assert_eq!(
            translations(&table, APP_NAME),
            [
                (None, "OmniBuild Shell".to_owned()),
                (Some("fr".to_owned()), "Marées".to_owned()),
                (Some("zh-Hans".to_owned()), "潮汐".to_owned()),
            ]
        );
        assert!(
            table
                .to_bytes()
                .windows(6)
                .any(|window| window == b"\x0a\x04\x1a\x02fr")
        );
        assert_eq!(strings(&table, 0x7f03_0001).unwrap(), ["OmniBuild Shell"]);
    }

    #[test]
    fn translates_into_the_languages_the_table_has() {
        // The default value, French, French in Canada in landscape, and
        // German.
        let mut table = ProtoResources::parse(&table_of(&[
            encode(&[Field::bytes(2, string_value("Shell"))]),
            config_value("fr", false, string_value("Shell")),
            config_value("fr-CA", true, string_value("Shell")),
            config_value("de", false, string_value("Shell")),
        ]))
        .unwrap();

        table
            .translate(
                0x7f02_0001,
                &BTreeMap::from([("fr", "Nom"), ("es", "Nombre")]),
            )
            .unwrap();

        // French's own, and Canada's, the closest to it; German has no
        // translation, and Spanish one of its own, the only one added.
        let found = |tag: Option<&str>, text: &str| (tag.map(str::to_owned), text.to_owned());
        assert_eq!(
            translations(&table, 0x7f02_0001),
            [
                found(None, "Shell"),
                found(Some("fr"), "Nom"),
                found(Some("fr-CA"), "Nom"),
                found(Some("de"), "Shell"),
                found(Some("es"), "Nombre"),
            ]
        );
    }

    #[test]
    fn reports_a_translation_it_cant_make() {
        let compound = encode(&[Field::bytes(5, [])]);
        let names = BTreeMap::from([("fr", "Nom")]);

        for (data, id, message) in [
            (
                table_of(&[config_value("fr", false, compound)]),
                0x7f02_0001,
                "the resource holds several values, not one",
            ),
            (
                table_of(&[vec![0xff]]),
                0x7f02_0001,
                "it's corrupt or cut short",
            ),
            (table_of(&[]), 0x7f02_0002, "it has no resource 0x7f020002"),
        ] {
            let mut table = ProtoResources::parse(&data).unwrap();

            assert_eq!(
                table.translate(id, &names).unwrap_err().to_string(),
                message
            );
        }
    }

    #[test]
    fn reports_what_it_cant_set() {
        let compound = encode(&[Field::bytes(5, [])]);
        for (data, id, message) in [
            (
                table(&[compound]),
                0x7f02_0001,
                "resource 0x7f020001 holds several values, not one",
            ),
            (
                table(&[vec![0xff]]),
                0x7f02_0001,
                "it's corrupt or cut short",
            ),
            (table(&[]), 0x7f02_0002, "it has no resource 0x7f020002"),
        ] {
            let mut table = ProtoResources::parse(&data).unwrap();

            assert_eq!(
                table.set_string(id, "Name").unwrap_err().to_string(),
                message
            );
        }
        assert_eq!(
            ProtoResources::parse(&[0xff]).err().unwrap().to_string(),
            "it's corrupt or cut short"
        );
        let corrupt = ProtoResources::parse(&table(&[vec![0xff]])).unwrap();
        assert_eq!(
            corrupt.files().unwrap_err().to_string(),
            "it's corrupt or cut short"
        );
        assert_eq!(
            strings(&corrupt, 0x7f02_0001).unwrap_err().to_string(),
            "it's corrupt or cut short"
        );
    }
}
