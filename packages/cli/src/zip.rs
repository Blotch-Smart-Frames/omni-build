//! Zip archives, which APKs, AABs and IPAs all are.
//!
//! OmniBuild writes its own rather than using a zip library because an APK
//! needs control a general writer doesn't give: entries Android maps
//! straight from the file must start on a 4-byte or page boundary, and the
//! signature goes between the entries and the central directory, so the
//! writer reports where that is.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::bytes::Reader;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
const END_OF_CENTRAL_DIRECTORY_SIZE: usize = 22;
const LOCAL_HEADER_SIZE: usize = 30;
/// 2.0, the version that introduced deflate.
const VERSION: u16 = 20;
/// General purpose flag bit 0: the entry is encrypted.
const ENCRYPTED: u16 = 1;
/// General purpose flag bit 11: the name is UTF-8.
const UTF8_NAME: u16 = 1 << 11;
/// The extra field apksigner and zipalign pad entries with: the alignment,
/// then zeros. A field of its own keeps the padding readable as a
/// well-formed extra field.
const ALIGNMENT_FIELD: u16 = 0xd935;
/// Unix, as the system byte of "version made by" names it. A Unix entry's
/// external attributes hold the file's mode in their high 16 bits.
const UNIX: u8 = 3;
/// A regular file that everyone can read and only its owner can write.
const FILE_MODE: u32 = 0o100_644;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Stored,
    Deflated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub method: Method,
    pub crc32: u32,
    /// The uncompressed size.
    pub size: u32,
    /// The data as it's stored in the archive: compressed, for a deflated
    /// entry, so an unchanged entry is copied without recompressing it.
    pub data: Vec<u8>,
    /// The MS-DOS time and date the entry was last modified.
    pub time: u16,
    pub date: u16,
    /// The system the entry was made on (the high byte of "version made
    /// by"), and its file attributes there. They're kept as they were read,
    /// because on Unix the attributes hold the file's mode, and an iOS
    /// app's executable won't run without its execute bit.
    pub system: u8,
    pub attributes: u32,
}

impl Entry {
    /// A new entry holding `contents`. It's dated as the Android Gradle
    /// plugin dates every entry, 1981-01-01 01:01:02, so the same files
    /// make the same archive. It's a regular file that everyone can read,
    /// as Xcode leaves an app's resources.
    pub fn new(name: &str, method: Method, contents: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            method,
            crc32: 0,
            size: 0,
            data: Vec::new(),
            time: 0x0821,
            date: 0x0221,
            system: UNIX,
            attributes: FILE_MODE << 16,
        }
        .with_contents(contents)
    }

    /// The entry's contents, decompressed.
    pub fn contents(&self) -> Result<Vec<u8>> {
        match self.method {
            Method::Stored => Ok(self.data.clone()),
            Method::Deflated => {
                let mut contents = Vec::new();
                DeflateDecoder::new(&self.data[..])
                    .read_to_end(&mut contents)
                    .with_context(|| format!("{} is corrupt", self.name))?;
                Ok(contents)
            }
        }
    }

    /// The entry with other contents, compressed as it was.
    pub fn with_contents(&self, contents: &[u8]) -> Self {
        let data = match self.method {
            Method::Stored => contents.to_vec(),
            Method::Deflated => {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                encoder
                    .write_all(contents)
                    .expect("compressing into memory can't fail");
                encoder
                    .finish()
                    .expect("compressing into memory can't fail")
            }
        };
        Self {
            name: self.name.clone(),
            method: self.method,
            crc32: crc32fast::hash(contents),
            size: u32_of(contents.len()),
            data,
            time: self.time,
            date: self.date,
            system: self.system,
            attributes: self.attributes,
        }
    }
}

/// Reads every entry of a zip archive. Only what APKs and AABs use is
/// supported: stored and deflated entries, no encryption, no zip64.
pub fn read(archive: &[u8]) -> Result<Vec<Entry>> {
    // The end of central directory record closes the archive, followed only
    // by a comment of up to 64 KiB.
    let signature = END_OF_CENTRAL_DIRECTORY.to_le_bytes();
    let Some(end) = (0..(archive.len() + 1).saturating_sub(END_OF_CENTRAL_DIRECTORY_SIZE))
        .rev()
        .take(usize::from(u16::MAX) + 1)
        .find(|&at| archive[at..].starts_with(&signature))
    else {
        bail!("it isn't a zip archive");
    };
    let mut record = Reader::at(archive, end + 10);
    let count = record.u16();
    record.skip(4);
    let mut central = Reader::at(archive, offset(record.u32()));
    (0..count)
        .map(|index| read_entry(archive, &mut central, index))
        .collect()
}

