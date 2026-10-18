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

/// A resource table whose string and file values can be read and changed,
/// in either format: an APK's `resources.arsc` or an AAB's `resources.pb`.
pub trait Resources {
    /// The resource's string values, one for each configuration it has a
    /// string in. A file resource's value is its path in the package.
    fn strings(&self, id: u32) -> Result<Vec<String>>;
    /// Makes a resource a string in every configuration it has a value in.
    fn set_string(&mut self, id: u32, value: &str) -> Result<()>;
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
                    slots.extend(self.slot(&chunk, index, id)?);
                }
            }
        }
        Ok(slots)
    }

    /// Where the value of the entry at `index` is in a type chunk, if the
    /// chunk has one.
    fn slot(&self, chunk: &Chunk, index: usize, id: u32) -> Result<Option<Slot>> {
        let data = &self.packages;
        let mut header = Reader::at(data, chunk.start + 9);
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
            return Ok(None);
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
            bail!("resource {id:#010x} holds several values, not one");
        }
        if slot.data + 4 > chunk.end() {
            bail!("it's corrupt or cut short");
        }
        Ok(Some(slot))
    }
}

impl Resources for ResourceTable {
    /// The resource's string values, one for each configuration it has a
    /// string in. A file resource's value is its path in the APK.
    fn strings(&self, id: u32) -> Result<Vec<String>> {
        Ok(self
            .slots(id)?
            .into_iter()
            .filter(|slot| self.packages[slot.data_type] == TYPE_STRING)
            .filter_map(|slot| {
                let index = Reader::at(&self.packages, slot.data).u32();
                self.pool.get(index)
            })
            .collect())
    }

    /// Makes a resource a string in every configuration it has a value in.
    fn set_string(&mut self, id: u32, value: &str) -> Result<()> {
        let slots = self.slots(id)?;
        if slots.is_empty() {
            bail!("it has no resource {id:#010x}");
        }
        let index = self.pool.add(value);
        for slot in slots {
            self.packages[slot.data_type] = TYPE_STRING;
            self.packages[slot.data..slot.data + 4].copy_from_slice(&index.to_le_bytes());
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

#[cfg(test)]
mod tests {
    use super::{COMPACT, COMPLEX, OFFSET16, ResourceTable, Resources, SPARSE, TYPE_STRING};
    use crate::android::string_pool::tests::empty;
    use crate::zip::{self, tests::SHELL};

    const APP_NAME: u32 = 0x7f02_0000;

    fn shell_table() -> Vec<u8> {
        zip::read(SHELL).unwrap()[4].contents().unwrap()
    }

    /// Each value the resource has: its type and string, if it's one.
    fn values(table: &ResourceTable, id: u32) -> Vec<(u8, Option<String>)> {
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
            values(&table, APP_NAME),
            [(TYPE_STRING, Some("Repackaged".to_owned()))]
        );
        // The icon's file path, in the same pool, is as it was.
        assert_eq!(table.strings(0x7f01_0000).unwrap(), ["res/o-.png"]);
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
                values(&table, 0x7f02_0001),
                [(TYPE_STRING, Some("Name".to_owned()))]
            );
            // Entry 0 has no value; entry 2 is past the end.
            assert_eq!(values(&table, 0x7f02_0000), []);
            assert_eq!(values(&table, 0x7f02_0002), []);
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

        assert_eq!(table.strings(0x7f02_0000).unwrap(), ["Shell"]);
        assert_eq!(table.strings(0x7f02_0001).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn turns_another_kind_of_value_into_a_string() {
        // A reference to another resource.
        let chunk = type_chunk(0, 1, &0u32.to_le_bytes(), &entry(0, 0x01, 0x7f02_0001));
        let mut table = ResourceTable::parse(&table(&[chunk])).unwrap();

        table.set_string(0x7f02_0000, "Name").unwrap();

        assert_eq!(
            values(&table, 0x7f02_0000),
            [(TYPE_STRING, Some("Name".to_owned()))]
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
