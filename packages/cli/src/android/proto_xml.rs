//! The `AndroidManifest.xml` in an Android App Bundle: XML as aapt2's
//! protobuf messages (`XmlNode` in `Resources.proto`), which bundletool
//! compiles to binary XML for each APK it makes, as Google Play does.
//!
//! Reading walks the tree for its attributes, in document order. Writing
//! walks it again and changes the attributes edited since, by their place
//! in that order, so everything else is encoded as it was. Adding or
//! removing an element writes the tree and reads it again.

use anyhow::Result;

use super::manifest::{Attribute, Document, Value};
use super::protobuf::{self, Field, Wire};

// Field numbers in Resources.proto.
const NODE_ELEMENT: u32 = 1;
const ELEMENT_NAME: u32 = 3;
const ELEMENT_ATTRIBUTE: u32 = 4;
const ELEMENT_CHILD: u32 = 5;
const ATTRIBUTE_NAMESPACE: u32 = 1;
const ATTRIBUTE_NAME: u32 = 2;
const ATTRIBUTE_VALUE: u32 = 3;
const ATTRIBUTE_RESOURCE_ID: u32 = 5;
const ATTRIBUTE_COMPILED_ITEM: u32 = 6;
const ITEM_REFERENCE: u32 = 1;
const ITEM_PRIMITIVE: u32 = 7;
const REFERENCE_ID: u32 = 2;
const PRIMITIVE_INT_DECIMAL: u32 = 6;
const PRIMITIVE_INT_HEX: u32 = 7;
const ANDROID: &str = "http://schemas.android.com/apk/res/android";

enum Edit {
    String(String),
    Int(u32),
}

pub struct ProtoXml {
    data: Vec<u8>,
    attributes: Vec<Attribute>,
    edits: Vec<Option<Edit>>,
}

impl ProtoXml {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut attributes = Vec::new();
        walk(data, &mut |element, node, attribute| {
            attributes.push(read(element, node, attribute));
        })?;
        let edits = attributes.iter().map(|_| None).collect();
        Ok(Self {
            data: data.to_vec(),
            attributes,
            edits,
        })
    }
}

/// An attribute's name, resource ID and value. aapt2 keeps every value as
/// text, and compiles references and numbers into an `Item` too.
fn read(element: &str, node: usize, attribute: &[Field]) -> Attribute {
    let item = protobuf::message(attribute, ATTRIBUTE_COMPILED_ITEM);
    let primitive = protobuf::message(&item, ITEM_PRIMITIVE);
    let int = protobuf::uint(&primitive, PRIMITIVE_INT_DECIMAL)
        .or_else(|| protobuf::uint(&primitive, PRIMITIVE_INT_HEX))
        .and_then(|value| u32::try_from(value).ok());
    let value = if protobuf::bytes(&item, ITEM_REFERENCE).is_some() {
        let reference = protobuf::message(&item, ITEM_REFERENCE);
        Value::Reference(
            protobuf::uint(&reference, REFERENCE_ID)
                .and_then(|id| u32::try_from(id).ok())
                .unwrap_or(0),
        )
    } else if let Some(int) = int {
        Value::Int(int)
    } else if primitive.is_empty() {
        Value::String(protobuf::string(attribute, ATTRIBUTE_VALUE).unwrap_or_default())
    } else {
        // A boolean, a colour, a dimension...
        Value::Other
    };
    Attribute {
        element: element.to_owned(),
        node,
        name: protobuf::string(attribute, ATTRIBUTE_NAME).unwrap_or_default(),
        resource_id: protobuf::uint(attribute, ATTRIBUTE_RESOURCE_ID)
            .and_then(|id| u32::try_from(id).ok()),
        value,
    }
}

/// What `walk` calls with each attribute: its element's name and place in
/// document order, and the attribute.
type Visit<'a> = dyn FnMut(&str, usize, &mut Vec<Field>) + 'a;