/// Reads the entry whose central directory header `central` is at, and its
/// data from its local header.
fn read_entry(archive: &[u8], central: &mut Reader, index: u16) -> Result<Entry> {
    let signature = central.u32();
    // "Version made by" is the zip version, then the system.
    central.skip(1);
    let system = central.u8();
    central.skip(2);
    let flags = central.u16();
    let method = central.u16();
    let time = central.u16();
    let date = central.u16();
    let crc32 = central.u32();
    let compressed_size = central.u32();
    let size = central.u32();
    let name_len = usize::from(central.u16());
    let extra_len = usize::from(central.u16());
    let comment_len = usize::from(central.u16());
    // The disk number and internal attributes.
    central.skip(4);
    let attributes = central.u32();
    let local_offset = offset(central.u32());
    let name = String::from_utf8(central.bytes(name_len).to_vec()).unwrap_or_default();
    central.skip(extra_len + comment_len);

    // The sizes come from the central directory: an entry written with a
    // data descriptor has zeros in its local header.
    let mut local = Reader::at(archive, local_offset);
    let local_signature = local.u32();
    local.skip(22);
    let local_name_len = usize::from(local.u16());
    let local_extra_len = usize::from(local.u16());
    local.skip(local_name_len + local_extra_len);
    let stored = local.bytes(offset(compressed_size)).to_vec();

    if signature != CENTRAL_HEADER
        || local_signature != LOCAL_HEADER
        || name.is_empty()
        || central.overrun()
        || local.overrun()
    {
        bail!("entry {index} of the zip archive is corrupt or cut short");
    }
    let method = match (method, flags & ENCRYPTED) {
        (0, 0) => Method::Stored,
        (8, 0) => Method::Deflated,
        _ => bail!(
            "{name} is encrypted or compressed with an unsupported method ({method}): only stored and deflated entries are"
        ),
    };
    Ok(Entry {
        name,
        method,
        crc32,
        size,
        data: stored,
        time,
        date,
        system,
        attributes,
    })
}

fn offset(value: u32) -> usize {
    usize::try_from(value).expect("OmniBuild runs on 32 and 64-bit systems")
}

fn u16_of(value: usize) -> u16 {
    u16::try_from(value).expect("zip archives without zip64 have under 65,536 entries")
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("zip archives without zip64 are under 4 GiB")
}

/// A written archive, with where its central directory and its end record
/// start: an APK's signature goes in front of the central directory, and
/// covers the end record.
pub struct Written {
    pub bytes: Vec<u8>,
    pub central_directory: usize,
    pub end: usize,
}

/// Writes the entries, in order, into a new archive.
pub fn write(entries: &[Entry]) -> Written {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for entry in entries {
        let local_offset = bytes.len();
        let name = entry.name.as_bytes();
        let extra = alignment_padding(entry, local_offset + LOCAL_HEADER_SIZE + name.len());
        // The fields local and central headers share, from the version
        // needed to the name's length.
        let fields = [
            &VERSION.to_le_bytes()[..],
            &UTF8_NAME.to_le_bytes(),
            &match entry.method {
                Method::Stored => 0u16,
                Method::Deflated => 8,
            }
            .to_le_bytes(),
            &entry.time.to_le_bytes(),
            &entry.date.to_le_bytes(),
            &entry.crc32.to_le_bytes(),
            &u32_of(entry.data.len()).to_le_bytes(),
            &entry.size.to_le_bytes(),
            &u16_of(name.len()).to_le_bytes(),
        ]
        .concat();

        bytes.extend(LOCAL_HEADER.to_le_bytes());
        bytes.extend(&fields);
        bytes.extend(u16_of(extra.len()).to_le_bytes());
        bytes.extend(name);
        bytes.extend(extra);
        bytes.extend(&entry.data);

        central.extend(CENTRAL_HEADER.to_le_bytes());
        // Version made by, then the shared fields. No extra field, comment,
        // disk number or internal attributes.
        central.extend((u16::from(entry.system) << 8 | VERSION).to_le_bytes());
        central.extend(&fields);
        central.extend([0; 8]);
        central.extend(entry.attributes.to_le_bytes());
        central.extend(u32_of(local_offset).to_le_bytes());
        central.extend(name);
    }

    let central_directory = bytes.len();
    bytes.extend(&central);
    let end = bytes.len();
    let count = u16_of(entries.len()).to_le_bytes();
    bytes.extend(END_OF_CENTRAL_DIRECTORY.to_le_bytes());
    // This disk's number and the central directory's: there's one disk.
    bytes.extend([0; 4]);
    bytes.extend(count);
    bytes.extend(count);
    bytes.extend(u32_of(central.len()).to_le_bytes());
    bytes.extend(u32_of(central_directory).to_le_bytes());
    // No comment.
    bytes.extend([0; 2]);
    Written {
        bytes,
        central_directory,
        end,
    }
}

