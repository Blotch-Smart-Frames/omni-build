//! DEX files, Android's compiled code, as far as adding a native module's
//! to an app goes: the version of the format, which says the oldest Android
//! that reads the file, and the classes the file defines. Android loads the
//! first definition of a class it finds in an app and never another, so a
//! module's copy of a library the shell has too would run against the
//! shell's, whichever version it was built for. Gradle refuses duplicate
//! classes when it builds an app, and so does OmniBuild.

use anyhow::{Result, bail};

use crate::bytes::Reader;

const MAGIC: &[u8] = b"dex\n";
/// Where the header has the count and offset of the string IDs, the type
/// IDs, and the class definitions, each two words.
const STRING_IDS: usize = 0x38;
const TYPE_IDS: usize = 0x40;
const CLASS_DEFS: usize = 0x60;
/// Each class definition's size: its type's index first, then seven more
/// words.
const CLASS_DEF_SIZE: usize = 32;
/// Each version of the format, in the header after the magic, and the
/// first Android to read it, by API level: 035 every version, then 7.0,
/// 8.0, 9 and 10, as AOSP's description of the format has them. 041, the
/// container format, is an experiment of Android 16's that d8 doesn't
/// write for apps, and OmniBuild doesn't read.
const VERSIONS: [(&[u8], u32); 5] = [
    (b"035\0", 1),
    (b"037\0", 24),
    (b"038\0", 26),
    (b"039\0", 28),
    (b"040\0", 29),
];

pub struct Dex {
    /// The version of the format, as the header has it (`035`).
    pub version: String,
    /// The oldest Android that reads it, by API level.
    pub min_sdk: u32,
    /// The classes it defines, as Java names them
    /// (`dev.example.haptics.Haptics`).
    pub classes: Vec<String>,
}

impl Dex {
    pub fn parse(dex: &[u8]) -> Result<Self> {
        let header = dex.get(MAGIC.len()..MAGIC.len() + 4).unwrap_or_default();
        if !dex.starts_with(MAGIC) {
            bail!("it isn't a DEX file");
        }
        let Some(&(_, min_sdk)) = VERSIONS.iter().find(|(version, _)| *version == header) else {
            bail!(
                "it's a version of the DEX format, {}, that OmniBuild doesn't read: build it for an older Android, with d8's --min-api",
                String::from_utf8_lossy(header).trim_end_matches('\0')
            );
        };
        let ids = |at: usize| {
            let mut header = Reader::at(dex, at);
            (header.u32() as usize, header.u32() as usize)
        };
        let (strings, string_ids) = ids(STRING_IDS);
        let (types, type_ids) = ids(TYPE_IDS);
        let (count, class_defs) = ids(CLASS_DEFS);
        let mut classes = Vec::new();
        for class in 0..count {
            let at = class_defs.saturating_add(class.saturating_mul(CLASS_DEF_SIZE));
            let mut reader = Reader::at(dex, at);
            let type_index = reader.u32() as usize;
            let string_index =
                Reader::at(dex, type_ids.saturating_add(type_index * 4)).u32() as usize;
            let data = Reader::at(dex, string_ids.saturating_add(string_index * 4)).u32() as usize;
            let descriptor = (!reader.overrun() && type_index < types && string_index < strings)
                .then(|| descriptor(dex, data))
                .flatten();
            let Some(descriptor) = descriptor else {
                bail!("it's cut short or corrupt");
            };
            classes.push(descriptor);
        }
        Ok(Self {
            version: String::from_utf8_lossy(&header[..3]).into_owned(),
            min_sdk,
            classes,
        })
    }
}

/// The class a type descriptor at `at` names, as Java names it: the
/// descriptor `Ldev/example/Haptics;` is `dev.example.Haptics`. A string's
/// data is its length in UTF-16 units, as a ULEB128, then its bytes in
/// Java's modified UTF-8, up to a 0.
fn descriptor(dex: &[u8], at: usize) -> Option<String> {
    let data = dex.get(at..)?;
    let start = data.iter().position(|byte| byte & 0x80 == 0)? + 1;
    let length = data[start..].iter().position(|&byte| byte == 0)?;
    let text = String::from_utf8_lossy(&data[start..start + length]);
    let class = text.strip_prefix('L')?.strip_suffix(';')?;
    Some(class.replace('/', "."))
}

