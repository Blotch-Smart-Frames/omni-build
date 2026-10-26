//! The binary `AndroidManifest.xml` in an APK: Android's binary XML
//! (`ResXMLTree` in AOSP's `ResourceTypes.h`), as aapt2 compiles it.
//!
//! Binary XML is a string pool, a resource map giving the resource ID of
//! each attribute name, and the nodes, whose attributes hold string pool
//! indexes or typed values. OmniBuild changes attribute values in place,
//! adds and removes whole elements, and adds the strings they need to the
//! end of the pool, so every other node and string stays as aapt2 wrote
//! it. The exception is an attribute the document doesn't name yet, such
//! as `android:scheme`: Android finds attributes by resource ID, which the
//! resource map gives the first strings in the pool, in order, so its name
//! goes after theirs, and the indexes of the strings after it move up one.
//! The other is an attribute added to an element, such as an activity's
//! `android:screenOrientation`, which goes among the element's others in
//! order of their resource IDs, as aapt2 sorts them and Android looks them
//! up.

use anyhow::{Context, Result, bail};

use super::chunk::{Chunk, chunks};
use super::manifest::{Attribute, Document, Element, Literal, Value};
use super::string_pool::StringPool;
use crate::bytes::Reader;

const XML: u16 = 0x0003;
const RESOURCE_MAP: u16 = 0x0180;
const START_NAMESPACE: u16 = 0x0100;
const END_NAMESPACE: u16 = 0x0101;
const START_ELEMENT: u16 = 0x0102;
const END_ELEMENT: u16 = 0x0103;
const CDATA: u16 = 0x0104;
/// A node's header: its type, header size and size, then its line number
/// and comment.
const NODE_HEADER: u16 = 16;
/// Where an element's attributes start after its header, and each one's
/// size.
const ATTRIBUTES_START: u16 = 20;
const ATTRIBUTE_SIZE: u16 = 20;
const ANDROID: &str = "http://schemas.android.com/apk/res/android";
/// A string pool index meaning "no string".
const NO_STRING: u32 = u32::MAX;
/// `Res_value` data types.
const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;
const TYPE_INT_BOOLEAN: u8 = 0x12;

