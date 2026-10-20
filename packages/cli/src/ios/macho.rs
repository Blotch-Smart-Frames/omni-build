//! Mach-O executables, as far as signing them goes. An iOS app's
//! executable is a 64-bit Mach-O file for one architecture, or, for the
//! Simulator, a universal file with one for each, which are each signed on
//! their own.
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

const MH_MAGIC_64: u32 = 0xfeed_facf;
/// A universal file's header is big-endian.
const FAT_MAGIC: u32 = 0xcafe_babe;
/// The header of a 64-bit Mach-O file, which the load commands follow.
const HEADER_SIZE: usize = 32;
/// A universal file's header, and the size of each architecture's entry.
const FAT_HEADER_SIZE: usize = 8;
const FAT_ARCH_SIZE: usize = 20;
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
/// Where a segment load command has its virtual size, file offset and file
/// size.
const SEGMENT_VM_SIZE: usize = 32;
const SEGMENT_FILE_OFFSET: usize = 40;
const SEGMENT_FILE_SIZE: usize = 48;
/// codesign leaves room for a signature 8 bytes longer than the one it
/// writes, padded to a multiple of 16 bytes, as measured signing a library
/// under identifiers of every length from 1 to 39.
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
}

/// The executable, each architecture with the signature `sign` makes for
/// its code in place of the one it had.
pub fn sign(executable: &[u8], sign: impl Fn(&Code) -> Vec<u8>) -> Result<Vec<u8>> {
    if Reader::at(executable, 0).u32_be() == FAT_MAGIC {
        sign_universal(executable, &sign)
    } else {
        sign_architecture(executable, &sign)
    }
}

/// A universal file: each architecture signed, then laid out again, since
/// signing changes their sizes.
fn sign_universal(executable: &[u8], sign: &impl Fn(&Code) -> Vec<u8>) -> Result<Vec<u8>> {
    let mut header = Reader::at(executable, 4);
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
        let code = executable
            .get(offset..offset.saturating_add(size))
            .context("it's truncated")?;
        architectures.push((cpu, subtype, sign_architecture(code, sign)?));
    }

    let mut out = vec![0; FAT_HEADER_SIZE + FAT_ARCH_SIZE * count];
    out[..4].copy_from_slice(&FAT_MAGIC.to_be_bytes());
    out[4..8].copy_from_slice(&u32_of(count).to_be_bytes());
    for (index, (cpu, subtype, code)) in architectures.into_iter().enumerate() {
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
    Ok(out)
}

/// One architecture's Mach-O file, signed.
fn sign_architecture(macho: &[u8], sign: &impl Fn(&Code) -> Vec<u8>) -> Result<Vec<u8>> {
    let mut header = Reader::at(macho, 0);
    if header.u32() != MH_MAGIC_64 {
        bail!("it isn't a 64-bit Mach-O executable");
    }
    let cpu = header.u32();
    header.skip(8); // The CPU subtype and file type.
    let commands = header.u32();

    let (mut text, mut linkedit, mut signature) = (None, None, None);
    let mut at = HEADER_SIZE;
    for _ in 0..commands {
        let mut command = Reader::at(macho, at);
        let (kind, size) = (command.u32(), command.u32() as usize);
        let name = command.bytes(16);
        let segment = || {
            let mut segment = Reader::at(macho, at + SEGMENT_FILE_OFFSET);
            (segment.u64(), segment.u64())
        };
        match (kind, name) {
            (LC_SEGMENT_64, b"__TEXT\0\0\0\0\0\0\0\0\0\0") => text = Some(segment()),
            (LC_SEGMENT_64, b"__LINKEDIT\0\0\0\0\0\0") => linkedit = Some(at),
            (LC_CODE_SIGNATURE, _) => signature = Some(at),
            _ => {}
        }
        // A command can't be shorter than its kind and size, and one that
        // was would have the loop read the same command again.
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
    let mut segment = Reader::at(macho, linkedit + SEGMENT_VM_SIZE);
    let vm_size = segment.u64();
    let (file_offset, file_size) = (segment.u64(), segment.u64());
    if data_offset.saturating_add(data_size) != macho.len()
        || file_offset.saturating_add(file_size) != macho.len() as u64
    {
        bail!("its code signature isn't at its end, where it's replaced");
    }

    let page_size = if cpu == CPU_TYPE_ARM64 { 14 } else { 12 };
    let mut code = macho[..data_offset].to_vec();
    // The signature's size depends only on the code's, so a first signature
    // says how much room the load commands give it.
    let first = sign(&Code {
        bytes: &code,
        page_size,
        text,
    });
    let size = (first.len() + SIGNATURE_SLACK).next_multiple_of(SIGNATURE_ALIGNMENT);
    let file_size = (data_offset + size) as u64 - file_offset.min(data_offset as u64);
    let vm_size = vm_size.max(file_size.next_multiple_of(LINKEDIT_ROUNDING));
    code[signature + 12..signature + 16].copy_from_slice(&u32_of(size).to_le_bytes());
    code[linkedit + SEGMENT_VM_SIZE..linkedit + SEGMENT_VM_SIZE + 8]
        .copy_from_slice(&vm_size.to_le_bytes());
    code[linkedit + SEGMENT_FILE_SIZE..linkedit + SEGMENT_FILE_SIZE + 8]
        .copy_from_slice(&file_size.to_le_bytes());

    let signed = sign(&Code {
        bytes: &code,
        page_size,
        text,
    });
    code.extend(signed);
    code.resize(data_offset + size, 0);
    Ok(code)
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("executables are under 4 GiB")
}

#[cfg(test)]
mod tests {
    use super::{Code, sign};
    use crate::bytes::Reader;
    use crate::ios::tests::{device_file, shell_file};

    /// The shells' executables: tiny programs, built as tests/fixtures/README.md
    /// says, signed by the linker.
    fn simulator() -> Vec<u8> {
        shell_file("OmniBuildShell.app/OmniBuildShell")
    }

    fn device() -> Vec<u8> {
        device_file("Payload/OmniBuildShell.app/OmniBuildShell")
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
    fn grows_linkedit_for_a_signature_that_needs_more_pages() {
        let signed = sign(&device(), |_: &Code| vec![1; 20_000]).unwrap();

        // 176 bytes of __LINKEDIT before the signature, then room for it,
        // 8 bytes more padded to 16, which the 16 KiB in memory don't hold.
        assert_eq!(layout(&signed), (32_944, 20_016, 32_768, 176 + 20_016));
    }

    #[test]
    fn reports_an_executable_it_cant_sign() {
        let device = device();
        let simulator = simulator();
        let set = |bytes: &[u8], at: usize, value: &[u8]| {
            let mut bytes = bytes.to_vec();
            bytes[at..at + value.len()].copy_from_slice(value);
            bytes
        };
        let cases = [
            (
                b"not an executable".to_vec(),
                "it isn't a 64-bit Mach-O executable",
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
                "it isn't a 64-bit Mach-O executable",
            ),
        ];

        for (executable, message) in cases {
            let error = sign(&executable, stub).unwrap_err();

            assert_eq!(error.to_string(), message);
        }
    }
}