#[cfg(test)]
pub mod tests {
    use super::Dex;

    /// A DEX file of the format's `version` with only what OmniBuild reads:
    /// the header, then the string IDs, the type IDs and the class
    /// definitions of `classes`, by their Java names, then the strings.
    pub fn made_up(version: &str, classes: &[&str]) -> Vec<u8> {
        let count = classes.len();
        let string_ids = 0x70;
        let type_ids = string_ids + 4 * count;
        let class_defs = type_ids + 4 * count;
        let mut strings = class_defs + 32 * count;
        let mut dex = vec![0; strings];
        dex[..8].copy_from_slice(format!("dex\n{version}\0").as_bytes());
        let mut put = |at: usize, value: usize| {
            dex[at..at + 4].copy_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
        };
        for (at, value) in [
            (0x38, count),
            (0x3c, string_ids),
            (0x40, count),
            (0x44, type_ids),
            (0x60, count),
            (0x64, class_defs),
        ] {
            put(at, value);
        }
        let mut data = Vec::new();
        for (index, class) in classes.iter().enumerate() {
            put(string_ids + 4 * index, strings);
            put(type_ids + 4 * index, index);
            put(class_defs + 32 * index, index);
            let descriptor = format!("L{};", class.replace('.', "/"));
            // A length of 200 takes two bytes, as a ULEB128.
            data.extend([0xc8, 0x01]);
            data.extend(descriptor.as_bytes());
            data.push(0);
            strings += descriptor.len() + 3;
        }
        dex.extend(data);
        dex
    }

    #[test]
    fn reads_the_version_and_the_classes() {
        let dex = Dex::parse(&made_up(
            "038",
            &[
                "dev.example.haptics.Haptics",
                "dev.example.haptics.Haptics$Style",
            ],
        ))
        .unwrap();

        assert_eq!(dex.version, "038");
        assert_eq!(dex.min_sdk, 26);
        assert_eq!(
            dex.classes,
            [
                "dev.example.haptics.Haptics",
                "dev.example.haptics.Haptics$Style"
            ]
        );
        assert_eq!(Dex::parse(&made_up("035", &[])).unwrap().min_sdk, 1);
    }

    #[test]
    fn reports_a_dex_file_it_cant_read() {
        let dex = made_up("035", &["dev.example.Haptics"]);
        let set = |at: usize, value: u32| {
            let mut changed = dex.clone();
            changed[at..at + 4].copy_from_slice(&value.to_le_bytes());
            changed
        };
        let named = |name: &[u8]| {
            let mut changed = dex.clone();
            // In place of the one string, its length and all.
            changed.truncate(changed.len() - 24);
            changed.extend(name);
            changed
        };
        for (dex, message) in [
            (b"PK\x03\x04".to_vec(), "it isn't a DEX file"),
            (
                made_up("041", &[]),
                "it's a version of the DEX format, 041, that OmniBuild doesn't read: build it for an older Android, with d8's --min-api",
            ),
            (dex[..dex.len() - 10].to_vec(), "it's cut short or corrupt"),
            (set(0x60, 2), "it's cut short or corrupt"),
            (set(0x64, 0xffff), "it's cut short or corrupt"),
            (set(0x40, 0), "it's cut short or corrupt"),
            (set(0x38, 0), "it's cut short or corrupt"),
            (set(0x70, 0xffff), "it's cut short or corrupt"),
            (named(b"\x80\x80"), "it's cut short or corrupt"),
            (
                named(b"\x15Ldev/example/Haptics;"),
                "it's cut short or corrupt",
            ),
            (
                named(b"\x14dev/example/Haptics;\0"),
                "it's cut short or corrupt",
            ),
            (
                named(b"\x14Ldev/example/Haptics\0"),
                "it's cut short or corrupt",
            ),
        ] {
            assert_eq!(Dex::parse(&dex).err().unwrap().to_string(), message);
        }
    }
}