/// Calls `visit` with each attribute in the node's tree, in document order.
/// Returns the node with the attributes as `visit` left them.
fn walk(node: &[u8], visit: &mut Visit) -> Result<Vec<u8>> {
    walk_node(node, &mut 0, visit)
}

/// `walk`, counting elements from `count`.
fn walk_node(node: &[u8], count: &mut usize, visit: &mut Visit) -> Result<Vec<u8>> {
    let mut fields = protobuf::decode(node)?;
    for field in &mut fields {
        if let (NODE_ELEMENT, Wire::Bytes(element)) = (field.number, &mut field.value) {
            *element = walk_element(element, count, visit)?;
        }
    }
    Ok(protobuf::encode(&fields))
}

fn walk_element(element: &[u8], count: &mut usize, visit: &mut Visit) -> Result<Vec<u8>> {
    let mut fields = protobuf::decode(element)?;
    let name = protobuf::string(&fields, ELEMENT_NAME).unwrap_or_default();
    let node = *count;
    *count += 1;
    for field in &mut fields {
        match (field.number, &mut field.value) {
            (ELEMENT_ATTRIBUTE, Wire::Bytes(attribute)) => {
                let mut attribute_fields = protobuf::decode(attribute)?;
                visit(&name, node, &mut attribute_fields);
                *attribute = protobuf::encode(&attribute_fields);
            }
            (ELEMENT_CHILD, Wire::Bytes(child)) => *child = walk_node(child, count, visit)?,
            _ => {}
        }
    }
    Ok(protobuf::encode(&fields))
}

/// Why a document OmniBuild wrote, from one it read, reads too.
const READ: &str = "the document was read once already";

/// How many elements a node's tree has, in a document that was read.
fn elements(node: &[u8]) -> usize {
    let mut count = 0;
    walk_node(node, &mut count, &mut |_, _, _| {}).expect(READ);
    count
}

/// A document that was read, with `change` made to its root element's
/// fields.
fn change_root(data: &[u8], change: &mut dyn FnMut(&mut Vec<Field>)) -> ProtoXml {
    let mut fields = protobuf::decode(data).expect(READ);
    for field in &mut fields {
        if let (NODE_ELEMENT, Wire::Bytes(element)) = (field.number, &mut field.value) {
            let mut element_fields = protobuf::decode(element).expect(READ);
            change(&mut element_fields);
            *element = protobuf::encode(&element_fields);
        }
    }
    ProtoXml::parse(&protobuf::encode(&fields)).expect(READ)
}

impl Document for ProtoXml {
    fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    fn set_string(&mut self, index: usize, value: &str) {
        self.edits[index] = Some(Edit::String(value.to_owned()));
        self.attributes[index].value = Value::String(value.to_owned());
    }

    fn set_int(&mut self, index: usize, value: u32) {
        self.edits[index] = Some(Edit::Int(value));
        self.attributes[index].value = Value::Int(value);
    }

    fn remove_children(&mut self, name: &str, remove: &dyn Fn(&[&Attribute]) -> bool) {
        *self = change_root(&self.to_bytes(), &mut |root| {
            // The root is element 0, and each child's elements follow it.
            let mut node = 1;
            let mut kept = Vec::with_capacity(root.len());
            for field in root.drain(..) {
                if let (ELEMENT_CHILD, Wire::Bytes(child)) = (field.number, &field.value) {
                    let element =
                        protobuf::message(&protobuf::decode(child).expect(READ), NODE_ELEMENT);
                    let attributes: Vec<&Attribute> = self
                        .attributes
                        .iter()
                        .filter(|attribute| attribute.node == node)
                        .collect();
                    node += elements(child);
                    if protobuf::string(&element, ELEMENT_NAME).as_deref() == Some(name)
                        && remove(&attributes)
                    {
                        continue;
                    }
                }
                kept.push(field);
            }
            *root = kept;
        });
    }

