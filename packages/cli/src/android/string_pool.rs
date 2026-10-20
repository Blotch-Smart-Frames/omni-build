//! The string pool chunk (`ResStringPool` in AOSP's `ResourceTypes.h`)
//! that binary XML and resource tables keep their strings in.

use anyhow::{Result, bail};

use crate::bytes::Reader;

const STRING_POOL: u16 = 0x0001;
const HEADER_SIZE: usize = 28;
/// The strings are sorted, so Android may binary-search them. Appending
/// breaks the order, so the flag goes.
const SORTED: u32 = 1 << 0;
/// The strings are UTF-8 rather than UTF-16.
const UTF8: u32 = 1 << 8;

/// A string pool that strings can be added to. The strings already there
/// keep their exact bytes: an Android string pool may share data between
/// strings and use encodings that decoding and encoding again wouldn't
/// reproduce, and every other index into it must stay valid.
#[derive(Clone)]
pub struct StringPool {
    flags: u32,
    /// Where each string starts in `strings`.
    offsets: Vec<u32>,
    /// Where each style (the spans of a styled string) starts in `styles`.
    style_offsets: Vec<u32>,
    strings: Vec<u8>,
    styles: Vec<u8>,
}

impl StringPool {
    /// Reads the pool chunk at the start of `data`. Returns the pool and the
    /// chunk's size.
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        let mut header = Reader::at(data, 0);
        let kind = header.u16();
        let header_size = usize::from(header.u16());
        let size = offset(header.u32());
        let string_count = offset(header.u32());
        let style_count = offset(header.u32());
        let flags = header.u32();
        let strings_start = offset(header.u32());
        let styles_start = offset(header.u32());
        let strings_end = if style_count == 0 { size } else { styles_start };
        if kind != STRING_POOL
            || header.overrun()
            || size > data.len()
            || header_size
                .saturating_add(string_count.saturating_add(style_count).saturating_mul(4))
                > strings_start
            || strings_start > strings_end
            || strings_end > size
        {
            bail!("its string pool is corrupt");
        }

        let mut offsets = Reader::at(data, header_size);
        Ok((
            Self {
                flags,
                offsets: (0..string_count).map(|_| offsets.u32()).collect(),
                style_offsets: (0..style_count).map(|_| offsets.u32()).collect(),
                strings: data[strings_start..strings_end].to_vec(),
                styles: data[strings_end..size].to_vec(),
            },
            size,
        ))
    }

    /// The string at `index`, or nothing if there's none there.
    pub fn get(&self, index: u32) -> Option<String> {
        let start = offset(*self.offsets.get(offset(index))?);
        let mut reader = Reader::at(&self.strings, start);
        let string = if self.flags & UTF8 == 0 {
            let len = utf16_length(&mut reader);
            let (units, _) = reader.bytes(len.saturating_mul(2)).as_chunks::<2>();
            let units: Vec<u16> = units.iter().copied().map(u16::from_le_bytes).collect();
            String::from_utf16_lossy(&units)
        } else {
            // The length in UTF-16 code units, then in bytes.
            utf8_length(&mut reader);
            let len = utf8_length(&mut reader);
            String::from_utf8_lossy(reader.bytes(len)).into_owned()
        };
        (!reader.overrun()).then_some(string)
    }

    /// Adds a string at the end, and returns its index.
    /// The index of the first string equal to `value`, if the pool has one.
    pub fn find(&self, value: &str) -> Option<u32> {
        (0..u32_of(self.offsets.len())).find(|&index| self.get(index).as_deref() == Some(value))
    }

    pub fn add(&mut self, value: &str) -> u32 {
        let index = u32_of(self.offsets.len());
        self.offsets.push(u32_of(self.strings.len()));
        let units = value.encode_utf16().count();
        if self.flags & UTF8 == 0 {
            self.strings.extend(encode_length(units, 0x8000, 2));
            self.strings
                .extend(value.encode_utf16().flat_map(u16::to_le_bytes));
            self.strings.extend([0, 0]);
        } else {
            self.strings.extend(encode_length(units, 0x80, 1));
            self.strings.extend(encode_length(value.len(), 0x80, 1));
            self.strings.extend(value.as_bytes());
            self.strings.push(0);
        }
        self.flags &= !SORTED;
        index
    }

    /// The pool as a chunk.
    pub fn to_bytes(&self) -> Vec<u8> {
        let strings_start = HEADER_SIZE + 4 * (self.offsets.len() + self.style_offsets.len());
        // Styles start on a 4-byte boundary.
        let padding = (4 - self.strings.len() % 4) % 4;
        let styles_start = if self.style_offsets.is_empty() {
            0
        } else {
            strings_start + self.strings.len() + padding
        };
        let size = strings_start + self.strings.len() + padding + self.styles.len();
        [
            &STRING_POOL.to_le_bytes()[..],
            &u16::try_from(HEADER_SIZE)
                .expect("the header is 28 bytes")
                .to_le_bytes(),
            &u32_of(size).to_le_bytes(),
            &u32_of(self.offsets.len()).to_le_bytes(),
            &u32_of(self.style_offsets.len()).to_le_bytes(),
            &self.flags.to_le_bytes(),
            &u32_of(strings_start).to_le_bytes(),
            &u32_of(styles_start).to_le_bytes(),
            &self
                .offsets
                .iter()
                .chain(&self.style_offsets)
                .flat_map(|offset| offset.to_le_bytes())
                .collect::<Vec<_>>(),
            &self.strings,
            &vec![0; padding],
            &self.styles,
        ]
        .concat()
    }
}

