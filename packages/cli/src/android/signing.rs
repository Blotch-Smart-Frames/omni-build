//! APK Signature Schemes v2 and v3
//! (source.android.com/docs/security/features/apksigning).
//!
//! Both sign a digest of the whole file except the signature itself, which
//! goes in an APK Signing Block between the zip entries and the central
//! directory. Android 7 to 8.1 verify v2, and Android 9 and later v3, which
//! adds key rotation. OmniBuild writes both, as apksigner does, and no v1
//! (JAR) signature: that's only for Android 6 and earlier.

use sha2::{Digest, Sha256};

use super::key::SigningKey;
use crate::zip::Written;

/// Signature algorithm 0x0103: RSASSA-PKCS1-v1_5 with SHA2-256, over a
/// SHA2-256 digest of the contents in 1 MiB chunks.
const RSA_PKCS1_SHA256: u32 = 0x0103;
const V2_BLOCK: u32 = 0x7109_871a;
const V3_BLOCK: u32 = 0xf053_68c0;
/// An attribute in the v2 signer that says a v3 signature is there too, so
/// that Android 9 and later reject the APK if someone strips it to get past
/// v3's checks.
const STRIPPING_PROTECTION: u32 = 0xbeef_f00d;
const V3_SCHEME: u32 = 3;
/// The Android versions the v3 signer covers: from 9, the first to verify
/// v3, through every later one.
const V3_MIN_SDK: u32 = 28;
const V3_MAX_SDK: u32 = i32::MAX.unsigned_abs();
const CHUNK: usize = 1024 * 1024;
const MAGIC: &[u8; 16] = b"APK Sig Block 42";

/// Signs an APK the zip module wrote, returning the signed file.
pub fn sign(apk: &Written, key: &SigningKey) -> Vec<u8> {
    let bytes = &apk.bytes;
    let digest = content_digest([
        &bytes[..apk.central_directory],
        &bytes[apk.central_directory..apk.end],
        &bytes[apk.end..],
    ]);
    let digests = sequence([[
        &RSA_PKCS1_SHA256.to_le_bytes()[..],
        &length_prefixed(&digest),
    ]
    .concat()]);
    let certificates = sequence([key.certificate().to_vec()]);

    let v2_attributes =
        sequence([[STRIPPING_PROTECTION.to_le_bytes(), V3_SCHEME.to_le_bytes()].concat()]);
    let v2_signed_data = [&digests[..], &certificates, &v2_attributes].concat();
    let v2_signer = [
        &length_prefixed(&v2_signed_data)[..],
        &signatures(key, &v2_signed_data),
        &length_prefixed(key.public_key()),
    ]
    .concat();

    let sdk_range = [V3_MIN_SDK.to_le_bytes(), V3_MAX_SDK.to_le_bytes()].concat();
    let v3_signed_data = [&digests[..], &certificates, &sdk_range, &sequence([])].concat();
    let v3_signer = [
        &length_prefixed(&v3_signed_data)[..],
        &sdk_range,
        &signatures(key, &v3_signed_data),
        &length_prefixed(key.public_key()),
    ]
    .concat();

    let block = signing_block(&[
        (V2_BLOCK, sequence([v2_signer])),
        (V3_BLOCK, sequence([v3_signer])),
    ]);
    // The end record now finds the central directory after the block.
    let mut end = bytes[apk.end..].to_vec();
    let central_directory = u32_of(apk.central_directory + block.len());
    end[16..20].copy_from_slice(&central_directory.to_le_bytes());
    [
        &bytes[..apk.central_directory],
        &block,
        &bytes[apk.central_directory..apk.end],
        &end,
    ]
    .concat()
}

/// The digest both schemes sign: of the zip entries, the central directory
/// and the end record, each split into 1 MiB chunks. The end record's
/// central directory offset is the one without a signing block, which is
/// what the unsigned archive has.
fn content_digest(sections: [&[u8]; 3]) -> [u8; 32] {
    let chunks: Vec<&[u8]> = sections
        .iter()
        .flat_map(|section| section.chunks(CHUNK))
        .collect();
    let mut digest = Sha256::new();
    digest.update([0x5a]);
    digest.update(u32_of(chunks.len()).to_le_bytes());
    for chunk in chunks {
        let mut chunk_digest = Sha256::new();
        chunk_digest.update([0xa5]);
        chunk_digest.update(u32_of(chunk.len()).to_le_bytes());
        chunk_digest.update(chunk);
        digest.update(chunk_digest.finalize());
    }
    digest.finalize().into()
}

fn signatures(key: &SigningKey, signed_data: &[u8]) -> Vec<u8> {
    sequence([[
        &RSA_PKCS1_SHA256.to_le_bytes()[..],
        &length_prefixed(&key.sign(signed_data)),
    ]
    .concat()])
}

/// The block's size, its ID-value pairs, the size again, and the magic.
/// The sizes are 64-bit and leave out the first size field itself.
fn signing_block(pairs: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let pairs: Vec<u8> = pairs
        .iter()
        .flat_map(|(id, value)| {
            [
                &u64_of(4 + value.len()).to_le_bytes()[..],
                &id.to_le_bytes(),
                value,
            ]
            .concat()
        })
        .collect();
    let size = u64_of(pairs.len() + 8 + MAGIC.len()).to_le_bytes();
    [&size[..], &pairs, &size, MAGIC].concat()
}