    fn add_child(
        &mut self,
        name: &str,
        (attribute, resource_id): (&str, u32),
        value: &str,
    ) -> Result<()> {
        // A plain string, as aapt2 writes one: no compiled item.
        let attribute = protobuf::encode(&[
            Field::bytes(ATTRIBUTE_NAMESPACE, ANDROID),
            Field::bytes(ATTRIBUTE_NAME, attribute),
            Field::bytes(ATTRIBUTE_VALUE, value),
            Field::varint(ATTRIBUTE_RESOURCE_ID, u64::from(resource_id)),
        ]);
        let element = protobuf::encode(&[
            Field::bytes(ELEMENT_NAME, name),
            Field::bytes(ELEMENT_ATTRIBUTE, attribute),
        ]);
        let child = protobuf::encode(&[Field::bytes(NODE_ELEMENT, element)]);
        *self = change_root(&self.to_bytes(), &mut |root| {
            root.push(Field::bytes(ELEMENT_CHILD, child.clone()));
        });
        Ok(())
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut index = 0;
        walk(&self.data, &mut |_, _, attribute| {
            match &self.edits[index] {
                // Plain text, as aapt2 writes a string attribute: no
                // compiled item.
                Some(Edit::String(value)) => {
                    protobuf::set(attribute, Field::bytes(ATTRIBUTE_VALUE, value.as_str()));
                    attribute.retain(|field| field.number != ATTRIBUTE_COMPILED_ITEM);
                }
                Some(Edit::Int(value)) => {
                    protobuf::set(attribute, Field::bytes(ATTRIBUTE_VALUE, value.to_string()));
                    let primitive = protobuf::encode(&[Field::varint(
                        PRIMITIVE_INT_DECIMAL,
                        u64::from(*value),
                    )]);
                    let item = protobuf::encode(&[Field::bytes(ITEM_PRIMITIVE, primitive)]);
                    protobuf::set(attribute, Field::bytes(ATTRIBUTE_COMPILED_ITEM, item));
                }
                None => {}
            }
            index += 1;
        })
        .expect("the manifest was read once already")
    }
}

#[cfg(test)]
pub mod tests {
    use super::ProtoXml;
    use crate::android::binary_xml::tests::{named, names, texts};
    use crate::android::manifest::{Document, Value};
    use crate::android::protobuf::{Field, encode};
    use crate::zip;

    pub const SHELL_AAB: &[u8] = include_bytes!("../../tests/fixtures/shell.aab");

    pub fn shell_manifest() -> Vec<u8> {
        zip::tests::file(SHELL_AAB, "base/manifest/AndroidManifest.xml")
    }

