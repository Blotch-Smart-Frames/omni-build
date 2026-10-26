//! Mach-O executables, as far as signing them goes. An iOS app's
//! executable is a 64-bit Mach-O file for one architecture, or, for the
//! Simulator, a universal file with one for each, which are each signed on
//! their own. A watch app's, for devices, is universal too, with a 32-bit
//! file for `arm64_32` (64-bit code with 32-bit pointers, which Apple
//! Watches since Series 4 run) beside one for `arm64`.
//!
//! The signature is the last thing in the file, at the end of the
//! `__LINKEDIT` segment, where the `LC_CODE_SIGNATURE` load command points.
//! OmniBuild replaces a signature and never adds one, which would mean
//! finding room for a load command: Xcode signs a Simulator build ad hoc,
//! and the linker reserves a signature in a device build when asked to
//! (`-Wl,-adhoc_codesign`). The signature hashes the load commands too, so
//! they're pointed at a signature of the new size before the code is
//! hashed.

use anyhow::{Context, Result, bail};

use crate::bytes::Reader;

const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
/// A universal file's header is big-endian.
const FAT_MAGIC: u32 = 0xcafe_babe;
/// A universal file's header, and the size of each architecture's entry.
const FAT_HEADER_SIZE: usize = 8;
const FAT_ARCH_SIZE: usize = 20;
const LC_SEGMENT: u32 = 0x1;
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;
/// The OS and its version the code is for, and the older command for
/// watchOS alone, each with the earliest version it runs on, as
/// `major << 16 | minor << 8 | patch`: in the first after its platform, in
/// the second first.
const LC_BUILD_VERSION: u32 = 0x32;
const LC_VERSION_MIN_WATCHOS: u32 = 0x30;
/// codesign signs `arm64_32` code for watchOS before 12 with SHA-1 hashes
/// as well as SHA-256 ones, and so every architecture in the same file, as
/// measured with Xcode 27.1's for each version from watchOS 4: `arm64`
/// code, and the watchOS Simulator's, get SHA-256 alone for every version.
const SHA256_ONLY_WATCHOS: u32 = 12 << 16;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const CPU_TYPE_ARM64_32: u32 = 0x0200_000c;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
/// The file type of a dynamic library, as a framework's code is.
pub const MH_DYLIB: u32 = 0x6;

/// Where a Mach-O file of each width has what signing reads and changes:
/// the size of its header, which the load commands follow, and the kind
/// of its segment load commands, whose virtual size, file offset and file
/// size follow their name and virtual address, each a word of that width.
struct Layout {
    header_size: usize,
    segment: u32,
    /// Bytes in a word, 4 or 8.
    word: usize,
}

static MACHO_32: Layout = Layout {
    header_size: 28,
    segment: LC_SEGMENT,
    word: 4,
};
static MACHO_64: Layout = Layout {
    header_size: 32,
    segment: LC_SEGMENT_64,
    word: 8,
};

impl Layout {
    /// Where a segment load command has its virtual size, after its kind,
    /// size, name and virtual address.
    fn vm_size(&self) -> usize {
        24 + self.word
    }

    fn file_offset(&self) -> usize {
        24 + 2 * self.word
    }

    fn file_size(&self) -> usize {
        24 + 3 * self.word
    }

    /// The word at `at`.
    fn read(&self, macho: &[u8], at: usize) -> u64 {
        let mut reader = Reader::at(macho, at);
        if self.word == 4 {
            u64::from(reader.u32())
        } else {
            reader.u64()
        }
    }

    /// Writes `value` as the word at `at`.
    fn write(&self, macho: &mut [u8], at: usize, value: u64) {
        macho[at..at + self.word].copy_from_slice(&value.to_le_bytes()[..self.word]);
    }
}
/// codesign leaves room for a signature 8 bytes longer for each code
/// directory than the one it writes, padded to a multiple of 16 bytes, as
/// measured signing a library under identifiers of every length from 1 to
/// 39, and a watch app for watchOS 11, with a SHA-1 code directory too,
/// from 1 to 40.
const SIGNATURE_SLACK: usize = 8;
const SIGNATURE_ALIGNMENT: usize = 16;
/// codesign rounds `__LINKEDIT`'s size in memory up to 16 KiB, arm64's page
/// size, for every architecture.
const LINKEDIT_ROUNDING: u64 = 0x4000;
/// And it starts each architecture of a universal file on a 16 KiB
/// boundary, as a power of 2, whatever alignment it had.
const ALIGN: u32 = 14;

