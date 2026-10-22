//! The resource table in an APK, `resources.arsc` (`ResTable` in AOSP's
//! `ResourceTypes.h`), as aapt2 compiles it.
//!
//! The table holds one string pool for every string value, then a package
//! per resource package: its type and key names, then for each type, a
//! chunk per configuration (language, density, ...) with an entry for each
//! resource that has a value in it. OmniBuild changes values in place and
//! adds the strings they need to the end of the pool, so every other
//! chunk, offset and string stays as aapt2 wrote it. A value in a language
//! the table has no configuration for gets a type chunk of its own.

use std::collections::BTreeMap;
use std::ops::Range;

use anyhow::{Result, bail};

use super::chunk::{Chunk, chunks};
use super::string_pool::StringPool;
use crate::bytes::Reader;
use crate::config::closest;

const TABLE: u16 = 0x0002;
const PACKAGE: u16 = 0x0200;
const TYPE: u16 = 0x0201;
const TYPE_SPEC: u16 = 0x0202;
/// The bit of a type spec's flags for an entry whose value differs from one
/// language to another (`ACONFIGURATION_LOCALE`).
const CONFIG_LOCALE: u32 = 0x0004;
/// Where a type chunk's configuration (`ResTable_config`) starts: after the
/// chunk's header, its type, flags, entry count and where its entries
/// start.
const CONFIG: usize = 20;
/// The size of a configuration as aapt2 writes one, which OmniBuild writes
/// too.
const CONFIG_SIZE: usize = 64;
/// Where a configuration keeps its language and region, packed, its
/// script, and whether aapt2 worked the script out from the language
/// rather than reading it from the resource's folder.
const LANGUAGE: usize = 8;
const REGION: usize = 10;
const SCRIPT: usize = 36;
const SCRIPT_COMPUTED: usize = 52;
/// A type chunk whose entry offsets are (entry index, offset / 4) pairs
/// for only the entries it has.
const SPARSE: u8 = 0x01;
/// A type chunk whose entry offsets are 16-bit, in units of 4 bytes.
const OFFSET16: u8 = 0x02;
/// An entry whose value is a map (a style, an array, a plural), not one
/// value.
const COMPLEX: u16 = 0x0001;
/// An entry packed into 8 bytes, its value's type in the top byte of its
/// flags.
const COMPACT: u16 = 0x0008;
const NO_ENTRY: u32 = u32::MAX;
const TYPE_STRING: u8 = 0x03;
/// A colour as `#AARRGGBB`, the first of the colour types: aapt2 keeps every
/// colour as 32-bit ARGB, however it was written, and says which way in
/// the type.
const TYPE_INT_COLOR_ARGB8: u8 = 0x1c;
const TYPE_INT_COLOR_RGB4: u8 = 0x1f;
/// Where an APK keeps the files its resource table names.
const FILES: &str = "res/";

/// A resource's value in one configuration, as far as OmniBuild tells them
/// apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A file in the package, by its path: a bitmap, or XML such as a
    /// vector drawable or an adaptive icon.
    File(String),
    String(String),
    /// A colour, as ARGB.
    Color(u32),
    Other,
}

/// A resource table whose values can be read and changed, in either
/// format: an APK's `resources.arsc` or an AAB's `resources.pb`.
pub trait Resources {
    /// The resource's value in each configuration it has one in.
    fn values(&self, id: u32) -> Result<Vec<Value>>;
    /// Every file the table names, in any resource.
    fn files(&self) -> Result<Vec<String>>;
    /// Makes a resource a string in every configuration it has a value in.
    fn set_string(&mut self, id: u32, value: &str) -> Result<()>;
    /// Makes a resource the file at `path`, a PNG, in every configuration
    /// it has a value in.
    fn set_file(&mut self, id: u32, path: &str) -> Result<()>;
    /// Makes a resource a colour, as ARGB, in every configuration it has a
    /// value in.
    fn set_color(&mut self, id: u32, argb: u32) -> Result<()>;
    /// Makes a string resource `names`' values, by language tag, in other
    /// languages: in each configuration with a language, the value that
    /// fits it best, if any, and in a configuration of its own, a language
    /// and nothing else, each value of a language that has none.
    fn translate(&mut self, id: u32, names: &BTreeMap<&str, &str>) -> Result<()>;
    fn to_bytes(&self) -> Vec<u8>;
}

pub struct ResourceTable {
    package_count: u32,
    pool: StringPool,
    /// Everything after the string pool: the packages, which rewriting
    /// changes in place.
    packages: Vec<u8>,
}

/// Where a resource's value is in `packages`, in one configuration: its
/// type, and its data (a string pool index, for a string), and where its
/// configuration is. And its entry's key, its name's index in the
/// package's key strings.
struct Slot {
    data_type: usize,
    data: usize,
    config: Range<usize>,
    key: u32,
}

/// What a type chunk has for an entry.
enum Found {
    Nothing,
    /// A map of values (a style, an array, a plural), which OmniBuild leaves
    /// as it is.
    Map,
    Value(Slot),
}