    fn attribute<'a>(xml: &'a ProtoXml, element: &str, name: &str) -> &'a Value {
        &xml.attributes()
            .iter()
            .find(|attribute| attribute.element == element && attribute.name == name)
            .unwrap()
            .value
    }

    #[test]
    fn removes_and_adds_the_roots_children() {
        let mut xml = ProtoXml::parse(&shell_manifest()).unwrap();
        let before = names(&xml, "activity");

        xml.remove_children("uses-permission", &named("android.permission.CAMERA"));
        // Only the root's children: the launcher activity's action stays.
        xml.remove_children("action", &|_| true);
        xml.add_child(
            "uses-permission",
            ("name", 0x0101_0003),
            "android.permission.VIBRATE",
        )
        .unwrap();

        let xml = ProtoXml::parse(&xml.to_bytes()).unwrap();
        assert_eq!(
            names(&xml, "uses-permission"),
            texts(&[
                "android.permission.INTERNET",
                "dev.omnibuild.shell.permission.SHELL",
                "android.permission.VIBRATE",
            ])
        );
        assert_eq!(
            names(&xml, "action"),
            texts(&["android.intent.action.MAIN"])
        );
        assert_eq!(names(&xml, "activity"), before);
        let added = xml.attributes().last().unwrap();
        assert_eq!(
            (added.element.as_str(), added.resource_id),
            ("uses-permission", Some(0x0101_0003))
        );
        // The manifest, uses-sdk, two permission requests, the permission,
        // the request for it, the application, the activity, its intent
        // filter, action and category, and the provider: then the new one.
        assert_eq!(added.node, 11);
    }

    #[test]
    fn reads_the_shells_manifest_and_writes_it_unchanged() {
        let manifest = shell_manifest();

        let xml = ProtoXml::parse(&manifest).unwrap();

        assert_eq!(
            attribute(&xml, "manifest", "package"),
            &Value::String("dev.omnibuild.shell".to_owned())
        );
        assert_eq!(attribute(&xml, "manifest", "versionCode"), &Value::Int(1));
        assert_eq!(
            attribute(&xml, "uses-sdk", "minSdkVersion"),
            &Value::Int(24)
        );
        assert_eq!(
            attribute(&xml, "application", "label"),
            &Value::Reference(0x7f03_0000)
        );
        assert_eq!(attribute(&xml, "activity", "exported"), &Value::Other);
        let label = xml
            .attributes()
            .iter()
            .find(|attribute| attribute.name == "label")
            .unwrap();
        assert_eq!(label.resource_id, Some(0x0101_0001));
        assert_eq!(xml.to_bytes(), manifest);
    }

    #[test]
    fn sets_strings_and_integers() {
        let manifest = shell_manifest();
        let mut xml = ProtoXml::parse(&manifest).unwrap();
        let index = |name: &str| {
            xml.attributes()
                .iter()
                .position(|attribute| attribute.name == name)
                .unwrap()
        };
        let (package, version_code, label) =
            (index("package"), index("versionCode"), index("label"));

        xml.set_string(package, "dev.example.app");
        xml.set_int(version_code, 42);
        xml.set_string(label, "Repackaged");

        let read = ProtoXml::parse(&xml.to_bytes()).unwrap();
        assert_eq!(
            attribute(&read, "manifest", "package"),
            &Value::String("dev.example.app".to_owned())
        );
        assert_eq!(attribute(&read, "manifest", "versionCode"), &Value::Int(42));
        // A string replaces the reference it was.
        assert_eq!(
            attribute(&read, "application", "label"),
            &Value::String("Repackaged".to_owned())
        );
        assert_eq!(attribute(&xml, "manifest", "versionCode"), &Value::Int(42));
    }

    #[test]
    fn reads_integers_in_hex_and_references_without_an_id() {
        let attribute = |name: &str, item: Vec<u8>| {
            Field::bytes(4, encode(&[Field::bytes(2, name), Field::bytes(6, item)]))
        };
        let hex = encode(&[Field::bytes(7, encode(&[Field::varint(7, 24)]))]);
        let reference = encode(&[Field::bytes(1, [])]);
        let element = encode(&[
            Field::bytes(3, "uses-sdk"),
            attribute("minSdkVersion", hex),
            attribute("theme", reference),
        ]);

        let xml = ProtoXml::parse(&encode(&[Field::bytes(1, element)])).unwrap();

        let values: Vec<&Value> = xml
            .attributes()
            .iter()
            .map(|attribute| &attribute.value)
            .collect();
        assert_eq!(values, [&Value::Int(24), &Value::Reference(0)]);
    }

    #[test]
    fn rejects_a_corrupt_manifest() {
        let corrupt = vec![0xff];
        let element = |fields: &[Field]| encode(&[Field::bytes(1, encode(fields))]);
        for data in [
            corrupt.clone(),
            encode(&[Field::bytes(1, corrupt.clone())]),
            element(&[Field::bytes(4, corrupt.clone())]),
            element(&[Field::bytes(5, corrupt)]),
        ] {
            assert_eq!(
                ProtoXml::parse(&data).err().unwrap().to_string(),
                "it's corrupt or cut short"
            );
        }
    }
}