/// One architecture's code, as its signature hashes it.
pub struct Code<'a> {
    /// The file up to its signature, its load commands pointing at the new
    /// one.
    pub bytes: &'a [u8],
    /// The size of the pages it's hashed in, as a power of 2: codesign
    /// hashes arm64 code in 16 KiB pages, as iOS maps it, and `x86_64` code
    /// in 4 KiB pages.
    pub page_size: u8,
    /// Where the `__TEXT` segment starts in the file, and its size: the
    /// code that runs, which iOS checks against the signature's own record
    /// of it.
    pub text: (u64, u64),
    /// Whether it's hashed with SHA-1 too, in a code directory of its own
    /// beside the SHA-256 one, as codesign signs every architecture of a
    /// file with `arm64_32` code for watchOS before 12.
    pub sha1: bool,
}

/// An executable, read for signing: a Mach-O file of one architecture, or
/// a universal file of several, each with its CPU type and subtype.
pub enum Executable<'a> {
    Thin(Architecture<'a>),
    Universal(Vec<(u32, u32, Architecture<'a>)>),
}

impl<'a> Executable<'a> {
    /// Reads an executable, each of whose architectures has to have a
    /// signature at its end, to replace.
    pub fn read(executable: &'a [u8]) -> Result<Self> {
        let Some(slices) = universal(executable)? else {
            return Ok(Self::Thin(Architecture::read(executable)?));
        };
        let mut architectures = Vec::new();
        for (cpu, subtype, code) in slices {
            architectures.push((cpu, subtype, Architecture::read(code)?));
        }
        Ok(Self::Universal(architectures))
    }

    /// Whether codesign signs it with SHA-1 hashes too, beside SHA-256
    /// ones: when it has `arm64_32` code for watchOS before 12, which has
    /// every architecture in the file signed so. Its bundle's seal has them
    /// too, so it's known before the bundle is sealed.
    pub fn needs_sha1(&self) -> bool {
        match self {
            Self::Thin(architecture) => architecture.is_for_old_watchos(),
            Self::Universal(architectures) => architectures
                .iter()
                .any(|(_, _, architecture)| architecture.is_for_old_watchos()),
        }
    }

    /// The executable, each architecture with the signature `sign` makes
    /// for its code in place of the one it had.
    pub fn sign(self, sign: impl Fn(&Code) -> Vec<u8>) -> Vec<u8> {
        let sha1 = self.needs_sha1();
        let architectures = match self {
            Self::Thin(architecture) => return architecture.sign(sha1, &sign),
            Self::Universal(architectures) => architectures,
        };

        // A universal file: each architecture signed, then laid out again,
        // since signing changes their sizes.
        let count = architectures.len();
        let mut out = vec![0; FAT_HEADER_SIZE + FAT_ARCH_SIZE * count];
        out[..4].copy_from_slice(&FAT_MAGIC.to_be_bytes());
        out[4..8].copy_from_slice(&u32_of(count).to_be_bytes());
        for (index, (cpu, subtype, architecture)) in architectures.into_iter().enumerate() {
            let code = architecture.sign(sha1, &sign);
            let offset = out.len().next_multiple_of(1 << ALIGN);
            out.resize(offset, 0);
            let entry = FAT_HEADER_SIZE + FAT_ARCH_SIZE * index;
            for (at, field) in [cpu, subtype, u32_of(offset), u32_of(code.len()), ALIGN]
                .into_iter()
                .enumerate()
            {
                out[entry + at * 4..entry + at * 4 + 4].copy_from_slice(&field.to_be_bytes());
            }
            out.extend(code);
        }
        out
    }
}

/// A universal file's architectures, each its CPU type and subtype and its
/// own Mach-O file.
type Slices<'a> = Vec<(u32, u32, &'a [u8])>;

/// A universal file's architectures, or none if it isn't one.
fn universal(file: &[u8]) -> Result<Option<Slices<'_>>> {
    if Reader::at(file, 0).u32_be() != FAT_MAGIC {
        return Ok(None);
    }
    let mut header = Reader::at(file, 4);
    let count = header.u32_be() as usize;
    let table = header.bytes(count.saturating_mul(FAT_ARCH_SIZE));
    if header.overrun() {
        bail!("it's truncated");
    }
    let mut architectures = Vec::new();
    for entry in table.as_chunks::<FAT_ARCH_SIZE>().0 {
        let mut entry = Reader::at(entry, 0);
        let (cpu, subtype) = (entry.u32_be(), entry.u32_be());
        let (offset, size) = (entry.u32_be() as usize, entry.u32_be() as usize);
        let code = file
            .get(offset..offset.saturating_add(size))
            .context("it's truncated")?;
        architectures.push((cpu, subtype, code));
    }
    Ok(Some(architectures))
}

