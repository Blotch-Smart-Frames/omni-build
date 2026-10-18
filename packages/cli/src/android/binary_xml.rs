//! The binary `AndroidManifest.xml` in an APK: Android's binary XML
//! (`ResXMLTree` in AOSP's `ResourceTypes.h`), as aapt2 compiles it.
//!
//! Binary XML is a string pool, a resource map giving the resource ID of
//! each attribute name, and the nodes, whose attributes hold string pool
//! indexes or typed values. OmniBuild changes attribute values in place and
//! adds the strings they need to the end of the pool, so every other node,
//! index and string stays as aapt2 wrote it.

use anyhow::{Result, bail};

use super::chunk::chunks;
use super::manifest::{Attribute, Document, Value};
use super::string_pool::StringPool;
use crate::bytes::Reader;

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

pub struct BinaryXml {
    pool: StringPool,
    /// Every chunk after the string pool: the resource map and the nodes,
    /// which rewriting changes in place.
    nodes: Vec<u8>,
    attributes: Vec<Attribute>,
    /// Where each attribute's record starts in `nodes`.
    offsets: Vec<usize>,
}

impl BinaryXml {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut header = Reader::at(data, 0);
        let (kind, header_size, size) = (header.u16(), header.u16(), header.u32());
        if kind != XML || header_size != 8 || offset(size) != data.len() {
            bail!("it isn't binary XML");
        }
        let (pool, pool_size) = StringPool::parse(&data[8..])?;
        let nodes = data[8 + pool_size..].to_vec();

        let mut resource_map = Vec::new();
        let mut attributes = Vec::new();
        let mut offsets = Vec::new();
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
                        let mut record = Reader::at(&nodes, offset + 4);
                        let name = record.u32();
                        record.skip(7);
                        let value = match (record.u8(), record.u32()) {
                            (TYPE_STRING, index) => {
                                pool.get(index).map_or(Value::Other, Value::String)
                            }
                            (TYPE_INT_DEC | TYPE_INT_HEX, value) => Value::Int(value),
                            (TYPE_REFERENCE, id) => Value::Reference(id),
                            _ => Value::Other,
                        };
                        attributes.push(Attribute {
                            element: element.clone(),
                            name: pool.get(name).unwrap_or_default(),
                            resource_id: resource_map.get(self::offset(name)).copied(),
                            value,
                        });
                        offsets.push(offset);
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            pool,
            nodes,
            attributes,
            offsets,
        })
    }

    /// Sets an attribute's raw value (a string, or none) and typed value.
    fn set(&mut self, index: usize, raw: u32, data_type: u8, data: u32) {
        let offset = self.offsets[index];
        self.nodes[offset + 8..offset + 12].copy_from_slice(&raw.to_le_bytes());
        self.nodes[offset + 15] = data_type;
        self.nodes[offset + 16..offset + 20].copy_from_slice(&data.to_le_bytes());
    }
}

impl Document for BinaryXml {
    fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    fn set_string(&mut self, index: usize, value: &str) {
        let string = self.pool.add(value);
        self.set(index, string, TYPE_STRING, string);
        self.attributes[index].value = Value::String(value.to_owned());
    }

    fn set_int(&mut self, index: usize, value: u32) {
        self.set(index, NO_STRING, TYPE_INT_DEC, value);
        self.attributes[index].value = Value::Int(value);
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

fn offset(value: u32) -> usize {
    usize::try_from(value).expect("OmniBuild runs on 32 and 64-bit systems")
}

#[cfg(test)]
pub mod tests {
    use super::{BinaryXml, NO_STRING, TYPE_INT_DEC, TYPE_INT_HEX, TYPE_REFERENCE, TYPE_STRING};
    use crate::android::manifest::{Document, Value as Typed};
    use crate::android::string_pool::tests::empty;
    use crate::zip::{self, tests::SHELL};

    pub enum Value<'a> {
        Str(&'a str),
        Int(u32),
        Ref(u32),
    }

    pub type Element<'a> = (&'a str, Vec<(&'a str, Option<u32>, Value<'a>)>);

    /// Binary XML with these elements, one after another: rewriting
    /// doesn't look at how they nest. Attributes with a resource ID are
    /// android: ones.
    pub fn binary_xml(elements: &[Element]) -> Vec<u8> {
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
                    Value::Str(value) => {
                        let index = pool.add(value);
                        (index, TYPE_STRING, index)
                    }
                    Value::Int(value) => (NO_STRING, TYPE_INT_DEC, *value),
                    Value::Ref(id) => (NO_STRING, TYPE_REFERENCE, *id),
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

    pub fn shell_manifest() -> Vec<u8> {
        zip::read(SHELL).unwrap()[2].contents().unwrap()
    }

    #[test]
    fn reads_the_shells_manifest_and_writes_it_unchanged() {
        let manifest = shell_manifest();

        let xml = BinaryXml::parse(&manifest).unwrap();

        let attribute = |element: &str, name: &str| {
            xml.attributes()
                .iter()
                .find(|attribute| attribute.element == element && attribute.name == name)
                .unwrap()
        };
        assert_eq!(
            attribute("manifest", "package").value,
            Typed::String("dev.omnibuild.shell".to_owned())
        );
        assert_eq!(attribute("manifest", "package").resource_id, None);
        assert_eq!(attribute("manifest", "versionCode").value, Typed::Int(1));
        assert_eq!(
            attribute("manifest", "versionCode").resource_id,
            Some(0x0101_021b)
        );
        assert_eq!(
            attribute("application", "label").value,
            Typed::Reference(0x7f02_0000)
        );
        // android:exported is a boolean.
        assert_eq!(attribute("activity", "exported").value, Typed::Other);
        assert_eq!(xml.to_bytes(), manifest);
    }

    #[test]
    fn sets_strings_and_integers() {
        let mut xml = BinaryXml::parse(&binary_xml(&[(
            "manifest",
            vec![
                ("package", None, Value::Str("a.b")),
                ("versionCode", Some(0x0101_021b), Value::Ref(0x7f01_0000)),
            ],
        )]))
        .unwrap();

        xml.set_string(0, "dev.example.app");
        xml.set_int(1, 42);

        let read = BinaryXml::parse(&xml.to_bytes()).unwrap();
        let values: Vec<&Typed> = read
            .attributes()
            .iter()
            .map(|attribute| &attribute.value)
            .collect();
        assert_eq!(
            values,
            [
                &Typed::String("dev.example.app".to_owned()),
                &Typed::Int(42)
            ]
        );
        assert_eq!(xml.attributes()[1].value, Typed::Int(42));
    }

    #[test]
    fn reads_an_integer_in_hex() {
        let mut data = binary_xml(&[(
            "uses-sdk",
            vec![("minSdkVersion", Some(0x0101_020c), Value::Int(24))],
        )]);
        // The record's data type, 5 bytes before the end.
        let at = data.len() - 5;
        data[at] = TYPE_INT_HEX;

        let xml = BinaryXml::parse(&data).unwrap();

        assert_eq!(xml.attributes()[0].value, Typed::Int(24));
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
        let mut bad_attributes =
            binary_xml(&[("manifest", vec![("package", None, Value::Str("a.b"))])]);
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
                BinaryXml::parse(corrupt).err().unwrap().to_string(),
                message
            );
        }
    }
}