/// A UTF-16 string's length in code units: one 16-bit unit, or two when the
/// first has its top bit set.
fn utf16_length(reader: &mut Reader) -> usize {
    let first = usize::from(reader.u16());
    if first & 0x8000 == 0 {
        first
    } else {
        ((first & 0x7fff) << 16) | usize::from(reader.u16())
    }
}

/// A UTF-8 string's length: one byte, or two when the first has its top bit
/// set.
fn utf8_length(reader: &mut Reader) -> usize {
    let first = usize::from(reader.u8());
    if first & 0x80 == 0 {
        first
    } else {
        ((first & 0x7f) << 8) | usize::from(reader.u8())
    }
}

/// A length as `utf16_length` or `utf8_length` read it, in units of
/// `width` bytes, where `high` is the flag for the two-unit form.
fn encode_length(len: usize, high: usize, width: usize) -> Vec<u8> {
    let units = if len < high {
        vec![len]
    } else {
        vec![high | (len >> (8 * width)), len & ((1 << (8 * width)) - 1)]
    };
    units
        .into_iter()
        .flat_map(|unit| unit.to_le_bytes()[..width].to_vec())
        .collect()
}

fn offset(value: u32) -> usize {
    usize::try_from(value).expect("OmniBuild runs on 32 and 64-bit systems")
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("string pools are under 4 GiB")
}

#[cfg(test)]
pub mod tests {
    use super::{SORTED, StringPool, UTF8};
    use crate::zip::{self, tests::SHELL};

    fn shell_file(name: &str) -> Vec<u8> {
        zip::read(SHELL)
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .contents()
            .unwrap()
    }

    /// An empty pool with these flags.
    pub fn empty(flags: u32) -> StringPool {
        StringPool {
            flags,
            offsets: Vec::new(),
            style_offsets: Vec::new(),
            strings: Vec::new(),
            styles: Vec::new(),
        }
    }

    #[test]
    fn reads_and_writes_the_manifests_utf16_pool_unchanged() {
        let manifest = shell_file("AndroidManifest.xml");

        let (pool, size) = StringPool::parse(&manifest[8..]).unwrap();

        assert_eq!(pool.flags & UTF8, 0);
        assert_eq!(pool.get(0).as_deref(), Some("theme"));
        assert_eq!(pool.get(99), None);
        assert_eq!(pool.to_bytes(), &manifest[8..8 + size]);
    }

    #[test]
    fn reads_and_writes_the_resource_tables_utf8_pool_unchanged() {
        let table = shell_file("resources.arsc");

        let (pool, size) = StringPool::parse(&table[12..]).unwrap();

        assert_eq!(pool.flags & UTF8, UTF8);
        assert_eq!(pool.get(8).as_deref(), Some("OmniBuild Shell"));
        assert_eq!(pool.to_bytes(), &table[12..12 + size]);
    }

    #[test]
    fn adds_strings_in_either_encoding_and_any_length() {
        let long = "é".repeat(200);
        for flags in [SORTED, UTF8 | SORTED] {
            let mut pool = empty(flags);

            let indexes = [pool.add("dev.omnibuild.app"), pool.add(&long), pool.add("")];

            let (pool, _) = StringPool::parse(&pool.to_bytes()).unwrap();
            assert_eq!(indexes, [0, 1, 2]);
            assert_eq!(pool.get(0).as_deref(), Some("dev.omnibuild.app"));
            assert_eq!(pool.get(1).as_deref(), Some(long.as_str()));
            assert_eq!(pool.get(2).as_deref(), Some(""));
            assert_eq!(pool.flags & SORTED, 0);
        }
    }

    #[test]
    fn reads_a_utf16_string_longer_than_a_unit_counts() {
        let long = "a".repeat(0x8001);
        let mut pool = empty(0);
        pool.add(&long);

        let (pool, _) = StringPool::parse(&pool.to_bytes()).unwrap();

        assert_eq!(pool.get(0).as_deref(), Some(long.as_str()));
    }

    #[test]
    fn keeps_styles_after_the_strings() {
        let mut pool = empty(UTF8);
        pool.add("bold");
        pool.style_offsets = vec![0];
        pool.styles = vec![1, 2, 3, 4, 0xff, 0xff, 0xff, 0xff];

        let (read, size) = StringPool::parse(&pool.to_bytes()).unwrap();

        assert_eq!(read.styles, pool.styles);
        assert_eq!(read.style_offsets, [0]);
        assert_eq!(read.get(0).as_deref(), Some("bold"));
        assert_eq!(size % 4, 0);
    }

    #[test]
    fn a_string_past_the_end_of_the_pool_is_missing() {
        let mut pool = empty(UTF8);
        pool.offsets.push(100);

        assert_eq!(pool.get(0), None);
    }

    #[test]
    fn rejects_a_corrupt_pool() {
        let manifest = shell_file("AndroidManifest.xml");
        let pool = &manifest[8..];
        let mut wrong_type = pool.to_vec();
        wrong_type[0] = 2;
        let mut too_many_strings = pool.to_vec();
        too_many_strings[8..12].copy_from_slice(&u32::MAX.to_le_bytes());

        for corrupt in [&wrong_type[..], &too_many_strings, &pool[..20]] {
            assert_eq!(
                StringPool::parse(corrupt).err().unwrap().to_string(),
                "its string pool is corrupt"
            );
        }
    }
}