/// The CPU type and the file type (`MH_EXECUTE`, `MH_DYLIB`) of each of a
/// Mach-O file's architectures, signed or not.
pub fn architectures(file: &[u8]) -> Result<Vec<(u32, u32)>> {
    let slices = match universal(file)? {
        Some(slices) => slices.into_iter().map(|(_, _, code)| code).collect(),
        None => vec![file],
    };
    slices
        .into_iter()
        .map(|code| {
            let mut header = Reader::at(code, 0);
            let (magic, cpu) = (header.u32(), header.u32());
            header.skip(4); // The CPU subtype.
            let kind = header.u32();
            if header.overrun() || !matches!(magic, MH_MAGIC | MH_MAGIC_64) {
                bail!("it isn't a Mach-O file");
            }
            Ok((cpu, kind))
        })
        .collect()
}

/// A CPU type's name, as Xcode's settings and `lipo` name it.
pub fn cpu_name(cpu: u32) -> String {
    match cpu {
        CPU_TYPE_ARM64 => "arm64".to_owned(),
        CPU_TYPE_ARM64_32 => "arm64_32".to_owned(),
        CPU_TYPE_X86_64 => "x86_64".to_owned(),
        _ => format!("CPU type {cpu:#x}"),
    }
}

/// One architecture's Mach-O file, and where it has what signing reads and
/// changes.
pub struct Architecture<'a> {
    macho: &'a [u8],
    layout: &'static Layout,
    cpu: u32,
    /// The `__TEXT` segment's offset in the file, and its size.
    text: (u64, u64),
    /// Where the `__LINKEDIT` segment's load command is, and the
    /// `LC_CODE_SIGNATURE` one, and where the signature starts.
    linkedit: usize,
    signature: usize,
    data_offset: usize,
    /// The earliest version of its OS it runs on, if it says.
    min_os: Option<u32>,
}

impl<'a> Architecture<'a> {
    /// Reads an architecture's Mach-O file, which has to have a signature
    /// at its end, to replace.
    fn read(macho: &'a [u8]) -> Result<Self> {
        let mut header = Reader::at(macho, 0);
        let magic = header.u32();
        let cpu = header.u32();
        let layout = match magic {
            MH_MAGIC_64 => &MACHO_64,
            // Only arm64_32's: OmniBuild signs no other 32-bit code.
            MH_MAGIC if cpu == CPU_TYPE_ARM64_32 => &MACHO_32,
            _ => bail!("it isn't a 64-bit Mach-O executable, nor one for arm64_32"),
        };
        header.skip(8); // The CPU subtype and file type.
        let commands = header.u32();

        let (mut text, mut linkedit, mut signature, mut min_os) = (None, None, None, None);
        let mut at = layout.header_size;
        for _ in 0..commands {
            let mut command = Reader::at(macho, at);
            let (kind, size) = (command.u32(), command.u32() as usize);
            let name = command.bytes(16);
            let segment = || {
                let offset = at + layout.file_offset();
                (
                    layout.read(macho, offset),
                    layout.read(macho, offset + layout.word),
                )
            };
            match (kind == layout.segment, kind, name) {
                (true, _, b"__TEXT\0\0\0\0\0\0\0\0\0\0") => text = Some(segment()),
                (true, _, b"__LINKEDIT\0\0\0\0\0\0") => linkedit = Some(at),
                (_, LC_CODE_SIGNATURE, _) => signature = Some(at),
                (_, LC_BUILD_VERSION, _) => min_os = Some(Reader::at(macho, at + 12).u32()),
                (_, LC_VERSION_MIN_WATCHOS, _) => {
                    min_os = Some(Reader::at(macho, at + 8).u32());
                }
                _ => {}
            }
            // A command can't be shorter than its kind and size, and one
            // that was would have the loop read the same command again.
            if command.overrun() || size < 8 {
                bail!("its load commands are truncated");
            }
            at += size;
        }
        let (Some(text), Some(linkedit), Some(signature)) = (text, linkedit, signature) else {
            bail!(
                "it has no code signature to replace. Build it signed, if only ad hoc (-Wl,-adhoc_codesign)"
            );
        };

        let mut fields = Reader::at(macho, signature + 8);
        let (data_offset, data_size) = (fields.u32() as usize, fields.u32() as usize);
        let file_offset = layout.read(macho, linkedit + layout.file_offset());
        let file_size = layout.read(macho, linkedit + layout.file_size());
        if data_offset.saturating_add(data_size) != macho.len()
            || file_offset.saturating_add(file_size) != macho.len() as u64
        {
            bail!("its code signature isn't at its end, where it's replaced");
        }
        Ok(Self {
            macho,
            layout,
            cpu,
            text,
            linkedit,
            signature,
            data_offset,
            min_os,
        })
    }