impl ResourceTable {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut header = Reader::at(data, 0);
        let (kind, header_size, size, package_count) =
            (header.u16(), header.u16(), header.u32(), header.u32());
        if kind != TABLE || header_size != 12 || offset(size) != data.len() {
            bail!("it isn't a resource table");
        }
        let (pool, pool_size) = StringPool::parse(&data[12..])?;
        Ok(Self {
            package_count,
            pool,
            packages: data[12 + pool_size..].to_vec(),
        })
    }

    /// Where the resource's value is in each configuration it has one in.
    /// A resource ID is its package's ID, its type's, and its entry's index
    /// in the type: 0xPPTTEEEE.
    fn slots(&self, id: u32) -> Result<Vec<Slot>> {
        let (package_id, type_id, index) = (id >> 24, (id >> 16) & 0xff, offset(id & 0xffff));
        let mut slots = Vec::new();
        for package in chunks(&self.packages, 0, self.packages.len())? {
            if package.kind != PACKAGE
                || Reader::at(&self.packages, package.start + 8).u32() != package_id
            {
                continue;
            }
            for chunk in chunks(&self.packages, package.body(), package.end())? {
                if chunk.kind == TYPE && u32::from(self.packages[chunk.start + 8]) == type_id {
                    match self.slot(&chunk, index)? {
                        Found::Nothing => {}
                        Found::Map => bail!("resource {id:#010x} holds several values, not one"),
                        Found::Value(slot) => slots.push(slot),
                    }
                }
            }
        }
        Ok(slots)
    }

    /// Where every value in the table is, but those in maps.
    fn all_slots(&self) -> Result<Vec<Slot>> {
        let mut slots = Vec::new();
        let packages = chunks(&self.packages, 0, self.packages.len())?;
        for package in packages.iter().filter(|chunk| chunk.kind == PACKAGE) {
            let types = chunks(&self.packages, package.body(), package.end())?;
            for chunk in types.iter().filter(|chunk| chunk.kind == TYPE) {
                let (flags, count, _) = self.type_header(chunk)?;
                for index in 0..count {
                    // A sparse chunk lists its entries' indexes.
                    let index = if flags & SPARSE == 0 {
                        index
                    } else {
                        usize::from(Reader::at(&self.packages, chunk.body() + 4 * index).u16())
                    };
                    if let Found::Value(slot) = self.slot(chunk, index)? {
                        slots.push(slot);
                    }
                }
            }
        }
        Ok(slots)
    }

    /// A type chunk's flags, its number of entries, and where they start.
    fn type_header(&self, chunk: &Chunk) -> Result<(u8, usize, usize)> {
        let mut header = Reader::at(&self.packages, chunk.start + 9);
        let flags = header.u8();
        header.skip(2);
        let (count, entries) = (offset(header.u32()), offset(header.u32()));
        let width = if flags & OFFSET16 == 0 { 4 } else { 2 };
        if entries > chunk.size
            || chunk
                .header_size
                .saturating_add(count.saturating_mul(width))
                > entries
        {
            bail!("it's corrupt or cut short");
        }
        Ok((flags, count, entries))
    }

    /// Where the value of the entry at `index` is in a type chunk, if the
    /// chunk has one.
    fn slot(&self, chunk: &Chunk, index: usize) -> Result<Found> {
        let data = &self.packages;
        let (flags, count, entries) = self.type_header(chunk)?;
        let offsets = chunk.body();
        let entry = if flags & SPARSE != 0 {
            (0..count).find_map(|pair| {
                let mut pair = Reader::at(data, offsets + 4 * pair);
                (usize::from(pair.u16()) == index).then(|| 4 * usize::from(pair.u16()))
            })
        } else if index >= count {
            None
        } else if flags & OFFSET16 != 0 {
            let offset = Reader::at(data, offsets + 2 * index).u16();
            (offset != u16::MAX).then(|| 4 * usize::from(offset))
        } else {
            let offset = Reader::at(data, offsets + 4 * index).u32();
            (offset != NO_ENTRY).then(|| self::offset(offset))
        };
        let Some(entry) = entry else {
            return Ok(Found::Nothing);
        };

        let at = chunk.start + entries + entry;
        let mut header = Reader::at(data, at);
        let (size, flags, key) = (usize::from(header.u16()), header.u16(), header.u32());
        let config = chunk.start + CONFIG..chunk.body();
        // A compact entry keeps its key where another keeps its size, its
        // value's type in the top byte of its flags, and its data after
        // them. Any other has a Res_value after its header: size, a zero
        // byte, the type, and the data.
        let slot = if flags & COMPACT == 0 {
            Slot {
                data_type: at + size + 3,
                data: at + size + 4,
                config,
                key,
            }
        } else {
            Slot {
                data_type: at + 3,
                data: at + 4,
                config,
                key: u32::try_from(size).expect("a u16 fits"),
            }
        };
        if flags & (COMPLEX | COMPACT) == COMPLEX {
            return Ok(Found::Map);
        }
        if slot.data + 4 > chunk.end() {
            bail!("it's corrupt or cut short");
        }
        Ok(Found::Value(slot))
    }

    /// Adds a type chunk for the language `tag`, with one entry, the
    /// resource's, named by `key`, a string at `index` in the pool, after
    /// the type's last chunk. Its type spec says the entry's value differs
    /// from one language to another.
    fn add_translation(&mut self, id: u32, key: u32, tag: &str, index: u32) {
        let (package_id, type_id, entry) = (id >> 24, (id >> 16) & 0xff, offset(id & 0xffff));
        // Read whole already, to find the resource's values.
        let packages = chunks(&self.packages, 0, self.packages.len()).expect("they were read");
        let package = packages
            .iter()
            .find(|chunk| {
                chunk.kind == PACKAGE
                    && Reader::at(&self.packages, chunk.start + 8).u32() == package_id
            })
            .expect("the resource has a value, so its package is there");
        let types = chunks(&self.packages, package.body(), package.end()).expect("they were read");
        let of_type = |chunk: &&Chunk| u32::from(self.packages[chunk.start + 8]) == type_id;
        let spec = types
            .iter()
            .filter(|chunk| chunk.kind == TYPE_SPEC)
            .find(of_type);
        let last = types
            .iter()
            .filter(|chunk| chunk.kind == TYPE)
            .rfind(of_type)
            .expect("the resource has a value, so its type has a chunk");
        // As many entries as the type has, all but this one empty.
        let count = spec.map_or(0, |spec| {
            offset(Reader::at(&self.packages, spec.start + 12).u32())
        });
        let count = count.max(entry + 1);
        let header_size = CONFIG + CONFIG_SIZE;
        let entries = header_size + 4 * count;
        let mut offsets = vec![0xff; 4 * count];
        offsets[4 * entry..4 * entry + 4].copy_from_slice(&0u32.to_le_bytes());
        let chunk = [
            &TYPE.to_le_bytes()[..],
            &u16::try_from(header_size)
                .expect("the header fits")
                .to_le_bytes(),
            &u32::try_from(entries + 16)
                .expect("a type's entries fit")
                .to_le_bytes(),
            &[u8::try_from(type_id).expect("a type ID is a byte"), 0, 0, 0],
            &u32::try_from(count)
                .expect("a type's entries fit")
                .to_le_bytes(),
            &u32::try_from(entries)
                .expect("a type's entries fit")
                .to_le_bytes(),
            &locale_config(tag),
            &offsets,
            // The entry, and its value: a string.
            &8u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &key.to_le_bytes(),
            &8u16.to_le_bytes(),
            &[0, TYPE_STRING],
            &index.to_le_bytes(),
        ]
        .concat();
        let (package_start, package_size, insert) = (package.start, package.size, last.end());
        let spec = spec.map(|spec| spec.start);
        self.packages.splice(insert..insert, chunk.iter().copied());
        let package_size = u32::try_from(package_size + chunk.len()).expect("a package fits");
        self.packages[package_start + 4..package_start + 8]
            .copy_from_slice(&package_size.to_le_bytes());
        if let Some(spec) = spec {
            // Its type chunks, counted since Android 14.
            let mut header = Reader::at(&self.packages, spec + 10);
            let (types, count) = (header.u16(), offset(header.u32()));
            let types = types + u16::from(types != 0);
            self.packages[spec + 10..spec + 12].copy_from_slice(&types.to_le_bytes());
            let flags = spec + 16 + 4 * entry;
            if entry < count {
                let value = Reader::at(&self.packages, flags).u32() | CONFIG_LOCALE;
                self.packages[flags..flags + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    /// The file a value names, if it's one: a string in `res/`.
    fn file(&self, slot: &Slot) -> Option<String> {
        let data = Reader::at(&self.packages, slot.data).u32();
        (self.packages[slot.data_type] == TYPE_STRING)
            .then(|| self.pool.get(data))
            .flatten()
            .filter(|path| path.starts_with(FILES))
    }
}

impl ResourceTable {
    /// Gives the resource this type and data in every configuration it has
    /// a value in.
    fn set(&mut self, id: u32, data_type: u8, data: u32) -> Result<()> {
        let slots = self.slots(id)?;
        if slots.is_empty() {
            bail!("it has no resource {id:#010x}");
        }
        for slot in slots {
            self.packages[slot.data_type] = data_type;
            self.packages[slot.data..slot.data + 4].copy_from_slice(&data.to_le_bytes());
        }
        Ok(())
    }
}

impl Resources for ResourceTable {
    /// The resource's values. A file's is its path, a string like any
    /// other, which is how a file's is told from another string: an APK
    /// keeps its resources' files in `res/`.
    fn values(&self, id: u32) -> Result<Vec<Value>> {
        Ok(self
            .slots(id)?
            .into_iter()
            .map(|slot| {
                let data = Reader::at(&self.packages, slot.data).u32();
                match (self.file(&slot), self.packages[slot.data_type]) {
                    (Some(path), _) => Value::File(path),
                    (None, TYPE_STRING) => self.pool.get(data).map_or(Value::Other, Value::String),
                    (None, TYPE_INT_COLOR_ARGB8..=TYPE_INT_COLOR_RGB4) => Value::Color(data),
                    _ => Value::Other,
                }
            })
            .collect())
    }

    fn files(&self) -> Result<Vec<String>> {
        Ok(self
            .all_slots()?
            .iter()
            .filter_map(|slot| self.file(slot))
            .collect())
    }

    fn set_string(&mut self, id: u32, value: &str) -> Result<()> {
        // Added only once it's known the resource is there.
        self.slots(id)?;
        let index = self.pool.add(value);
        self.set(id, TYPE_STRING, index)
    }

    fn set_file(&mut self, id: u32, path: &str) -> Result<()> {
        self.set_string(id, path)
    }

    fn set_color(&mut self, id: u32, argb: u32) -> Result<()> {
        self.set(id, TYPE_INT_COLOR_ARGB8, argb)
    }

    fn translate(&mut self, id: u32, names: &BTreeMap<&str, &str>) -> Result<()> {
        let slots = self.slots(id)?;
        let Some(key) = slots.first().map(|slot| slot.key) else {
            bail!("it has no resource {id:#010x}");
        };
        // The languages with a configuration of their own already.
        let mut plain = Vec::new();
        for slot in slots {
            let Some((tag, only)) = locale(&self.packages[slot.config]) else {
                continue;
            };
            if let Some(name) = closest(names, &tag) {
                let index = self.pool.add(name);
                self.packages[slot.data_type] = TYPE_STRING;
                self.packages[slot.data..slot.data + 4].copy_from_slice(&index.to_le_bytes());
            }
            if only {
                plain.push(tag);
            }
        }
        for (&tag, &name) in names {
            if !plain.iter().any(|plain| plain == tag) {
                let index = self.pool.add(name);
                self.add_translation(id, key, tag, index);
            }
        }
        Ok(())
    }

    fn to_bytes(&self) -> Vec<u8> {
        let pool = self.pool.to_bytes();
        let size = u32::try_from(12 + pool.len() + self.packages.len())
            .expect("resource tables are under 4 GiB");
        [
            &TABLE.to_le_bytes()[..],
            &12u16.to_le_bytes(),
            &size.to_le_bytes(),
            &self.package_count.to_le_bytes(),
            &pool,
            &self.packages,
        ]
        .concat()
    }
}

fn offset(value: u32) -> usize {
    usize::try_from(value).expect("OmniBuild runs on 32 and 64-bit systems")
}

/// The language tag a configuration is for, such as `pt-BR`, and whether
/// a language is all it's for; or none, for one that's for every language.
fn locale(config: &[u8]) -> Option<(String, bool)> {
    let byte = |at: usize| config.get(at).copied().unwrap_or_default();
    let mut tag = unpack(byte(LANGUAGE), byte(LANGUAGE + 1), b'a');
    if tag.is_empty() {
        return None;
    }
    // A script aapt2 worked out from the language isn't in the tag the
    // resource's folder gave.
    if byte(SCRIPT) != 0 && byte(SCRIPT_COMPUTED) == 0 {
        tag.push('-');
        tag.extend((SCRIPT..SCRIPT + 4).map(|at| char::from(byte(at))));
    }
    let region = unpack(byte(REGION), byte(REGION + 1), b'0');
    if !region.is_empty() {
        tag.push('-');
        tag.push_str(&region);
    }
    // Past its size, every other qualifier is nothing: a density, a
    // variant of the language, an Android version.
    let only = config.iter().enumerate().skip(4).all(|(at, &value)| {
        value == 0 || matches!(at, LANGUAGE..=11 | SCRIPT..=39 | SCRIPT_COMPUTED)
    });
    Some((tag, only))
}

/// A language or region as a configuration packs it into two bytes: two
/// letters as they are, and three letters or digits five bits each, as
/// their distance from `base`, with the top bit set.
fn unpack(first: u8, second: u8, base: u8) -> String {
    if first & 0x80 == 0 {
        [first, second]
            .into_iter()
            .filter(|&byte| byte != 0)
            .map(char::from)
            .collect()
    } else {
        [
            second & 0x1f,
            ((second & 0xe0) >> 5) | ((first & 0x03) << 3),
            (first & 0x7c) >> 2,
        ]
        .into_iter()
        .map(|value| char::from(base + value))
        .collect()
    }
}

fn pack(subtag: &str, base: u8) -> [u8; 2] {
    if let [first, second, third] = *subtag.as_bytes() {
        let [first, second, third] = [first, second, third].map(|byte| byte.wrapping_sub(base));
        [0x80 | (third << 2) | (second >> 3), (second << 5) | first]
    } else {
        // Two letters, as they are: the language tags OmniBuild takes have
        // two or three.
        let mut packed = [0; 2];
        packed
            .iter_mut()
            .zip(subtag.bytes())
            .for_each(|(to, from)| *to = from);
        packed
    }
}

/// A configuration for the language `tag` and nothing else, as aapt2
/// writes one for a resource's folder, such as `values-b+zh+Hans`. A
/// script the tag doesn't give is left for Android to work out, as it
/// does for tables aapt2 wrote before it wrote scripts.
fn locale_config(tag: &str) -> Vec<u8> {
    let mut config = vec![0; CONFIG_SIZE];
    config[..4].copy_from_slice(&u32::try_from(CONFIG_SIZE).expect("it fits").to_le_bytes());
    let mut subtags = tag.split('-');
    let language = pack(subtags.next().unwrap_or_default(), b'a');
    config[LANGUAGE..LANGUAGE + 2].copy_from_slice(&language);
    for subtag in subtags {
        if subtag.len() == 4 {
            config[SCRIPT..SCRIPT + 4].copy_from_slice(subtag.as_bytes());
        } else {
            config[REGION..REGION + 2].copy_from_slice(&pack(subtag, b'0'));
        }
    }
    config
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;

    use super::{
        COMPACT, COMPLEX, CONFIG_LOCALE, OFFSET16, ResourceTable, Resources, SPARSE,
        TYPE_INT_COLOR_ARGB8, TYPE_SPEC, TYPE_STRING, Value, locale, locale_config,
    };
    use crate::android::chunk::chunks;
    use crate::android::string_pool::tests::empty;
    use crate::bytes::Reader;
    use crate::zip::{self, tests::SHELL};

    const APP_NAME: u32 = 0x7f03_0000;

    fn shell_table() -> Vec<u8> {
        zip::tests::file(SHELL, "resources.arsc")
    }

    /// Each value the resource has: its type and string, if it's one.
    /// The resource's string values, one for each configuration it has a
    /// string in. A file resource's value is its path in the package.
    pub fn strings(table: &dyn Resources, id: u32) -> anyhow::Result<Vec<String>> {
        Ok(table
            .values(id)?
            .into_iter()
            .filter_map(|value| match value {
                Value::File(text) | Value::String(text) => Some(text),
                Value::Color(_) | Value::Other => None,
            })
            .collect())
    }

    fn values_with_types(table: &ResourceTable, id: u32) -> Vec<(u8, Option<String>)> {
        table
            .slots(id)
            .unwrap()
            .iter()
            .map(|slot| {
                let data = u32::from_le_bytes(
                    table.packages[slot.data..slot.data + 4].try_into().unwrap(),
                );
                (table.packages[slot.data_type], table.pool.get(data))
            })
            .collect()
    }

    /// An entry with a `Res_value`.
    fn entry(flags: u16, data_type: u8, data: u32) -> Vec<u8> {
        [
            &8u16.to_le_bytes()[..],
            &flags.to_le_bytes(),
            &0u32.to_le_bytes(),
            &8u16.to_le_bytes(),
            &[0, data_type],
            &data.to_le_bytes(),
        ]
        .concat()
    }

    /// A type chunk for type 2 with these flags, entry count, entry offsets
    /// and entries. Its configuration is the default one, written as just
    /// its size.
    fn type_chunk(flags: u8, count: u32, offsets: &[u8], entries: &[u8]) -> Vec<u8> {
        type_chunk_for(&4u32.to_le_bytes(), flags, count, offsets, entries)
    }

    /// The same, for this configuration.
    fn type_chunk_for(
        config: &[u8],
        flags: u8,
        count: u32,
        offsets: &[u8],
        entries: &[u8],
    ) -> Vec<u8> {
        let header_size = 20 + config.len();
        let entries_start = header_size + offsets.len();
        let size = u32::try_from(entries_start + entries.len()).unwrap();
        [
            &0x0201u16.to_le_bytes()[..],
            &u16::try_from(header_size).unwrap().to_le_bytes(),
            &size.to_le_bytes(),
            &[2, flags, 0, 0],
            &count.to_le_bytes(),
            &u32::try_from(entries_start).unwrap().to_le_bytes(),
            config,
            offsets,
            entries,
        ]
        .concat()
    }

    /// A type spec for type 2, with these flags for its entries, and this
    /// count of type chunks, which aapt2 has written since Android 14.
    fn type_spec(types: u16, flags: &[u32]) -> Vec<u8> {
        let size = u32::try_from(16 + 4 * flags.len()).unwrap();
        [
            &TYPE_SPEC.to_le_bytes()[..],
            &16u16.to_le_bytes(),
            &size.to_le_bytes(),
            &[2, 0],
            &types.to_le_bytes(),
            &u32::try_from(flags.len()).unwrap().to_le_bytes(),
            &flags
                .iter()
                .flat_map(|flags| flags.to_le_bytes())
                .collect::<Vec<_>>(),
        ]
        .concat()
    }

    /// Each configuration's language the resource has a value in, if it
    /// has one, and the value.
    fn translations(table: &ResourceTable, id: u32) -> Vec<(Option<String>, Option<String>)> {
        table
            .slots(id)
            .unwrap()
            .into_iter()
            .map(|slot| {
                let data = Reader::at(&table.packages, slot.data).u32();
                (
                    locale(&table.packages[slot.config]).map(|(tag, _)| tag),
                    table.pool.get(data),
                )
            })
            .collect()
    }

    /// The type spec's flags for each entry, and its count of type chunks.
    fn spec_of(table: &ResourceTable, package: usize) -> (Vec<u32>, u16) {
        let packages = chunks(&table.packages, 0, table.packages.len()).unwrap();
        let package = &packages[package];
        let types = chunks(&table.packages, package.body(), package.end()).unwrap();
        let spec = types.iter().find(|chunk| chunk.kind == TYPE_SPEC).unwrap();
        let mut header = Reader::at(&table.packages, spec.start + 10);
        let (count, entries) = (header.u16(), header.u32());
        let flags = (0..entries).map(|_| header.u32()).collect();
        (flags, count)
    }

    fn names<'a>(names: &[(&'a str, &'a str)]) -> BTreeMap<&'a str, &'a str> {
        names.iter().copied().collect()
    }

    #[test]
    fn translates_the_shells_name_into_languages_it_hasnt() {
        let mut table = ResourceTable::parse(&shell_table()).unwrap();

        table
            .translate(
                APP_NAME,
                &names(&[
                    ("fil", "Taog"),
                    ("fr", "Marées"),
                    ("es-419", "Mareas"),
                    ("zh-Hans", "潮汐"),
                ]),
            )
            .unwrap();

        // Read back from what it writes, with a type chunk for each.
        let table = ResourceTable::parse(&table.to_bytes()).unwrap();
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(
            translations(&table, APP_NAME),
            [
                (None, some("OmniBuild Shell")),
                (some("es-419"), some("Mareas")),
                (some("fil"), some("Taog")),
                (some("fr"), some("Marées")),
                (some("zh-Hans"), some("潮汐")),
            ]
        );
        // The launcher's name, the next entry, has none; the icon's files
        // are as they were.
        assert_eq!(strings(&table, 0x7f03_0001).unwrap(), ["OmniBuild Shell"]);
        assert_eq!(
            strings(&table, 0x7f02_0000).unwrap(),
            ["res/d2.webp", "res/sK.webp", "res/BW.xml"]
        );
        assert_eq!(
            ResourceTable::parse(&table.to_bytes()).unwrap().to_bytes(),
            table.to_bytes()
        );
    }

    #[test]
    fn translates_into_the_languages_the_table_has_and_says_so_in_its_spec() {
        let offsets = [[0xff; 4], 0u32.to_le_bytes()].concat();
        let in_config = |tag: &str, data| {
            let mut config = locale_config(tag);
            // French in Canada in landscape: more than a language.
            if tag == "fr-CA" {
                config[12] = 2;
            }
            type_chunk_for(&config, 0, 2, &offsets, &entry(0, TYPE_STRING, data))
        };
        let chunks = [
            type_spec(4, &[0, 0x0080]),
            type_chunk(0, 2, &offsets, &entry(0, TYPE_STRING, 0)),
            in_config("fr", 0),
            in_config("fr-CA", 0),
            in_config("de", 0),
        ];
        let mut table = ResourceTable::parse(&table(&chunks)).unwrap();

        table
            .translate(0x7f02_0001, &names(&[("fr", "Nom"), ("es", "Nombre")]))
            .unwrap();

        let some = |text: &str| Some(text.to_owned());
        // French's own, and Canada's, the closest to it; German has no
        // translation, and Spanish one of its own, the only one added.
        assert_eq!(
            translations(&table, 0x7f02_0001),
            [
                (None, some("Shell")),
                (some("fr"), some("Nom")),
                (some("fr-CA"), some("Nom")),
                (some("de"), some("Shell")),
                (some("es"), some("Nombre")),
            ]
        );
        // In the package with the ID: its entry varies by language, among
        // what it did, and it has one type chunk more.
        assert_eq!(spec_of(&table, 1), (vec![0, 0x0080 | CONFIG_LOCALE], 5));
        assert_eq!(spec_of(&table, 0), (vec![0, 0x0080], 4));
        assert_eq!(
            table
                .translate(0x7f02_0000, &names(&[("fr", "Nom")]))
                .unwrap_err()
                .to_string(),
            "it has no resource 0x7f020000"
        );
    }

    #[test]
    fn translates_in_a_table_without_type_specs_or_their_counts() {
        // A compact entry, which keeps its key in place of a size.
        let compact = [
            &7u16.to_le_bytes()[..],
            &(COMPACT | u16::from(TYPE_STRING) << 8).to_le_bytes(),
            &0u32.to_le_bytes(),
        ]
        .concat();
        let offsets = [[0xff; 4], 0u32.to_le_bytes()].concat();
        for chunks in [
            vec![type_chunk(0, 2, &offsets, &compact)],
            vec![type_spec(0, &[0, 0]), type_chunk(0, 2, &offsets, &compact)],
            // A spec too short for the entry, which has no flags to set.
            vec![type_spec(0, &[0]), type_chunk(0, 2, &offsets, &compact)],
        ] {
            let mut table = ResourceTable::parse(&table(&chunks)).unwrap();

            table
                .translate(0x7f02_0001, &names(&[("fr", "Nom")]))
                .unwrap();

            assert_eq!(
                translations(&table, 0x7f02_0001)[1],
                (Some("fr".to_owned()), Some("Nom".to_owned()))
            );
            // Under the same key.
            let entry = table.packages.len() - 16;
            assert_eq!(Reader::at(&table.packages, entry + 4).u32(), 7);
        }
        let mut table = ResourceTable::parse(&table(&[
            type_spec(0, &[0, 0]),
            type_chunk(0, 2, &offsets, &compact),
        ]))
        .unwrap();
        table
            .translate(0x7f02_0001, &names(&[("fr", "Nom")]))
            .unwrap();
        assert_eq!(spec_of(&table, 1), (vec![0, CONFIG_LOCALE], 0));
    }

    #[test]
    fn packs_languages_and_regions_as_android_does() {
        for tag in ["fr", "fil", "pt-BR", "zh-Hans", "zh-Hant-TW", "es-419"] {
            assert_eq!(
                locale(&locale_config(tag)),
                Some((tag.to_owned(), true)),
                "{tag}"
            );
        }
        // Filipino's three letters, as aapt2 packs them.
        assert_eq!(
            locale_config("fil")[8..10],
            // f, i and l are 5, 8 and 11 from a: 0x80 | 11 << 2 | 8 >> 3,
            // then 8 << 5 | 5 in a byte.
            [0xad, 0x05]
        );
        // A script worked out from the language isn't in the tag.
        let mut computed = locale_config("sr-Latn");
        computed[52] = 1;
        assert_eq!(locale(&computed), Some(("sr".to_owned(), true)));
        // A configuration for every language, and one cut short.
        assert_eq!(locale(&4u32.to_le_bytes()), None);
        assert_eq!(
            locale(&locale_config("fr")[..10]),
            Some(("fr".to_owned(), true))
        );
    }
    /// A table with a package 0x7f holding these chunks, and one with
    /// another ID holding them too, which lookups must pass over.
    fn table(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut pool = empty(1 << 8);
        pool.add("Shell");
        let package = |id: u32| {
            let body = chunks.concat();
            let size = u32::try_from(288 + body.len()).unwrap();
            [
                &0x0200u16.to_le_bytes()[..],
                &288u16.to_le_bytes(),
                &size.to_le_bytes(),
                &id.to_le_bytes(),
                &[0; 276],
                &body,
            ]
            .concat()
        };
        let body = [pool.to_bytes(), package(0x01), package(0x7f)].concat();
        let size = u32::try_from(12 + body.len()).unwrap();
        [
            &0x0002u16.to_le_bytes()[..],
            &12u16.to_le_bytes(),
            &size.to_le_bytes(),
            &2u32.to_le_bytes(),
            &body,
        ]
        .concat()
    }

    #[test]
    fn writes_the_shells_table_unchanged() {
        let data = shell_table();

        assert_eq!(ResourceTable::parse(&data).unwrap().to_bytes(), data);
    }

    #[test]
    fn sets_the_shells_name() {
        let mut table = ResourceTable::parse(&shell_table()).unwrap();

        table.set_string(APP_NAME, "Repackaged").unwrap();

        let table = ResourceTable::parse(&table.to_bytes()).unwrap();
        assert_eq!(
            values_with_types(&table, APP_NAME),
            [(TYPE_STRING, Some("Repackaged".to_owned()))]
        );
        // The icon's file paths, in the same pool, are as they were.
        assert_eq!(
            strings(&table, 0x7f02_0000).unwrap(),
            ["res/d2.webp", "res/sK.webp", "res/BW.xml"]
        );
    }

    #[test]
    fn finds_entries_however_the_type_chunk_lists_them() {
        let dense = type_chunk(
            0,
            2,
            &[[0xff; 4], 0u32.to_le_bytes()].concat(),
            &entry(0, TYPE_STRING, 0),
        );
        let offset16 = type_chunk(OFFSET16, 2, &[0xff, 0xff, 0, 0], &entry(0, TYPE_STRING, 0));
        let sparse = type_chunk(SPARSE, 1, &[1, 0, 0, 0], &entry(0, TYPE_STRING, 0));
        let compact = type_chunk(
            0,
            2,
            &[[0xff; 4], 0u32.to_le_bytes()].concat(),
            &[
                &0u16.to_le_bytes()[..],
                &(COMPACT | u16::from(TYPE_STRING) << 8).to_le_bytes(),
                &0u32.to_le_bytes(),
            ]
            .concat(),
        );
        for chunk in [dense, offset16, sparse, compact] {
            let mut table = ResourceTable::parse(&table(&[chunk])).unwrap();

            table.set_string(0x7f02_0001, "Name").unwrap();

            assert_eq!(
                values_with_types(&table, 0x7f02_0001),
                [(TYPE_STRING, Some("Name".to_owned()))]
            );
            // Entry 0 has no value; entry 2 is past the end.
            assert_eq!(values_with_types(&table, 0x7f02_0000), []);
            assert_eq!(values_with_types(&table, 0x7f02_0002), []);
            // A string, not a file.
            assert_eq!(table.files().unwrap(), Vec::<String>::new());
        }
    }

    #[test]
    fn reads_only_the_values_that_are_strings() {
        let entries = [entry(0, TYPE_STRING, 0), entry(0, 0x10, 7)].concat();
        let chunk = type_chunk(
            0,
            2,
            &[0u32.to_le_bytes(), 16u32.to_le_bytes()].concat(),
            &entries,
        );
        let table = ResourceTable::parse(&table(&[chunk])).unwrap();

        assert_eq!(strings(&table, 0x7f02_0000).unwrap(), ["Shell"]);
        assert_eq!(strings(&table, 0x7f02_0001).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn turns_another_kind_of_value_into_a_string() {
        // A reference to another resource.
        let chunk = type_chunk(0, 1, &0u32.to_le_bytes(), &entry(0, 0x01, 0x7f02_0001));
        let mut table = ResourceTable::parse(&table(&[chunk])).unwrap();

        table.set_string(0x7f02_0000, "Name").unwrap();

        assert_eq!(
            values_with_types(&table, 0x7f02_0000),
            [(TYPE_STRING, Some("Name".to_owned()))]
        );
    }

    #[test]
    fn tells_files_strings_and_colours_apart() {
        // The shell's icon: a WebP for two densities, and an adaptive icon.
        let shell = ResourceTable::parse(&shell_table()).unwrap();
        // A string, a colour (#FF2F6FEB, written as #2F6FEB), a reference
        // and a string the pool doesn't have.
        let entries = [
            entry(0, TYPE_STRING, 0),
            entry(0, 0x1d, 0xff2f_6feb),
            entry(0, 0x01, 0x7f02_0000),
            entry(0, TYPE_STRING, 9),
        ]
        .concat();
        let offsets: Vec<u8> = (0..4u32)
            .flat_map(|index| (16 * index).to_le_bytes())
            .collect();
        let mut table =
            ResourceTable::parse(&table(&[type_chunk(0, 4, &offsets, &entries)])).unwrap();

        let values: Vec<_> = (0..4)
            .flat_map(|index| table.values(0x7f02_0000 + index).unwrap())
            .collect();
        table.set_file(0x7f02_0002, "res/icon.png").unwrap();
        table.set_color(0x7f02_0003, 0xff00_ff00).unwrap();

        assert_eq!(
            shell.values(0x7f02_0000).unwrap(),
            [
                Value::File("res/d2.webp".to_owned()),
                Value::File("res/sK.webp".to_owned()),
                Value::File("res/BW.xml".to_owned()),
            ]
        );
        assert_eq!(
            values,
            [
                Value::String("Shell".to_owned()),
                Value::Color(0xff2f_6feb),
                Value::Other,
                Value::Other,
            ]
        );
        assert_eq!(
            table.values(0x7f02_0002).unwrap(),
            [Value::File("res/icon.png".to_owned())]
        );
        assert_eq!(
            table.values(0x7f02_0003).unwrap(),
            [Value::Color(0xff00_ff00)]
        );
        assert_eq!(
            values_with_types(&table, 0x7f02_0003)[0].0,
            TYPE_INT_COLOR_ARGB8
        );
        assert_eq!(
            table.set_color(0x7f02_0009, 0).unwrap_err().to_string(),
            "it has no resource 0x7f020009"
        );
    }

    #[test]
    fn reports_what_it_cant_set() {
        let complex = type_chunk(0, 1, &0u32.to_le_bytes(), &entry(COMPLEX, 0, 0));
        let past_the_end = type_chunk(0, 1, &64u32.to_le_bytes(), &entry(0, TYPE_STRING, 0));
        let offsets_past_entries = {
            let mut chunk = type_chunk(0, 1, &0u32.to_le_bytes(), &entry(0, TYPE_STRING, 0));
            chunk[12..16].copy_from_slice(&100u32.to_le_bytes());
            chunk
        };
        let bad_package = {
            let mut table = table(&[]);
            let package = table.len() - 288;
            table[package + 4..package + 8].copy_from_slice(&2u32.to_le_bytes());
            table
        };
        let bad_type = {
            let mut chunk = type_chunk(0, 1, &0u32.to_le_bytes(), &entry(0, TYPE_STRING, 0));
            chunk[4..8].copy_from_slice(&2u32.to_le_bytes());
            table(&[chunk])
        };
        for (data, message) in [
            (
                table(&[complex]),
                "resource 0x7f020000 holds several values, not one",
            ),
            (table(&[past_the_end]), "it's corrupt or cut short"),
            (table(&[offsets_past_entries]), "it's corrupt or cut short"),
            (bad_package, "it's corrupt or cut short"),
            (bad_type, "it's corrupt or cut short"),
            (table(&[]), "it has no resource 0x7f020000"),
        ] {
            let mut table = ResourceTable::parse(&data).unwrap();

            assert_eq!(
                table
                    .set_string(0x7f02_0000, "Name")
                    .unwrap_err()
                    .to_string(),
                message
            );
            assert_eq!(
                table.set_color(0x7f02_0000, 0).unwrap_err().to_string(),
                message
            );
            // A map isn't a file, and an empty table has none.
            let corrupt = message == "it's corrupt or cut short";
            assert_eq!(
                table.files().map_err(|error| error.to_string()),
                if corrupt {
                    Err(message.to_owned())
                } else {
                    Ok(Vec::new())
                }
            );
        }
    }

    #[test]
    fn rejects_what_isnt_a_resource_table() {
        let data = shell_table();
        let mut bad_pool = data.clone();
        bad_pool[12] = 0;

        for (corrupt, message) in [
            (&b"<resources/>"[..], "it isn't a resource table"),
            (&data[..100], "it isn't a resource table"),
            (&bad_pool, "its string pool is corrupt"),
        ] {
            assert_eq!(
                ResourceTable::parse(corrupt).err().unwrap().to_string(),
                message
            );
        }
    }
}
