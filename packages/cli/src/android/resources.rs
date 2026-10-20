//! The resource table in an APK, `resources.arsc` (`ResTable` in AOSP's
//! `ResourceTypes.h`), as aapt2 compiles it.
//!
//! The table holds one string pool for every string value, then a package
//! per resource package: its type and key names, then for each type, a
//! chunk per configuration (language, density, ...) with an entry for each
//! resource that has a value in it. OmniBuild changes values in place and
//! adds the strings they need to the end of the pool, so every other
//! chunk, offset and string stays as aapt2 wrote it.

use anyhow::{Result, bail};

use super::chunk::{Chunk, chunks};
use super::string_pool::StringPool;
use crate::bytes::Reader;

const TABLE: u16 = 0x0002;
const PACKAGE: u16 = 0x0200;
const TYPE: u16 = 0x0201;
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
/// type, and its data (a string pool index, for a string).
struct Slot {
    data_type: usize,
    data: usize,
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
        let (size, flags) = (usize::from(header.u16()), header.u16());
        // A compact entry keeps its value's type in the top byte of its
        // flags, and its data after them. Any other has a Res_value after
        // its header: size, a zero byte, the type, and the data.
        let slot = if flags & COMPACT == 0 {
            Slot {
                data_type: at + size + 3,
                data: at + size + 4,
            }
        } else {
            Slot {
                data_type: at + 3,
                data: at + 4,
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

#[cfg(test)]
pub mod tests {
    use super::{
        COMPACT, COMPLEX, OFFSET16, ResourceTable, Resources, SPARSE, TYPE_INT_COLOR_ARGB8,
        TYPE_STRING, Value,
    };
    use crate::android::string_pool::tests::empty;
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
        let header_size = 24;
        let entries_start = header_size + offsets.len();
        let size = u32::try_from(entries_start + entries.len()).unwrap();
        [
            &0x0201u16.to_le_bytes()[..],
            &u16::try_from(header_size).unwrap().to_le_bytes(),
            &size.to_le_bytes(),
            &[2, flags, 0, 0],
            &count.to_le_bytes(),
            &u32::try_from(entries_start).unwrap().to_le_bytes(),
            &4u32.to_le_bytes(),
            offsets,
            entries,
        ]
        .concat()
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