    /// Whether it's `arm64_32` code for watchOS before 12, which codesign
    /// signs with SHA-1 hashes too. Code that doesn't say which version it's
    /// for gets them too: an OS that reads SHA-256 hashes takes those, and
    /// one that doesn't has the others.
    fn is_for_old_watchos(&self) -> bool {
        self.cpu == CPU_TYPE_ARM64_32
            && self
                .min_os
                .is_none_or(|version| version < SHA256_ONLY_WATCHOS)
    }

    /// The file, signed with SHA-1 hashes as well as SHA-256 ones if
    /// `sha1`.
    fn sign(&self, sha1: bool, sign: &impl Fn(&Code) -> Vec<u8>) -> Vec<u8> {
        let (layout, linkedit, data_offset) = (self.layout, self.linkedit, self.data_offset);
        let vm_size = layout.read(self.macho, linkedit + layout.vm_size());
        let file_offset = layout.read(self.macho, linkedit + layout.file_offset());
        // arm64_32's too, as watchOS maps it.
        let page_size = if matches!(self.cpu, CPU_TYPE_ARM64 | CPU_TYPE_ARM64_32) {
            14
        } else {
            12
        };
        let mut code = self.macho[..data_offset].to_vec();
        // The signature's size depends only on the code's, so a first
        // signature says how much room the load commands give it.
        let first = sign(&Code {
            bytes: &code,
            page_size,
            text: self.text,
            sha1,
        });
        let code_directories = if sha1 { 2 } else { 1 };
        let size = (first.len() + SIGNATURE_SLACK * code_directories)
            .next_multiple_of(SIGNATURE_ALIGNMENT);
        let file_size = (data_offset + size) as u64 - file_offset.min(data_offset as u64);
        let vm_size = vm_size.max(file_size.next_multiple_of(LINKEDIT_ROUNDING));
        code[self.signature + 12..self.signature + 16].copy_from_slice(&u32_of(size).to_le_bytes());
        layout.write(&mut code, linkedit + layout.vm_size(), vm_size);
        layout.write(&mut code, linkedit + layout.file_size(), file_size);

        let signed = sign(&Code {
            bytes: &code,
            page_size,
            text: self.text,
            sha1,
        });
        code.extend(signed);
        code.resize(data_offset + size, 0);
        code
    }
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("executables are under 4 GiB")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::Result;

    use super::{Code, Executable, MH_DYLIB, MH_MAGIC, architectures, cpu_name};

    /// A program's file type.
    const MH_EXECUTE: u32 = 0x2;
    use crate::bytes::Reader;
    use crate::ios::tests::{device_file, shell_file};

    /// The executable, read and signed with `sign`.
    fn sign(executable: &[u8], sign: impl Fn(&Code) -> Vec<u8>) -> Result<Vec<u8>> {
        Executable::read(executable).map(|executable| executable.sign(sign))
    }

    /// Whether the executable is signed with SHA-1 hashes too.
    fn needs_sha1(executable: &[u8]) -> Result<bool> {
        Executable::read(executable).map(|executable| executable.needs_sha1())
    }

    /// The shells' executables: tiny programs, built as tests/fixtures/README.md
    /// says, signed by the linker.
    fn simulator() -> Vec<u8> {
        shell_file("OmniBuildShell.app/OmniBuildShell")
    }