/// The extra field that makes a stored entry's data start where Android
/// needs it, given where the data would start without one. Android maps
/// stored entries straight from the APK: native libraries must start on a
/// page boundary (16 KiB, which Android 15 devices can use and Google Play
/// requires), and everything else on 4 bytes, as zipalign does. Deflated
/// entries are read through a decompressor, so they need nothing.
fn alignment_padding(entry: &Entry, data_offset: usize) -> Vec<u8> {
    let alignment = match entry.method {
        Method::Deflated => return Vec::new(),
        // Case-sensitive, as Android's linker and zipalign are.
        Method::Stored if Path::new(&entry.name).extension() == Some(OsStr::new("so")) => 16384,
        Method::Stored => 4,
    };
    // The field's own header and the alignment come before the padding.
    let padding = (alignment - (data_offset + 6) % alignment) % alignment;
    [
        &ALIGNMENT_FIELD.to_le_bytes()[..],
        &u16_of(2 + padding).to_le_bytes(),
        &u16_of(alignment).to_le_bytes(),
        &vec![0; padding],
    ]
    .concat()
}

#[cfg(test)]
pub mod tests {
    use super::{Entry, Method, read, write};

    pub const SHELL: &[u8] = include_bytes!("../tests/fixtures/shell.apk");

    pub fn entry(name: &str, method: Method, data: &[u8]) -> Entry {
        Entry {
            name: name.to_owned(),
            method,
            crc32: 0x1234_5678,
            size: 99,
            data: data.to_vec(),
            time: 0x0821,
            date: 0x0221,
            system: 0,
            attributes: 0,
        }
    }

