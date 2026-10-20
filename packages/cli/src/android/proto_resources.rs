//! The resource table in an Android App Bundle, `resources.pb`: aapt2's
//! `ResourceTable` message (`Resources.proto`), which bundletool compiles
//! into each APK's `resources.arsc`, as Google Play does.

use anyhow::{Result, bail};

use super::protobuf::{self, Field, Wire};
use super::resources::{Resources, Value};

// Field numbers in Resources.proto, along the path from the table to a
// value: a package, its types, their entries, each entry's value in each
// configuration.
const TABLE_PACKAGE: u32 = 2;
const PACKAGE_TYPE: u32 = 3;
const TYPE_ENTRY: u32 = 3;
const ENTRY_CONFIG_VALUE: u32 = 6;
const CONFIG_VALUE_VALUE: u32 = 2;
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
        let string = protobuf::encode(&[Field::bytes(STRING_VALUE, value)]);
        self.set_item(id, &protobuf::encode(&[Field::bytes(ITEM_STRING, string)]))
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

    fn to_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::ProtoResources;
    use crate::android::proto_xml::tests::SHELL_AAB;
    use crate::android::protobuf::{Field, encode};
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