    fn device() -> Vec<u8> {
        device_file("Payload/OmniBuildShell.app/OmniBuildShell")
    }

    /// The device shell's watch app's: arm64 code, then `arm64_32`'s for
    /// watchOS 11.
    fn watch() -> Vec<u8> {
        device_file("Payload/OmniBuildShell.app/Watch/ShellWatch.app/ShellWatch")
    }

    /// Its widgets': the same, for watchOS 26.
    fn widgets() -> Vec<u8> {
        device_file(
            "Payload/OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets",
        )
    }

    /// `bytes` with `value` at `at`.
    fn set(bytes: &[u8], at: usize, value: &[u8]) -> Vec<u8> {
        let mut bytes = bytes.to_vec();
        bytes[at..at + value.len()].copy_from_slice(value);
        bytes
    }

    /// A signature of 100 bytes of the code's first byte, its page size and
    /// its `__TEXT` size, so the tests see what each call was given.
    fn stub(code: &Code) -> Vec<u8> {
        let mut signature = vec![code.bytes[0]; 98];
        signature.push(code.page_size);
        signature.push(u8::try_from(code.text.1 >> 12).unwrap());
        signature
    }

    /// Where `__LINKEDIT`'s and `LC_CODE_SIGNATURE`'s load commands are.
    fn commands(macho: &[u8]) -> (usize, usize) {
        let (mut linkedit, mut signature, mut at) = (0, 0, 32);
        for _ in 0..Reader::at(macho, 16).u32() {
            let mut command = Reader::at(macho, at);
            let (kind, size) = (command.u32(), command.u32() as usize);
            if command.bytes(10) == b"__LINKEDIT" {
                linkedit = at;
            }
            if kind == 0x1d {
                signature = at;
            }
            at += size;
        }
        (linkedit, signature)
    }

    /// The `LC_CODE_SIGNATURE` offset and size, and `__LINKEDIT`'s virtual
    /// and file sizes.
    fn layout(macho: &[u8]) -> (usize, usize, u64, u64) {
        let (linkedit, signature) = commands(macho);
        let mut fields = Reader::at(macho, signature + 8);
        let (offset, size) = (fields.u32() as usize, fields.u32() as usize);
        let mut segment = Reader::at(macho, linkedit + 32);
        let vm_size = segment.u64();
        segment.skip(8);
        (offset, size, vm_size, segment.u64())
    }

    #[test]
    fn replaces_the_signature_at_the_end_of_an_executable() {
        let device = device();

        let signed = sign(&device, stub).unwrap();

        // The linker's signature was 408 bytes at 32,944; the new one is
        // 100 bytes, padded to 112.
        assert_eq!(
            layout(&signed),
            (32_944, 112, 16_384, 32_944 + 112 - 32_768)
        );
        assert_eq!(signed.len(), 32_944 + 112);
        // Everything else up to the signature is as it was.
        assert_eq!(commands(&device), (336, 784));
        assert_eq!(signed[..336 + 32], device[..336 + 32]);
        assert_eq!(signed[336 + 56..784 + 12], device[336 + 56..784 + 12]);
        assert_eq!(signed[800..32_944], device[800..32_944]);
        // Hashed in 16 KiB pages, with the 32 KiB __TEXT segment.
        assert_eq!(
            signed[32_944..32_944 + 100],
            [&[0xcf; 98][..], &[14, 8]].concat()
        );
        assert_eq!(signed[32_944 + 100..], [0; 12]);
    }