    /// The entry `name` among `entries`.
    pub fn named<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        entries.iter().find(|entry| entry.name == name).expect(name)
    }

    /// The contents of the file `name` in the archive `archive`.
    pub fn file(archive: &[u8], name: &str) -> Vec<u8> {
        named(&read(archive).unwrap(), name).contents().unwrap()
    }

    /// Where an entry's data starts in a written archive.
    fn data_offset(archive: &[u8], data: &[u8]) -> usize {
        archive
            .windows(data.len())
            .position(|window| window == data)
            .unwrap()
    }

    #[test]
    fn reads_the_shell_apk() {
        let entries = read(SHELL).unwrap();

        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "META-INF/com/android/build/gradle/app-metadata.properties",
                "lib/arm64-v8a/libshell.so",
                "lib/armeabi-v7a/libshell.so",
                "lib/x86/libshell.so",
                "lib/x86_64/libshell.so",
                "assets/payload.txt",
                "AndroidManifest.xml",
                "res/-6.webp",
                "res/0K.xml",
                "res/0w.xml",
                "res/BW.xml",
                "res/Qr.xml",
                "res/d2.webp",
                "res/sK.webp",
                "res/yw.webp",
                "resources.arsc",
            ]
        );
        let arsc = named(&entries, "resources.arsc");
        assert_eq!(arsc.method, Method::Stored);
        assert_eq!(arsc.size, 1440);
        assert_eq!(arsc.data.len(), 1440);
        assert_eq!(arsc.crc32, crc32(&arsc.data));
        let manifest = named(&entries, "AndroidManifest.xml");
        assert_eq!(manifest.method, Method::Deflated);
        assert_eq!((manifest.size, manifest.data.len()), (3012, 991));
        assert_eq!((manifest.time, manifest.date), (0x0821, 0x0221));
        // Gradle makes some entries as Unix files, with a mode, and others
        // as MS-DOS ones.
        let payload = named(&entries, "assets/payload.txt");
        assert_eq!((payload.system, payload.attributes), (3, 0o100_644 << 16));
        assert_eq!(manifest.system, 0);
    }

    #[test]
    fn writes_what_it_reads() {
        let entries = read(SHELL).unwrap();

        let written = write(&entries);

        assert_eq!(read(&written.bytes).unwrap(), entries);
        assert_eq!(
            &written.bytes[written.central_directory..][..4],
            b"PK\x01\x02"
        );
        assert_eq!(&written.bytes[written.end..][..4], b"PK\x05\x06");
    }

    #[test]
    fn aligns_stored_entries_and_native_libraries() {
        let entries = [
            entry("a.txt", Method::Deflated, b"deflated"),
            entry("resources.arsc", Method::Stored, b"table"),
            entry(
                "lib/arm64-v8a/libshell.so",
                Method::Stored,
                b"\x7fELF library",
            ),
        ];

        let archive = write(&entries).bytes;

        assert_eq!(data_offset(&archive, b"table") % 4, 0);
        assert_eq!(data_offset(&archive, b"\x7fELF library") % 16384, 0);
        assert_eq!(read(&archive).unwrap(), entries);
    }

    #[test]
    fn decompresses_and_recompresses_entries() {
        let entries = read(SHELL).unwrap();
        let manifest = named(&entries, "AndroidManifest.xml");
        let contents = manifest.contents().unwrap();

        let rewritten = manifest.with_contents(b"other contents");
        let stored = named(&entries, "resources.arsc").with_contents(b"table");

        assert_eq!(contents.len(), 3012);
        assert_eq!(&contents[..4], b"\x03\x00\x08\x00");
        assert_eq!(
            manifest.with_contents(&contents).contents().unwrap(),
            contents
        );
        assert_eq!(rewritten.contents().unwrap(), b"other contents");
        assert_eq!(rewritten.crc32, crc32(b"other contents"));
        assert_eq!((rewritten.size, rewritten.time), (14, manifest.time));
        assert_eq!(
            (stored.method, stored.data.as_slice()),
            (Method::Stored, &b"table"[..])
        );
    }

    #[test]
    fn makes_new_entries_as_the_shells_are() {
        let entries = read(SHELL).unwrap();
        let shell = named(&entries, "assets/payload.txt");

        let new = Entry::new("assets/payload.txt", Method::Deflated, b"payload");

        assert_eq!(new.contents().unwrap(), b"payload");
        assert_eq!((new.time, new.date), (shell.time, shell.date));
        assert_eq!(
            (new.system, new.attributes),
            (shell.system, shell.attributes)
        );
    }

    #[test]
    fn keeps_the_system_and_attributes_of_each_entry() {
        let mut executable = entry("Shell.app/Shell", Method::Stored, b"\xcf\xfa\xed\xfe");
        (executable.system, executable.attributes) = (3, 0o100_755 << 16);
        let mut folder = entry("Shell.app/", Method::Stored, b"");
        // MS-DOS's directory attribute.
        folder.attributes = 0x10;

        let entries = read(&write(&[executable.clone(), folder.clone()]).bytes).unwrap();

        assert_eq!(entries, [executable, folder]);
    }

    #[test]
    fn reports_a_corrupt_deflated_entry() {
        let corrupt = entry("AndroidManifest.xml", Method::Deflated, &[0xff; 8]);

        assert_eq!(
            corrupt.contents().unwrap_err().to_string(),
            "AndroidManifest.xml is corrupt"
        );
    }

    #[test]
    fn writes_an_empty_archive() {
        let written = write(&[]);

        assert_eq!(read(&written.bytes).unwrap(), []);
        assert_eq!(written.central_directory, 0);
    }

    #[test]
    fn rejects_what_isnt_a_zip_archive() {
        for bytes in [&b"PK"[..], &[0; 100]] {
            assert_eq!(
                read(bytes).unwrap_err().to_string(),
                "it isn't a zip archive"
            );
        }
    }

    #[test]
    fn rejects_an_archive_cut_short() {
        let mut archive = write(&[entry("a.txt", Method::Stored, b"text")]).bytes;
        // Point the end record's central directory offset past the end.
        let end = archive.len() - 22;
        archive[end + 16..end + 20].copy_from_slice(&u32::MAX.to_le_bytes());

        assert_eq!(
            read(&archive).unwrap_err().to_string(),
            "entry 0 of the zip archive is corrupt or cut short"
        );
    }

    #[test]
    fn rejects_encrypted_entries_and_other_methods() {
        let archive = write(&[entry("a.txt", Method::Stored, b"text")]).bytes;
        let central = archive.len() - 22 - (46 + 5);
        for (field, value) in [(8, 1u16), (10, 12)] {
            let mut archive = archive.clone();
            archive[central + field..central + field + 2].copy_from_slice(&value.to_le_bytes());

            assert!(
                read(&archive)
                    .unwrap_err()
                    .to_string()
                    .starts_with("a.txt is encrypted or compressed with an unsupported method"),
            );
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        // Bitwise CRC-32, as zip uses: enough to check one entry.
        let mut crc = !0u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }
}