/// The bytes after their length, as a 32-bit little-endian integer: how
/// every field of a signer is written.
fn length_prefixed(bytes: &[u8]) -> Vec<u8> {
    [&u32_of(bytes.len()).to_le_bytes()[..], bytes].concat()
}

/// Length-prefixed items, the whole also length-prefixed.
fn sequence(items: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    length_prefixed(
        &items
            .into_iter()
            .flat_map(|item| length_prefixed(&item))
            .collect::<Vec<_>>(),
    )
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("APKs are under 4 GiB")
}

fn u64_of(value: usize) -> u64 {
    u64::try_from(value).expect("usize fits in 64 bits")
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{CHUNK, content_digest, sign};
    use crate::android::key::tests::{KEY, verifies};
    use crate::bytes::Reader;
    use crate::zip::{self, Method, tests::entry};

    /// One length-prefixed field of a signer.
    fn field<'a>(reader: &mut Reader<'a>) -> &'a [u8] {
        let len = reader.u32() as usize;
        reader.bytes(len)
    }

    /// The items of a length-prefixed sequence's contents.
    fn items(sequence: &[u8]) -> Vec<&[u8]> {
        let mut reader = Reader::at(sequence, 0);
        let mut items = Vec::new();
        let mut read = 0;
        while read < sequence.len() {
            let item = field(&mut reader);
            read += 4 + item.len();
            items.push(item);
        }
        assert!(!reader.overrun());
        items
    }

    fn sha256(parts: &[&[u8]]) -> Vec<u8> {
        let mut digest = Sha256::new();
        for part in parts {
            digest.update(part);
        }
        digest.finalize().to_vec()
    }

    #[test]
    fn digests_each_mebibyte_chunk_of_each_section() {
        let large = vec![7; CHUNK + 1];

        let digest = content_digest([&large, b"", b"end"]);

        let chunk_digests: Vec<Vec<u8>> = [&large[..CHUNK], &large[CHUNK..], b"end"]
            .iter()
            .map(|chunk| {
                let len = u32::try_from(chunk.len()).unwrap();
                sha256(&[&[0xa5], &len.to_le_bytes(), chunk])
            })
            .collect();
        let expected = sha256(
            &[&[0x5a][..], &3u32.to_le_bytes()]
                .into_iter()
                .chain(chunk_digests.iter().map(Vec::as_slice))
                .collect::<Vec<_>>(),
        );
        assert_eq!(digest.to_vec(), expected);
    }

    #[test]
    fn signs_with_v2_and_v3_in_front_of_the_central_directory() {
        let unsigned = zip::write(&[
            entry("AndroidManifest.xml", Method::Deflated, b"manifest"),
            entry("resources.arsc", Method::Stored, b"table"),
        ]);

        let apk = sign(&unsigned, &KEY);

        // Still a zip archive with the same entries.
        assert_eq!(
            zip::read(&apk).unwrap(),
            zip::read(&unsigned.bytes).unwrap()
        );
        // The block, from the end record's central directory offset back.
        let end = apk.len() - 22;
        let central_directory = Reader::at(&apk, end + 16).u32() as usize;
        assert_eq!(
            &apk[central_directory - 16..central_directory],
            b"APK Sig Block 42"
        );
        let start = unsigned.central_directory;
        let mut block = Reader::at(&apk[..central_directory - 16], start);
        let size = block.bytes(8);
        assert_eq!(
            usize::try_from(u64::from_le_bytes(size.try_into().unwrap())).unwrap(),
            central_directory - start - 8
        );

        let expected_digest = content_digest([
            &unsigned.bytes[..start],
            &unsigned.bytes[start..unsigned.end],
            &unsigned.bytes[unsigned.end..],
        ]);
        let mut ids = Vec::new();
        for _ in 0..2 {
            let pair_len =
                usize::try_from(u64::from_le_bytes(block.bytes(8).try_into().unwrap())).unwrap();
            let mut pair = Reader::at(block.bytes(pair_len), 0);
            let id = pair.u32();
            ids.push(id);
            let signers = items(field(&mut pair));
            assert_eq!(signers.len(), 1);

            let mut signer = Reader::at(signers[0], 0);
            let signed_data = field(&mut signer);
            if id == super::V3_BLOCK {
                assert_eq!(signer.u32(), 28);
                assert_eq!(signer.u32(), i32::MAX as u32);
            }
            let signatures = items(field(&mut signer));
            let public_key = field(&mut signer);
            assert_eq!(public_key, KEY.public_key());

            let mut data = Reader::at(signed_data, 0);
            let digests = items(field(&mut data));
            let mut digest = Reader::at(digests[0], 0);
            assert_eq!(digest.u32(), 0x0103);
            assert_eq!(field(&mut digest), expected_digest);
            assert_eq!(items(field(&mut data)), [KEY.certificate()]);
            let attributes: &[&[u8]] = if id == super::V3_BLOCK {
                assert_eq!((data.u32(), data.u32()), (28, i32::MAX as u32));
                &[]
            } else {
                // Stripping protection, naming scheme v3.
                &[&[0x0d, 0xf0, 0xef, 0xbe, 3, 0, 0, 0]]
            };
            assert_eq!(items(field(&mut data)), attributes);

            let mut signature = Reader::at(signatures[0], 0);
            assert_eq!(signature.u32(), 0x0103);
            assert!(verifies(public_key, signed_data, field(&mut signature)));
        }
        assert_eq!(ids, [super::V2_BLOCK, super::V3_BLOCK]);
    }
}