    #[test]
    fn signs_each_architecture_of_a_universal_executable() {
        let simulator = simulator();

        let signed = sign(&simulator, stub).unwrap();

        let mut header = Reader::at(&signed, 0);
        assert_eq!((header.u32_be(), header.u32_be()), (0xcafe_babe, 2));
        let mut architectures = Vec::new();
        for _ in 0..2 {
            let (cpu, _) = (header.u32_be(), header.u32_be());
            let (offset, size) = (header.u32_be() as usize, header.u32_be() as usize);
            architectures.push((cpu, offset, size, header.u32_be()));
        }
        // Each on a 16 KiB boundary, as codesign lays them out, although the
        // linker aligned x86_64 to 4 KiB.
        assert_eq!(
            architectures,
            [
                (0x0100_0007, 16_384, 4272 + 112, 14),
                (0x0100_000c, 32_768, 16_560 + 112, 14)
            ]
        );
        assert_eq!(signed.len(), 32_768 + 16_560 + 112);
        let x86_64 = &signed[16_384..16_384 + 4272 + 112];
        let arm64 = &signed[32_768..];
        // __LINKEDIT rounded up to 16 KiB in memory for both.
        assert_eq!(layout(x86_64), (4272, 112, 16_384, 4272 + 112 - 4096));
        assert_eq!(layout(arm64), (16_560, 112, 16_384, 16_560 + 112 - 16_384));
        // 4 KiB pages for x86_64 and its 4 KiB __TEXT, 16 KiB for arm64.
        assert_eq!(x86_64[4272 + 98..4272 + 100], [12, 1]);
        assert_eq!(arm64[16_560 + 98..16_560 + 100], [14, 4]);
        assert!(signed[..16_384][48..].iter().all(|&byte| byte == 0));
        assert!(
            signed[16_384 + 4272 + 112..32_768]
                .iter()
                .all(|&byte| byte == 0)
        );
    }

    #[test]
    fn signs_a_watch_apps_arm64_32_code_too() {
        let watch = widgets();

        let signed = sign(&watch, stub).unwrap();

        // After the arm64 code, on the next 16 KiB boundary.
        let mut entry = Reader::at(&signed, 8 + 20);
        let cpu = entry.u32_be();
        entry.skip(4);
        assert_eq!((cpu, entry.u32_be()), (0x0200_000c, 65_536));
        let (arm64_32, original) = (&signed[65_536..], &watch[65_536..]);
        assert_eq!(arm64_32.len(), 32_944 + 112);
        // Its 32-bit load commands pointed at the new signature, 112 bytes
        // at 32,944: LC_CODE_SIGNATURE's size, and __LINKEDIT's sizes in
        // memory and in the file, each a 32-bit word.
        assert_eq!(Reader::at(arm64_32, 696 + 12).u32(), 112);
        let mut linkedit = Reader::at(arm64_32, 276 + 28);
        let (vm_size, file_offset) = (linkedit.u32(), linkedit.u32());
        assert_eq!(
            (vm_size, file_offset, linkedit.u32()),
            (16_384, 32_768, 32_944 + 112 - 32_768)
        );
        // Everything else up to the signature is as it was.
        assert_eq!(arm64_32[..276 + 28], original[..276 + 28]);
        assert_eq!(arm64_32[276 + 40..696 + 12], original[276 + 40..696 + 12]);
        assert_eq!(arm64_32[696 + 16..32_944], original[696 + 16..32_944]);
        // Hashed in 16 KiB pages, as watchOS maps them, with the 32 KiB
        // __TEXT segment.
        assert_eq!(
            arm64_32[32_944..32_944 + 100],
            [&[0xce; 98][..], &[14, 8]].concat()
        );
    }

    #[test]
    fn hashes_code_for_watchos_before_12_with_sha1_too() {
        let widgets = widgets();
        let watchos_11 = (11_u32 << 16).to_le_bytes();
        let cases = [
            // The watch app's: arm64 code, which watchOS runs from 26, and
            // arm64_32 code for watchOS 11, which has every architecture
            // hashed with SHA-1 too.
            (watch(), true),
            (widgets.clone(), false),
            // The widgets' arm64_32 code for watchOS 11, in
            // LC_BUILD_VERSION, then in LC_VERSION_MIN_WATCHOS, whose
            // version is where LC_BUILD_VERSION has its platform (watchOS,
            // 4), and with neither.
            (set(&widgets, 65_536 + 520 + 12, &watchos_11), true),
            (set(&widgets, 65_536 + 520, &[0x30]), true),
            (set(&widgets, 65_536 + 520, &[0x2a]), true),
            // Its arm64 code alone for watchOS 11, which codesign hashes
            // with SHA-256 alone.
            (set(&widgets, 16_384 + 600 + 12, &watchos_11), false),
            (device(), false),
            (simulator(), false),
        ];

        for (executable, sha1) in cases {
            let calls = RefCell::new(Vec::new());

            let signed = sign(&executable, |code: &Code| {
                calls.borrow_mut().push(code.sha1);
                stub(code)
            })
            .unwrap();

            assert_eq!(needs_sha1(&executable).unwrap(), sha1);
            // Twice for each architecture.
            assert!(calls.borrow().len() >= 2);
            assert!(calls.borrow().iter().all(|&call| call == sha1));
            // Room for 8 bytes more for each code directory, padded to 16.
            let room = if sha1 { 128 } else { 112 };
            let code = if signed.starts_with(&[0xca, 0xfe]) {
                16_384
            } else {
                0
            };
            assert_eq!(layout(&signed[code..]).1, room);
        }
        assert!(needs_sha1(b"not an executable").is_err());
    }