pub struct BinaryXml {
    pool: StringPool,
    /// Every chunk after the string pool: the resource map and the nodes,
    /// which rewriting changes in place.
    nodes: Vec<u8>,
    /// The resource ID of each attribute name, by its index in the pool.
    resource_map: Vec<u32>,
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
        let mut node = 0;
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
                            (TYPE_INT_BOOLEAN, value) => Value::Boolean(value != 0),
                            _ => Value::Other,
                        };
                        attributes.push(Attribute {
                            element: element.clone(),
                            node,
                            name: pool.get(name).unwrap_or_default(),
                            resource_id: resource_map.get(self::offset(name)).copied(),
                            value,
                        });
                        offsets.push(offset);
                    }
                    node += 1;
                }
                _ => {}
            }
        }
        Ok(Self {
            pool,
            nodes,
            resource_map,
            attributes,
            offsets,
        })
    }

    /// The document with its nodes replaced, read again so its attributes
    /// and their places are those of the new nodes.
    fn with_nodes(&self, nodes: Vec<u8>) -> Result<Self> {
        Self::parse(
            &Self {
                nodes,
                pool: self.pool.clone(),
                resource_map: Vec::new(),
                attributes: Vec::new(),
                offsets: Vec::new(),
            }
            .to_bytes(),
        )
    }

    /// The element's name.
    fn element_name(&self, chunk: &Chunk) -> Option<String> {
        self.pool
            .get(Reader::at(&self.nodes, chunk.body() + 4).u32())
    }

    /// The pool index of the name of the android: attribute with resource
    /// ID `id`, adding `name` for it if the document has none: after the
    /// names the resource map gives IDs, with the ID at the end of the map,
    /// and the indexes of every string after it moved up one.
    fn attribute_name(&mut self, name: &str, id: u32) -> u32 {
        if let Some(index) = self.resource_map.iter().position(|&mapped| mapped == id) {
            return u32_of(index);
        }
        let index = u32_of(self.resource_map.len());
        self.pool.insert(index, name);
        shift_strings(&mut self.nodes, index);
        self.resource_map.push(id);
        let mut map = Vec::new();
        map.extend(RESOURCE_MAP.to_le_bytes());
        map.extend(8u16.to_le_bytes());
        map.extend(u32_of(8 + 4 * self.resource_map.len()).to_le_bytes());
        map.extend(self.resource_map.iter().flat_map(|id| id.to_le_bytes()));
        let read = "the document was read once already";
        let chunks = chunks(&self.nodes, 0, self.nodes.len()).expect(read);
        // Where aapt2 puts it, before the nodes, unless there's one.
        let old = chunks
            .iter()
            .find(|chunk| chunk.kind == RESOURCE_MAP)
            .map_or(0..0, |chunk| chunk.start..chunk.end());
        self.nodes.splice(old, map);
        index
    }

    /// Appends the nodes of `element` and the elements in it: its
    /// attributes in the android: namespace, whose URI is the pool's string
    /// `namespace`, in order of their resource IDs, as aapt2 writes them and
    /// Android looks them up, and each of their names in the resource map
    /// already. Each node has the line number `line`.
    fn encode(&mut self, element: &Element, namespace: u32, line: u32, out: &mut Vec<u8>) {
        let name = match self.pool.find(element.name) {
            Some(index) => index,
            None => self.pool.add(element.name),
        };
        let mut attributes = element.attributes.clone();
        attributes.sort_by_key(|&(_, id, _)| id);
        let start = out.len();
        node(out, START_ELEMENT, line, &[NO_STRING, name]);
        let count = u32_of(attributes.len());
        for field in [ATTRIBUTES_START, ATTRIBUTE_SIZE] {
            out.extend(field.to_le_bytes());
        }
        out.extend(
            u16::try_from(count)
                .expect("an element has few attributes")
                .to_le_bytes(),
        );
        out.extend([0; 6]);
        for (attribute, id, value) in attributes {
            let attribute_name = self.attribute_name(attribute, id);
            // As aapt2 compiles each: a string kept as its raw value too, and
            // true as every bit set, with no raw value.
            let (raw, data_type, data) = match value {
                Literal::Text(text) => {
                    let string = self.pool.add(text);
                    (string, TYPE_STRING, string)
                }
                Literal::Boolean(boolean) => (
                    NO_STRING,
                    TYPE_INT_BOOLEAN,
                    if boolean { u32::MAX } else { 0 },
                ),
            };
            for field in [namespace, attribute_name, raw] {
                out.extend(field.to_le_bytes());
            }
            out.extend([8, 0, 0, data_type]);
            out.extend(data.to_le_bytes());
        }
        let size = u32_of(out.len() - start);
        out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
        for child in &element.children {
            self.encode(child, namespace, line, out);
        }
        node(out, END_ELEMENT, line, &[NO_STRING, name]);
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

    fn remove_children(&mut self, name: &str, remove: &dyn Fn(&[&Attribute]) -> bool) {
        let mut kept = Vec::with_capacity(self.nodes.len());
        // How deep in the tree each element is (the root is 1), and the
        // depth of the element being left out, while one is.
        let (mut depth, mut node, mut removing) = (0, 0, None);
        let read = "the document was read once already";
        for chunk in chunks(&self.nodes, 0, self.nodes.len()).expect(read) {
            match chunk.kind {
                START_ELEMENT => {
                    depth += 1;
                    let attributes: Vec<&Attribute> = self
                        .attributes
                        .iter()
                        .filter(|attribute| attribute.node == node)
                        .collect();
                    if removing.is_none()
                        && depth == 2
                        && self.element_name(&chunk).as_deref() == Some(name)
                        && remove(&attributes)
                    {
                        removing = Some(depth);
                    }
                    node += 1;
                }
                END_ELEMENT => {
                    depth -= 1;
                    if removing == Some(depth + 1) {
                        removing = None;
                        continue;
                    }
                }
                _ => {}
            }
            if removing.is_none() {
                kept.extend(&self.nodes[chunk.start..chunk.end()]);
            }
        }
        *self = self.with_nodes(kept).expect(read);
    }

    fn add_element(&mut self, parent: usize, element: &Element) -> Result<()> {
        let read = "the document was read once already";
        // The names first, since adding one moves the strings after it, in
        // order of their IDs, as aapt2 lists them.
        let mut ids = Vec::new();
        attribute_ids(element, &mut ids);
        ids.sort_by_key(|&(_, id)| id);
        for (name, id) in ids {
            self.attribute_name(name, id);
        }
        let chunks = chunks(&self.nodes, 0, self.nodes.len()).expect(read);
        let parent = chunks
            .iter()
            .filter(|chunk| chunk.kind == START_ELEMENT)
            .nth(parent)
            .context("it has no element to add one to")?;
        let namespace = chunks
            .iter()
            .filter(|chunk| chunk.kind == START_NAMESPACE)
            .map(|chunk| Reader::at(&self.nodes, chunk.body() + 4).u32())
            .find(|&uri| self.pool.get(uri).as_deref() == Some(ANDROID))
            .context("it declares no android: namespace")?;
        let line = Reader::at(&self.nodes, parent.start + 8).u32();
        let at = parent.end();

        let mut nodes = Vec::new();
        self.encode(element, namespace, line, &mut nodes);
        let mut all = self.nodes.clone();
        // The parent's first child, right after its start.
        all.splice(at..at, nodes);
        *self = self.with_nodes(all).expect(read);
        Ok(())
    }

    fn set_enum(&mut self, node: usize, attribute: (&str, u32), value: (&str, i32)) {
        let (name, id) = attribute;
        // Its number, as aapt2 compiles an enum's value: aapt2 keeps no
        // text for one in binary XML.
        let data = value.1.cast_unsigned();
        if let Some(index) = self
            .attributes
            .iter()
            .position(|attribute| attribute.node == node && attribute.resource_id == Some(id))
        {
            self.set_int(index, data);
            return;
        }
        let read = "the document was read once already";
        let name = self.attribute_name(name, id);
        // Read again, for the attributes' places now that the strings and
        // the resource map have moved.
        let document = self.with_nodes(self.nodes.clone()).expect(read);
        // The namespace the document's other android: attributes are in.
        let namespace = document
            .attributes
            .iter()
            .zip(&document.offsets)
            .find(|(attribute, _)| attribute.resource_id.is_some())
            .map(|(_, &offset)| Reader::at(&document.nodes, offset).u32())
            .expect("the element is one an android: attribute was found on, or in");
        let chunks = chunks(&document.nodes, 0, document.nodes.len()).expect(read);
        let element = chunks
            .iter()
            .filter(|chunk| chunk.kind == START_ELEMENT)
            .nth(node)
            .expect("the element is the document's");
        let body = element.body();
        let mut fields = Reader::at(&document.nodes, body + 8);
        let (start, size, count) = (
            usize::from(fields.u16()),
            usize::from(fields.u16()),
            usize::from(fields.u16()),
        );
        // Before the first attribute with a greater resource ID, or none.
        let place = (0..count)
            .find(|&index| {
                let name = Reader::at(&document.nodes, body + start + index * size + 4).u32();
                document
                    .resource_map
                    .get(offset(name))
                    .is_none_or(|&other| other > id)
            })
            .unwrap_or(count);
        let mut record = Vec::new();
        for field in [namespace, name, NO_STRING] {
            record.extend(field.to_le_bytes());
        }
        record.extend([8, 0, 0, TYPE_INT_DEC]);
        record.extend(data.to_le_bytes());
        record.resize(size, 0);

        let mut nodes = document.nodes.clone();
        let at = body + start + place * size;
        nodes.splice(at..at, record);
        let element_size = u32_of(element.size + size);
        nodes[element.start + 4..element.start + 8].copy_from_slice(&element_size.to_le_bytes());
        // Its count of attributes, and the places, counted from 1, of its
        // android:id, class and style attributes, which move up one after
        // the new one.
        for (field, added) in [(12, true), (14, false), (16, false), (18, false)] {
            let at = body + field;
            let value = Reader::at(&nodes, at).u16();
            if added || usize::from(value) > place {
                nodes[at..at + 2].copy_from_slice(&(value + 1).to_le_bytes());
            }
        }
        *self = document.with_nodes(nodes).expect(read);
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

/// Appends a node: its header, with its size counted from what's appended
/// after it, which `fields` begins, as an element's start does with its
/// namespace and name.
fn node(out: &mut Vec<u8>, kind: u16, line: u32, fields: &[u32]) {
    let start = out.len();
    out.extend(kind.to_le_bytes());
    out.extend(NODE_HEADER.to_le_bytes());
    // The size, filled in below for an element's end, and for its start
    // once its attributes are there.
    out.extend([0; 4]);
    out.extend(line.to_le_bytes());
    out.extend(NO_STRING.to_le_bytes());
    for field in fields {
        out.extend(field.to_le_bytes());
    }
    let size = out.len() - start;
    out[start + 4..start + 8].copy_from_slice(&u32_of(size).to_le_bytes());
}

/// The android: attributes `element` and the elements in it have, by name
/// and resource ID.
fn attribute_ids<'a>(element: &Element<'a>, ids: &mut Vec<(&'a str, u32)>) {
    ids.extend(element.attributes.iter().map(|&(name, id, _)| (name, id)));
    for child in &element.children {
        attribute_ids(child, ids);
    }
}

/// Moves up one every index into the pool, in `nodes`, of a string from
/// `from` on, for a string inserted there: in each node's comment, a
/// namespace's prefix and URI, an element's namespace and name, and each
/// of its attributes' namespace, name, raw value and string value, and
/// text.
fn shift_strings(nodes: &mut [u8], from: u32) {
    let shift = |nodes: &mut [u8], at: usize| {
        let index = Reader::at(nodes, at).u32();
        if index != NO_STRING && index >= from {
            nodes[at..at + 4].copy_from_slice(&(index + 1).to_le_bytes());
        }
    };
    let read = "the document was read once already";
    for chunk in chunks(nodes, 0, nodes.len()).expect(read) {
        let body = chunk.body();
        let mut fields = vec![chunk.start + 12, body, body + 4];
        match chunk.kind {
            START_NAMESPACE | END_NAMESPACE | END_ELEMENT => {}
            START_ELEMENT => {
                let mut element = Reader::at(nodes, body + 8);
                let (start, size, count) = (
                    usize::from(element.u16()),
                    usize::from(element.u16()),
                    usize::from(element.u16()),
                );
                for attribute in (0..count).map(|index| body + start + index * size) {
                    fields.extend([attribute, attribute + 4, attribute + 8]);
                    if nodes[attribute + 15] == TYPE_STRING {
                        fields.push(attribute + 16);
                    }
                }
            }
            CDATA => {
                // The text, and its typed value.
                fields.truncate(2);
                if nodes[body + 7] == TYPE_STRING {
                    fields.push(body + 8);
                }
            }
            // The resource map's IDs aren't strings.
            _ => continue,
        }
        for field in fields {
            shift(nodes, field);
        }
    }
}

fn offset(value: u32) -> usize {
    usize::try_from(value).expect("OmniBuild runs on 32 and 64-bit systems")
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("binary XML is under 4 GiB")
}

#[cfg(test)]
pub mod tests {
    use super::{
        BinaryXml, CDATA, NO_STRING, START_ELEMENT, TYPE_INT_BOOLEAN, TYPE_INT_DEC, TYPE_INT_HEX,
        TYPE_REFERENCE, TYPE_STRING, chunks, node,
    };
    use crate::android::manifest::{
        Attribute, Document, Element as Added, Literal, Value as Typed,
    };
    use crate::android::string_pool::tests::empty;
    use crate::bytes::Reader;
    use crate::zip::{self, tests::SHELL};

    pub enum Value<'a> {
        Str(&'a str),
        Int(u32),
        Ref(u32),
        Bool(bool),
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
                    // aapt2 writes true as all ones.
                    Value::Bool(value) => (
                        NO_STRING,
                        TYPE_INT_BOOLEAN,
                        if *value { u32::MAX } else { 0 },
                    ),
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
        zip::tests::file(SHELL, "AndroidManifest.xml")
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
            Typed::Reference(0x7f03_0000)
        );
        // android:exported is a boolean.
        assert_eq!(
            attribute("activity", "exported").value,
            Typed::Boolean(true)
        );
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

    /// The android:name of each element named `element`, in order.
    pub fn names(xml: &dyn Document, element: &str) -> Vec<Typed> {
        xml.attributes()
            .iter()
            .filter(|attribute| attribute.element == element && attribute.name == "name")
            .map(|attribute| attribute.value.clone())
            .collect()
    }

    /// Strings, as attributes' values.
    pub fn texts(texts: &[&str]) -> Vec<Typed> {
        texts
            .iter()
            .map(|&text| Typed::String(text.to_owned()))
            .collect()
    }

    /// Whether an element's attributes include this android:name.
    pub fn named(name: &str) -> impl Fn(&[&Attribute]) -> bool {
        move |attributes| {
            attributes
                .iter()
                .any(|attribute| attribute.value == Typed::String(name.to_owned()))
        }
    }

    #[test]
    fn removes_and_adds_the_roots_children() {
        let mut xml = BinaryXml::parse(&shell_manifest()).unwrap();
        let before = names(&xml, "activity");

        xml.remove_children("uses-permission", &named("android.permission.CAMERA"));
        // Only the root's children: the launcher activity's action stays.
        xml.remove_children("action", &|_| true);
        xml.add_element(
            0,
            &Added::with(
                "uses-permission",
                ("name", 0x0101_0003),
                "android.permission.VIBRATE",
            ),
        )
        .unwrap();
        xml.add_element(
            0,
            &Added::with("feature", ("name", 0x0101_0003), "android.hardware.camera"),
        )
        .unwrap();

        let xml = BinaryXml::parse(&xml.to_bytes()).unwrap();
        assert_eq!(
            names(&xml, "uses-permission"),
            texts(&[
                "android.permission.VIBRATE",
                "android.permission.INTERNET",
                "dev.omnibuild.shell.permission.SHELL",
            ])
        );
        assert_eq!(names(&xml, "feature"), texts(&["android.hardware.camera"]));
        assert_eq!(
            names(&xml, "action"),
            texts(&["android.intent.action.MAIN"])
        );
        assert_eq!(names(&xml, "activity"), before);
        // The new elements are counted with the rest: the manifest, then
        // uses-sdk.
        let added = xml
            .attributes()
            .iter()
            .find(|attribute| attribute.element == "feature")
            .unwrap();
        assert_eq!(added.node, 1);
        assert_eq!(added.resource_id, Some(0x0101_0003));
    }

    /// The launcher activity's intent filter for URLs with the scheme
    /// `scheme`, with attributes the shell's manifest doesn't name: a
    /// boolean, false where an App Links filter's is true, and text.
    pub fn url_filter(scheme: &str) -> Added<'_> {
        Added {
            name: "intent-filter",
            attributes: vec![("autoVerify", 0x0101_04ee, Literal::Boolean(false))],
            children: vec![
                Added::with(
                    "action",
                    ("name", 0x0101_0003),
                    "android.intent.action.VIEW",
                ),
                Added {
                    name: "data",
                    // Two the manifest doesn't name, out of order.
                    attributes: vec![
                        ("host", 0x0101_0028, Literal::Text("report")),
                        ("scheme", 0x0101_0027, Literal::Text(scheme)),
                    ],
                    children: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn adds_elements_with_attributes_the_document_doesnt_name_yet() {
        let shell = BinaryXml::parse(&shell_manifest()).unwrap();
        let activity = shell
            .attributes()
            .iter()
            .find(|attribute| attribute.element == "activity")
            .unwrap()
            .node;
        let mut xml = BinaryXml::parse(&shell_manifest()).unwrap();

        xml.add_element(activity, &url_filter("myapp")).unwrap();

        let xml = BinaryXml::parse(&xml.to_bytes()).unwrap();
        // Every attribute the shell had is as it was, with its name and
        // resource ID, though its strings moved.
        let summary = |attribute: &Attribute| {
            (
                attribute.element.clone(),
                attribute.name.clone(),
                attribute.resource_id,
                attribute.value.clone(),
            )
        };
        let added: Vec<_> = xml
            .attributes()
            .iter()
            .filter(|attribute| attribute.node > activity && attribute.node <= activity + 3)
            .map(summary)
            .collect();
        assert_eq!(
            added,
            [
                (
                    "intent-filter".to_owned(),
                    "autoVerify".to_owned(),
                    Some(0x0101_04ee),
                    Typed::Boolean(false)
                ),
                (
                    "action".to_owned(),
                    "name".to_owned(),
                    Some(0x0101_0003),
                    Typed::String("android.intent.action.VIEW".to_owned())
                ),
                (
                    "data".to_owned(),
                    "scheme".to_owned(),
                    Some(0x0101_0027),
                    Typed::String("myapp".to_owned())
                ),
                (
                    "data".to_owned(),
                    "host".to_owned(),
                    Some(0x0101_0028),
                    Typed::String("report".to_owned())
                ),
            ]
        );
        let others: Vec<_> = xml
            .attributes()
            .iter()
            .filter(|attribute| attribute.node <= activity || attribute.node > activity + 3)
            .map(summary)
            .collect();
        let before: Vec<_> = shell.attributes().iter().map(summary).collect();
        assert_eq!(others, before);
        // The new names follow the resource map's, which gives their IDs.
        let names = xml.resource_map.len();
        assert_eq!(
            xml.resource_map[names - 3..],
            [0x0101_0027, 0x0101_0028, 0x0101_04ee]
        );
        assert_eq!(
            xml.pool.get(u32::try_from(names - 3).unwrap()).as_deref(),
            Some("scheme")
        );
    }

    #[test]
    fn moves_the_strings_of_text_and_of_a_document_without_a_resource_map() {
        let mut xml = BinaryXml::parse(&binary_xml(&[(
            "manifest",
            vec![("package", None, Value::Str("a.b"))],
        )]))
        .unwrap();
        // Without the resource map, as a document without android:
        // attributes may be; then text, its string and typed value the
        // element's name.
        xml.nodes.drain(..8);
        let name = xml.pool.find("manifest").unwrap();
        // Text whose typed value is a string, and text whose typed value
        // isn't, though its data is the same number.
        for data_type in [TYPE_STRING, TYPE_INT_DEC] {
            let mut text = Vec::new();
            node(&mut text, CDATA, 1, &[name]);
            text.extend([8, 0, 0, data_type]);
            text.extend(name.to_le_bytes());
            let size = u32::try_from(text.len()).unwrap();
            text[4..8].copy_from_slice(&size.to_le_bytes());
            xml.nodes.extend(&text);
        }

        let index = xml.attribute_name("scheme", 0x0101_0027);

        assert_eq!(index, 0);
        assert_eq!(xml.resource_map, [0x0101_0027]);
        let xml = BinaryXml::parse(&xml.to_bytes()).unwrap();
        assert_eq!(xml.pool.get(0).as_deref(), Some("scheme"));
        let end = xml.nodes.len();
        let at = |at: usize| u32::from_le_bytes(xml.nodes[at..at + 4].try_into().unwrap());
        // Each text node is 28 bytes, the string's index 12 from its end
        // and its typed value's data 4.
        assert_eq!((at(end - 40), at(end - 32)), (name + 1, name + 1));
        assert_eq!((at(end - 12), at(end - 4)), (name + 1, name));
        assert_eq!(xml.attributes()[0].value, Typed::String("a.b".to_owned()));
    }

    #[test]
    fn says_why_it_cant_add_an_element() {
        let name = |name| ("name", Some(0x0101_0003), Value::Str(name));
        let cases = [
            (binary_xml(&[]), "it has no element to add one to"),
            (
                binary_xml(&[("manifest", vec![name("a.b")])]),
                "it declares no android: namespace",
            ),
        ];

        for (data, message) in cases {
            let mut xml = BinaryXml::parse(&data).unwrap();

            assert_eq!(
                xml.add_element(
                    0,
                    &Added::with("uses-permission", ("name", 0x0101_0003), "x")
                )
                .unwrap_err()
                .to_string(),
                message
            );
        }
    }

    #[test]
    fn sets_an_enum_adding_it_among_the_elements_attributes() {
        let android = |name, id, value| (name, Some(id), value);
        let mut xml = BinaryXml::parse(&binary_xml(&[
            ("manifest", vec![("package", None, Value::Str("a.b"))]),
            (
                "activity",
                vec![
                    android("name", 0x0101_0003, Value::Str(".Main")),
                    android("configChanges", 0x0101_001f, Value::Int(0x80)),
                    ("tools", None, Value::Str("x")),
                ],
            ),
        ]))
        .unwrap();
        // Its android:id, class and style attributes, counted from 1, as if
        // they were its second, first and third.
        let start = chunks(&xml.nodes, 0, xml.nodes.len())
            .unwrap()
            .into_iter()
            .filter(|chunk| chunk.kind == START_ELEMENT)
            .nth(1)
            .unwrap()
            .body();
        for (field, index) in [(14, 2u16), (16, 1), (18, 3)] {
            xml.nodes[start + field..start + field + 2].copy_from_slice(&index.to_le_bytes());
        }
        let orientation = ("screenOrientation", 0x0101_001e);

        xml.set_enum(1, orientation, ("userLandscape", 11));
        let added = BinaryXml::parse(&xml.to_bytes()).unwrap();
        xml.set_enum(1, orientation, ("unspecified", -1));

        let activity: Vec<(&str, Option<u32>, &Typed)> = added
            .attributes()
            .iter()
            .filter(|attribute| attribute.node == 1)
            .map(|attribute| {
                (
                    attribute.name.as_str(),
                    attribute.resource_id,
                    &attribute.value,
                )
            })
            .collect();
        // In order of their resource IDs, and those without one last.
        assert_eq!(
            activity,
            [
                (
                    "name",
                    Some(0x0101_0003),
                    &Typed::String(".Main".to_owned())
                ),
                ("screenOrientation", Some(0x0101_001e), &Typed::Int(11)),
                ("configChanges", Some(0x0101_001f), &Typed::Int(0x80)),
                ("tools", None, &Typed::String("x".to_owned())),
            ]
        );
        let start = chunks(&added.nodes, 0, added.nodes.len())
            .unwrap()
            .into_iter()
            .filter(|chunk| chunk.kind == START_ELEMENT)
            .nth(1)
            .unwrap()
            .body();
        let fields: Vec<u16> = (12..20)
            .step_by(2)
            .map(|field| Reader::at(&added.nodes, start + field).u16())
            .collect();
        // Four attributes now, and those after the new one a place further.
        assert_eq!(fields, [4, 3, 1, 4]);
        // The value changes in place.
        let changed = BinaryXml::parse(&xml.to_bytes()).unwrap();
        assert_eq!(changed.attributes()[2].value, Typed::Int(u32::MAX));
        assert_eq!(xml.to_bytes().len(), added.to_bytes().len());
    }

    #[test]
    fn adds_an_enum_in_the_android_namespace() {
        let mut xml = BinaryXml::parse(&shell_manifest()).unwrap();
        let activity = xml
            .attributes()
            .iter()
            .find(|attribute| attribute.element == "activity")
            .unwrap()
            .node;

        xml.set_enum(
            activity,
            ("screenOrientation", 0x0101_001e),
            ("portrait", 1),
        );

        let namespace = |name: &str| {
            let index = xml
                .attributes()
                .iter()
                .position(|attribute| attribute.node == activity && attribute.name == name)
                .unwrap();
            Reader::at(&xml.nodes, xml.offsets[index]).u32()
        };
        assert_eq!(namespace("screenOrientation"), namespace("name"));
        assert_ne!(namespace("name"), NO_STRING);
    }

    #[test]
    fn reads_an_integer_in_hex_and_values_of_other_types() {
        let typed = |data_type: u8| {
            let mut data = binary_xml(&[(
                "uses-sdk",
                vec![("minSdkVersion", Some(0x0101_020c), Value::Int(24))],
            )]);
            // The record's data type, 5 bytes before the end.
            let at = data.len() - 5;
            data[at] = data_type;
            BinaryXml::parse(&data).unwrap().attributes()[0]
                .value
                .clone()
        };

        assert_eq!(typed(TYPE_INT_HEX), Typed::Int(24));
        // A dimension, which OmniBuild has no use for.
        assert_eq!(typed(0x05), Typed::Other);
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