    #[test]
    fn grows_linkedit_for_a_signature_that_needs_more_pages() {
        let signed = sign(&device(), |_: &Code| vec![1; 20_000]).unwrap();

        // 176 bytes of __LINKEDIT before the signature, then room for it,
        // 8 bytes more padded to 16, which the 16 KiB in memory don't hold.
        assert_eq!(layout(&signed), (32_944, 20_016, 32_768, 176 + 20_016));
    }

    #[test]
    fn reads_the_architectures_of_a_program_or_a_library() {
        let (arm64, x86_64) = (0x0100_000c, 0x0100_0007);

        assert_eq!(architectures(&device()).unwrap(), [(arm64, MH_EXECUTE)]);
        assert_eq!(
            architectures(&simulator()).unwrap(),
            [(x86_64, MH_EXECUTE), (arm64, MH_EXECUTE)]
        );
        assert_eq!(
            architectures(&shell_file(
                "OmniBuildShell.app/Frameworks/ShellKit.framework/ShellKit"
            ))
            .unwrap()
            .iter()
            .map(|&(cpu, kind)| (cpu_name(cpu), kind))
            .collect::<Vec<_>>(),
            [
                ("x86_64".to_owned(), MH_DYLIB),
                ("arm64".to_owned(), MH_DYLIB)
            ]
        );
        // A 32-bit file's header, for x86.
        let x86 = [MH_MAGIC, 7, 3, MH_DYLIB].map(u32::to_le_bytes).concat();
        assert_eq!(architectures(&x86).unwrap(), [(7, MH_DYLIB)]);
        assert_eq!(cpu_name(0x0200_000c), "arm64_32");
        assert_eq!(cpu_name(7), "CPU type 0x7");
        for (file, message) in [
            (b"!<arch>\n".to_vec(), "it isn't a Mach-O file"),
            (b"a text file, at length".to_vec(), "it isn't a Mach-O file"),
            (device()[..12].to_vec(), "it isn't a Mach-O file"),
            (simulator()[..30].to_vec(), "it's truncated"),
        ] {
            assert_eq!(architectures(&file).unwrap_err().to_string(), message);
        }
    }

    #[test]
    fn reports_an_executable_it_cant_sign() {
        let device = device();
        let simulator = simulator();
        let cases = [
            (
                b"not an executable".to_vec(),
                "it isn't a 64-bit Mach-O executable, nor one for arm64_32",
            ),
            (
                // A 32-bit file for another architecture: x86.
                [&MH_MAGIC.to_le_bytes()[..], &7_u32.to_le_bytes()].concat(),
                "it isn't a 64-bit Mach-O executable, nor one for arm64_32",
            ),
            (
                // Its last load command, LC_CODE_SIGNATURE, made one of no size.
                set(&device, 788, &[0; 4]),
                "its load commands are truncated",
            ),
            (device[..700].to_vec(), "its load commands are truncated"),
            (
                set(&device, 784, &[0; 4]),
                "it has no code signature to replace. Build it signed, if only ad hoc (-Wl,-adhoc_codesign)",
            ),
            (
                [&device[..], &[0]].concat(),
                "its code signature isn't at its end, where it's replaced",
            ),
            (
                // __LINKEDIT's file size, short of the signature's end.
                set(&device, 336 + 48, &[0; 8]),
                "its code signature isn't at its end, where it's replaced",
            ),
            (simulator[..30].to_vec(), "it's truncated"),
            (simulator[..20_000].to_vec(), "it's truncated"),
            (
                // The second architecture's code, made not Mach-O.
                set(&simulator, 16_384, &[0; 4]),
                "it isn't a 64-bit Mach-O executable, nor one for arm64_32",
            ),
        ];

        for (executable, message) in cases {
            let error = sign(&executable, stub).unwrap_err();

            assert_eq!(error.to_string(), message);
        }
    }
}
