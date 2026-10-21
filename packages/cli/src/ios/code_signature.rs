//! Apple's code signature for one architecture of an executable, as
//! codesign signs an iOS app. Apple documents the format only in the
//! headers of its open-source Security framework (`cscdefs.h`); what
//! OmniBuild writes was checked against what codesign writes for the same
//! app, which the tests compare it with.
//!
//! The signature is a "super blob" of blobs, each a magic number, its
//! length and its contents, all big-endian:
//!
//! - The code directory: the app's identifier and team, and a SHA-256
//!   hash of each page of the code and of each "special slot": Info.plist,
//!   the requirements, the resource seal (`CodeResources`) and the
//!   entitlements. Its own hash, the cdhash, is what iOS knows the app by.
//!   Code for older systems has a second one, of SHA-1 hashes, which comes
//!   first, and the SHA-256 one after the others, as an alternate.
//! - The requirements: which signatures count as this app's
//!   ("designated"), by its identifier and its certificate.
//! - The entitlements, as XML and as DER.
//! - A CMS signature of the code directory with the certificate's key.
//!   An ad hoc signature, which only the Simulator runs, has none.

use std::time::SystemTime;

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use const_oid::ObjectIdentifier;
use const_oid::db::rfc4519::{CN, O, OU};
use const_oid::db::rfc5911::{
    ID_CONTENT_TYPE, ID_DATA, ID_MESSAGE_DIGEST, ID_SIGNED_DATA, ID_SIGNING_TIME,
};
use const_oid::db::rfc5912::{ID_SHA_1, ID_SHA_256, SHA_256_WITH_RSA_ENCRYPTION};
use plist::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use x509_cert::der::asn1::{OctetString, SetOfVec, UtcTime};
use x509_cert::der::{Any, Decode, Encode, Tag};
use x509_cert::name::Name;
use x509_cert::spki::AlgorithmIdentifierOwned;

use super::entitlements::{Entitlement, Entitlements};
use super::macho::Code;
use crate::key::SigningKey;

const EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
const CODE_DIRECTORY: u32 = 0xfade_0c02;
const REQUIREMENTS: u32 = 0xfade_0c01;
const REQUIREMENT: u32 = 0xfade_0c00;
const ENTITLEMENTS: u32 = 0xfade_7171;
const DER_ENTITLEMENTS: u32 = 0xfade_7172;
const BLOB_WRAPPER: u32 = 0xfade_0b01;

/// Each blob's slot in the super blob's index. A code directory beside the
/// first, of other hashes, is an alternate, from slot 0x1000.
const CODE_DIRECTORY_SLOT: u32 = 0;
const ALTERNATE_CODE_DIRECTORY_SLOT: u32 = 0x1000;
const REQUIREMENTS_SLOT: u32 = 2;
const ENTITLEMENTS_SLOT: u32 = 5;
const DER_ENTITLEMENTS_SLOT: u32 = 7;
const SIGNATURE_SLOT: u32 = 0x1_0000;
/// The designated requirement's slot among the requirements.
const DESIGNATED: u32 = 3;

/// Version 0x20400, which records the executable segment, and its header.
const VERSION: u32 = 0x2_0400;
const CODE_DIRECTORY_HEADER: usize = 88;
const ADHOC: u32 = 0x2;
/// The executable segment's flags: it's the app's main executable, and
/// may be debugged, which `get-task-allow` allows.
const MAIN_BINARY: u64 = 0x1;
const ALLOW_UNSIGNED: u64 = 0x10;

/// The requirement language's operations, as `csreq` compiles them.
const OP_IDENT: u32 = 2;
const OP_ANCHOR_HASH: u32 = 4;
const OP_AND: u32 = 6;
const OP_CERT_FIELD: u32 = 11;
const OP_CERT_GENERIC: u32 = 14;
const OP_APPLE_GENERIC_ANCHOR: u32 = 15;
const MATCH_EXISTS: u32 = 0;
const MATCH_EQUAL: u32 = 1;
/// A requirement's expression kind, and its certificate positions.
const EXPRESSION: u32 = 1;
const LEAF: u32 = 0;
const ISSUER: u32 = 1;
const ROOT: u32 = u32::MAX;
/// The extension (1.2.840.113635.100.6.2.1) on the certificate of Apple's
/// Worldwide Developer Relations authority, which issues app signing
/// certificates, as DER.
const WWDR_EXTENSION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x06, 0x02, 0x01];
/// The signed attributes in which Apple's CMS signature lists the cdhash:
/// as a property list, truncated to 20 bytes, and as a digest.
const CD_HASHES: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113635.100.9.1");
const CD_HASHES_2: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113635.100.9.2");
/// The organization of the authorities that issue Apple's certificates.
const APPLE: &str = "Apple Inc.";
/// The certificates above an Apple signing certificate: Apple's Worldwide
/// Developer Relations authority (G3), which has issued Apple Development
/// and Apple Distribution certificates since 2021, and Apple's root, which
/// issued it. codesign's signature carries both, but Keychain Access
/// exports a signing identity without them, so OmniBuild brings its own,
/// from apple.com/certificateauthority, verified against the root in
/// macOS's trust store.
const WWDR_G3: &[u8] = include_bytes!("apple/AppleWWDRCAG3.cer");
const APPLE_ROOT: &[u8] = include_bytes!("apple/AppleRootCA.cer");

/// A hash a code directory is made of, with its hash type's number in one.
#[derive(Clone, Copy)]
enum Hash {
    Sha1 = 1,
    Sha256 = 2,
}

impl Hash {
    fn of(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha1 => Sha1::digest(data).to_vec(),
            Self::Sha256 => Sha256::digest(data).to_vec(),
        }
    }

    fn size(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }

    fn oid(self) -> ObjectIdentifier {
        match self {
            Self::Sha1 => ID_SHA_1,
            Self::Sha256 => ID_SHA_256,
        }
    }
}

/// What goes into an app's signature besides its code.
#[derive(Clone, Copy)]
pub struct Signer<'a> {
    /// The bundle ID, which codesign uses as the identifier.
    pub identifier: &'a str,
    pub info_plist: &'a [u8],
    pub code_resources: &'a [u8],
    pub entitlements: Option<&'a Entitlements>,
    /// Signs ad hoc without one.
    pub key: Option<&'a SigningKey>,
    /// Whether it's the app's own executable, rather than a framework's.
    pub main: bool,
}

impl Signer<'_> {
    /// The signature of `code`.
    pub fn sign(&self, code: &Code) -> Vec<u8> {
        let requirements = self.requirements();
        let entitlements = self.entitlements.map(|entitlements| {
            (
                blob(ENTITLEMENTS, &entitlements.xml()),
                blob(DER_ENTITLEMENTS, &entitlements.der()),
            )
        });
        // What each special slot hashes, from slot -1 down. Slots -4 and -6
        // are for what iOS apps don't have, and are left empty.
        let mut special = vec![
            Some(self.info_plist),
            Some(&requirements[..]),
            Some(self.code_resources),
        ];
        if let Some((xml, der)) = &entitlements {
            special.extend([None, Some(&xml[..]), None, Some(&der[..])]);
        }
        // SHA-1's first, where it's there, as codesign orders them.
        let hashes: &[Hash] = if code.sha1 {
            &[Hash::Sha1, Hash::Sha256]
        } else {
            &[Hash::Sha256]
        };
        let directories: Vec<(Hash, Vec<u8>)> = hashes
            .iter()
            .map(|&hash| (hash, self.code_directory(code, hash, &special)))
            .collect();
        let signature = blob(BLOB_WRAPPER, &self.cms(&directories));
        let mut directories = directories.into_iter().map(|(_, directory)| directory);
        let mut blobs = vec![
            (
                CODE_DIRECTORY_SLOT,
                directories.next().expect("there's one"),
            ),
            (REQUIREMENTS_SLOT, requirements),
        ];
        if let Some((xml, der)) = entitlements {
            blobs.extend([(ENTITLEMENTS_SLOT, xml), (DER_ENTITLEMENTS_SLOT, der)]);
        }
        blobs.extend((ALTERNATE_CODE_DIRECTORY_SLOT..).zip(directories));
        blobs.push((SIGNATURE_SLOT, signature));
        super_blob(EMBEDDED_SIGNATURE, &blobs)
    }

    /// The code directory of `hash` hashes of the code's pages and of what
    /// each special slot has, `special`: nothing, for an empty one.
    fn code_directory(&self, code: &Code, hash: Hash, special: &[Option<&[u8]>]) -> Vec<u8> {
        let mut strings = [self.identifier.as_bytes(), &[0]].concat();
        let team_offset = match self.key.and_then(|key| team(key.x509())) {
            Some(team) => {
                let offset = CODE_DIRECTORY_HEADER + strings.len();
                strings.extend(team.as_bytes());
                strings.push(0);
                offset
            }
            None => 0,
        };
        let pages: Vec<Vec<u8>> = code
            .bytes
            .chunks(1 << code.page_size)
            .map(|page| hash.of(page))
            .collect();
        let special: Vec<Vec<u8>> = special
            .iter()
            .map(|slot| slot.map_or_else(|| vec![0; hash.size()], |data| hash.of(data)))
            .collect();
        let hash_offset = CODE_DIRECTORY_HEADER + strings.len() + special.len() * hash.size();
        let length = hash_offset + pages.len() * hash.size();
        let debuggable = self
            .entitlements
            .and_then(|entitlements| entitlements.get("get-task-allow"))
            == Some(&Entitlement::Boolean(true));

        let mut directory = Vec::with_capacity(length);
        for field in [
            CODE_DIRECTORY,
            u32_of(length),
            VERSION,
            if self.key.is_some() { 0 } else { ADHOC },
            u32_of(hash_offset),
            u32_of(CODE_DIRECTORY_HEADER),
            u32_of(special.len()),
            u32_of(pages.len()),
            u32_of(code.bytes.len()),
        ] {
            directory.extend(field.to_be_bytes());
        }
        // The hash size and type, the platform (0, not Apple's own), and
        // the page size.
        directory.extend([
            u8::try_from(hash.size()).expect("a hash's size fits"),
            hash as u8,
            0,
            code.page_size,
        ]);
        // A spare field, no scatter vector, the team, and another spare.
        for field in [0, 0, u32_of(team_offset), 0] {
            directory.extend(field.to_be_bytes());
        }
        // A 64-bit code limit, unused under 4 GiB, then the executable
        // segment.
        let flags = match (self.main, debuggable) {
            (false, _) => 0,
            (true, false) => MAIN_BINARY,
            (true, true) => MAIN_BINARY | ALLOW_UNSIGNED,
        };
        for field in [0, code.text.0, code.text.1, flags] {
            directory.extend(field.to_be_bytes());
        }
        directory.extend(strings);
        directory.extend(special.iter().rev().flatten());
        directory.extend(pages.iter().flatten());
        directory
    }

    /// The designated requirement, as codesign writes it. For a
    /// certificate Apple issued: the identifier, Apple's anchor, the
    /// certificate's name, and Apple's developer authority above it. For
    /// any other: the identifier, and the hash of its root certificate. An
    /// ad hoc signature has none.
    fn requirements(&self) -> Vec<u8> {
        let designated = self.key.map(|key| {
            let certificate = key.x509();
            let mut expression = Vec::new();
            op(&mut expression, EXPRESSION);
            if issued_by_apple(certificate) {
                for code in [OP_AND, OP_AND, OP_AND, OP_IDENT] {
                    op(&mut expression, code);
                }
                data(&mut expression, self.identifier.as_bytes());
                op(&mut expression, OP_APPLE_GENERIC_ANCHOR);
                op(&mut expression, OP_CERT_FIELD);
                op(&mut expression, LEAF);
                data(&mut expression, b"subject.CN");
                op(&mut expression, MATCH_EQUAL);
                let name = attribute(&certificate.tbs_certificate.subject, CN);
                data(&mut expression, name.unwrap_or_default().as_bytes());
                op(&mut expression, OP_CERT_GENERIC);
                op(&mut expression, ISSUER);
                data(&mut expression, WWDR_EXTENSION);
                op(&mut expression, MATCH_EXISTS);
            } else {
                op(&mut expression, OP_AND);
                op(&mut expression, OP_IDENT);
                data(&mut expression, self.identifier.as_bytes());
                op(&mut expression, OP_ANCHOR_HASH);
                op(&mut expression, ROOT);
                let root = key.chain().last().unwrap_or(certificate);
                data(&mut expression, &Sha1::digest(der(root)));
            }
            (DESIGNATED, blob(REQUIREMENT, &expression))
        });
        super_blob(REQUIREMENTS, designated.as_slice())
    }

    /// The CMS signature of the first of the code directories, each with
    /// the hash it's of, as codesign makes it: with the certificates, and
    /// among the signed attributes each one's cdhash, its own hash by the
    /// hash it's of, which iOS reads to know which were signed.
    fn cms(&self, code_directories: &[(Hash, Vec<u8>)]) -> Vec<u8> {
        let Some(key) = self.key else {
            return Vec::new();
        };
        let digest = Sha256::digest(&code_directories[0].1);
        let cd_hashes: Vec<(Hash, Vec<u8>)> = code_directories
            .iter()
            .map(|(hash, directory)| (*hash, hash.of(directory)))
            .collect();
        // As a property list, each truncated to 20 bytes.
        let mut truncated = Vec::new();
        Value::Dictionary(
            [(
                "cdhashes".to_owned(),
                Value::Array(
                    cd_hashes
                        .iter()
                        .map(|(_, cd_hash)| Value::Data(cd_hash[..20].to_vec()))
                        .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        )
        .to_writer_xml(&mut truncated)
        .expect("writing into memory can't fail");
        truncated.push(b'\n');
        // And whole, each with its hash.
        let whole = cd_hashes.iter().map(|(hash, cd_hash)| {
            Any::new(
                Tag::Sequence,
                [
                    der(&hash.oid()),
                    der(&OctetString::new(cd_hash.clone()).expect("a digest fits")),
                ]
                .concat(),
            )
        });
        let time = UtcTime::from_system_time(SystemTime::now()).expect("it's before 2050");
        let signed_attributes = SetOfVec::try_from(vec![
            signed_attribute(ID_CONTENT_TYPE, [Any::encode_from(&ID_DATA)]),
            signed_attribute(ID_SIGNING_TIME, [Any::encode_from(&time)]),
            signed_attribute(ID_MESSAGE_DIGEST, [octets(&digest)]),
            signed_attribute(CD_HASHES_2, whole),
            signed_attribute(CD_HASHES, [octets(&truncated)]),
        ])
        .expect("the attributes are distinct");

        let certificate = key.x509();
        let sha256 = AlgorithmIdentifierOwned {
            oid: ID_SHA_256,
            parameters: None,
        };
        let signer = SignerInfo {
            version: CmsVersion::V1,
            sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: certificate.tbs_certificate.issuer.clone(),
                serial_number: certificate.tbs_certificate.serial_number.clone(),
            }),
            digest_alg: sha256.clone(),
            signature: OctetString::new(key.sign(&der(&signed_attributes)))
                .expect("a signature fits an octet string"),
            signed_attrs: Some(signed_attributes),
            signature_algorithm: AlgorithmIdentifierOwned {
                oid: SHA_256_WITH_RSA_ENCRYPTION,
                parameters: Some(Any::null()),
            },
            unsigned_attrs: None,
        };
        // A certificate listed twice, in the key's file and among Apple's,
        // is carried once.
        let mut certificates = SetOfVec::new();
        for certificate in std::iter::once(certificate)
            .chain(key.chain())
            .chain(&apple_chain(certificate))
        {
            certificates
                .insert(CertificateChoices::Certificate(certificate.clone()))
                .ok();
        }
        let signed_data = SignedData {
            version: CmsVersion::V1,
            digest_algorithms: SetOfVec::try_from(vec![sha256]).expect("one algorithm is a set"),
            encap_content_info: EncapsulatedContentInfo {
                econtent_type: ID_DATA,
                econtent: None,
            },
            certificates: Some(CertificateSet(certificates)),
            crls: None,
            signer_infos: SignerInfos(
                SetOfVec::try_from(vec![signer]).expect("one signer is a set"),
            ),
        };
        der(&ContentInfo {
            content_type: ID_SIGNED_DATA,
            content: Any::encode_from(&signed_data).expect("signed data encodes"),
        })
    }
}

/// Apple's certificates above `certificate`, if WWDR G3 issued it.
fn apple_chain(certificate: &Certificate) -> Vec<Certificate> {
    let wwdr = Certificate::from_der(WWDR_G3).expect("Apple's certificate parses");
    if certificate.tbs_certificate.issuer == wwdr.tbs_certificate.subject {
        vec![
            wwdr,
            Certificate::from_der(APPLE_ROOT).expect("Apple's certificate parses"),
        ]
    } else {
        Vec::new()
    }
}

/// Whether Apple issued the certificate: its issuer, Apple's Worldwide
/// Developer Relations authority, is Apple's organization.
fn issued_by_apple(certificate: &Certificate) -> bool {
    attribute(&certificate.tbs_certificate.issuer, O).as_deref() == Some(APPLE)
}

/// The team a certificate Apple issued is for, which is its
/// organizational unit. codesign records a team only for those.
fn team(certificate: &Certificate) -> Option<String> {
    issued_by_apple(certificate)
        .then(|| attribute(&certificate.tbs_certificate.subject, OU))
        .flatten()
}

/// The value of a name's attribute, such as its common name.
fn attribute(name: &Name, oid: ObjectIdentifier) -> Option<String> {
    name.0
        .iter()
        .flat_map(|names| names.0.iter())
        .find(|attribute| attribute.oid == oid)
        .and_then(|attribute| String::from_utf8(attribute.value.value().to_vec()).ok())
}

/// A signed attribute with `values`, which DER puts in order.
fn signed_attribute(
    oid: ObjectIdentifier,
    values: impl IntoIterator<Item = x509_cert::der::Result<Any>>,
) -> Attribute {
    let values: Vec<Any> = values
        .into_iter()
        .map(|value| value.expect("the attribute encodes"))
        .collect();
    Attribute {
        oid,
        values: SetOfVec::try_from(values).expect("the values are distinct"),
    }
}

fn octets(bytes: &[u8]) -> x509_cert::der::Result<Any> {
    OctetString::new(bytes.to_vec()).and_then(|octets| Any::encode_from(&octets))
}

fn der(value: &impl Encode) -> Vec<u8> {
    value.to_der().expect("it encodes")
}

/// A blob: its magic number, its length, and its contents.
fn blob(magic: u32, contents: &[u8]) -> Vec<u8> {
    [
        &magic.to_be_bytes()[..],
        &u32_of(8 + contents.len()).to_be_bytes(),
        contents,
    ]
    .concat()
}

/// A super blob: its magic number, its length and its number of blobs,
/// then each blob's slot and offset, then the blobs.
fn super_blob(magic: u32, blobs: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let header = 12 + 8 * blobs.len();
    let length = header + blobs.iter().map(|(_, blob)| blob.len()).sum::<usize>();
    let mut out = Vec::with_capacity(length);
    for field in [magic, u32_of(length), u32_of(blobs.len())] {
        out.extend(field.to_be_bytes());
    }
    let mut offset = header;
    for (slot, blob) in blobs {
        out.extend(slot.to_be_bytes());
        out.extend(u32_of(offset).to_be_bytes());
        offset += blob.len();
    }
    for (_, blob) in blobs {
        out.extend(blob);
    }
    out
}

/// An operation or match in a requirement's expression.
fn op(expression: &mut Vec<u8>, code: u32) {
    expression.extend(code.to_be_bytes());
}

/// Data in a requirement's expression: its length, then it, padded to 4
/// bytes.
fn data(expression: &mut Vec<u8>, bytes: &[u8]) {
    op(expression, u32_of(bytes.len()));
    expression.extend(bytes);
    expression.resize(expression.len().next_multiple_of(4), 0);
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("signatures are under 4 GiB")
}

#[cfg(test)]
pub mod tests {
    use std::path::Path;

    use cms::content_info::ContentInfo;
    use cms::signed_data::SignedData;
    use sha1::Sha1;
    use sha2::{Digest, Sha256};
    use x509_cert::der::asn1::OctetString;
    use x509_cert::der::{Decode, Encode};

    use x509_cert::Certificate;

    use super::{Signer, WWDR_G3};
    use crate::bytes::Reader;
    use crate::ios::entitlements::tests::shell_entitlements;
    use crate::ios::macho::Code;
    use crate::key::SigningKey;
    use crate::key::tests::{issued_key, verifies};

    /// The test identity: a key with a self-signed certificate,
    /// "OmniBuild Test", made as tests/fixtures/README.md says.
    pub fn identity() -> SigningKey {
        SigningKey::read(
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/identity.pem"
            )),
            "",
        )
        .unwrap()
    }

    pub fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    /// Each blob in a super blob, by its slot.
    pub fn blobs(super_blob: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let mut header = Reader::at(super_blob, 8);
        (0..header.u32_be())
            .map(|_| {
                let (slot, offset) = (header.u32_be(), header.u32_be() as usize);
                let length = Reader::at(super_blob, offset + 4).u32_be() as usize;
                (slot, super_blob[offset..offset + length].to_vec())
            })
            .collect()
    }

    /// A code directory's fields, by their offsets.
    fn field(directory: &[u8], at: usize) -> u32 {
        Reader::at(directory, at).u32_be()
    }

    /// 40,000 bytes of code: three pages of 16 KiB, the last one short.
    fn code(bytes: &[u8]) -> Code<'_> {
        Code {
            bytes,
            page_size: 14,
            text: (0, 32_768),
            sha1: false,
        }
    }

    #[test]
    fn signs_ad_hoc_with_hashes_of_the_code_and_its_resources() {
        let bytes: Vec<u8> = (0..40_000u32)
            .map(|byte| u8::try_from(byte % 251).unwrap())
            .collect();
        let signer = Signer {
            identifier: "dev.omnibuild.shell",
            info_plist: b"Info.plist",
            code_resources: b"CodeResources",
            entitlements: None,
            key: None,
            main: true,
        };

        let signature = signer.sign(&code(&bytes));

        assert_eq!(&signature[..4], [0xfa, 0xde, 0x0c, 0xc0]);
        let blobs = blobs(&signature);
        let slots: Vec<u32> = blobs.iter().map(|(slot, _)| *slot).collect();
        assert_eq!(slots, [0, 2, 0x1_0000]);
        let directory = &blobs[0].1;
        // Ad hoc, 3 special slots, 3 pages, the whole code, 16 KiB pages.
        assert_eq!(field(directory, 12), 0x2);
        assert_eq!(field(directory, 24), 3);
        assert_eq!(field(directory, 28), 3);
        assert_eq!(field(directory, 32), 40_000);
        assert_eq!(directory[36..40], [32, 2, 0, 14]);
        // No team, and the main executable's segment, not debuggable.
        assert_eq!(field(directory, 48), 0);
        assert_eq!(
            directory[64..88],
            [
                &[0; 8][..],
                &[0, 0, 0, 0, 0, 0, 0x80, 0],
                &[0, 0, 0, 0, 0, 0, 0, 1]
            ]
            .concat()
        );
        assert_eq!(&directory[88..108], b"dev.omnibuild.shell\0");
        let hashes: Vec<&[u8]> = directory[108..].chunks(32).collect();
        assert_eq!(hashes[0], Sha256::digest(b"CodeResources").as_slice());
        assert_eq!(hashes[1], Sha256::digest(&blobs[1].1).as_slice());
        assert_eq!(hashes[2], Sha256::digest(b"Info.plist").as_slice());
        assert_eq!(hashes[3], Sha256::digest(&bytes[..16_384]).as_slice());
        assert_eq!(hashes[5], Sha256::digest(&bytes[32_768..]).as_slice());
        // No requirements, and an empty CMS blob, as codesign signs ad hoc.
        assert_eq!(
            blobs[1].1,
            [0xfa, 0xde, 0x0c, 0x01, 0, 0, 0, 12, 0, 0, 0, 0]
        );
        assert_eq!(blobs[2].1, [0xfa, 0xde, 0x0b, 0x01, 0, 0, 0, 8]);
    }

    #[test]
    fn signs_with_a_key_its_entitlements_and_a_cms_signature() {
        let key = identity();
        let entitlements = shell_entitlements();
        let bytes = vec![7; 100];
        let signer = Signer {
            identifier: "dev.omnibuild.shell",
            info_plist: b"Info.plist",
            code_resources: b"CodeResources",
            entitlements: Some(&entitlements),
            key: Some(&key),
            main: true,
        };

        let signature = signer.sign(&code(&bytes));

        let blobs = blobs(&signature);
        let slots: Vec<u32> = blobs.iter().map(|(slot, _)| *slot).collect();
        assert_eq!(slots, [0, 2, 5, 7, 0x1_0000]);
        let directory = &blobs[0].1;
        // Not ad hoc, 7 special slots, no team (the certificate isn't
        // Apple's), and debuggable, as get-task-allow says.
        assert_eq!(field(directory, 12), 0);
        assert_eq!(field(directory, 24), 7);
        assert_eq!(field(directory, 48), 0);
        assert_eq!(directory[87], 0x11);
        let hashes: Vec<&[u8]> = directory[108..].chunks(32).collect();
        assert_eq!(hashes[0], Sha256::digest(&blobs[4 - 1].1).as_slice());
        assert_eq!(hashes[1], [0; 32]);
        assert_eq!(hashes[2], Sha256::digest(&blobs[2].1).as_slice());
        assert_eq!(&blobs[2].1[8..], entitlements.xml());
        assert_eq!(&blobs[3].1[8..], entitlements.der());
        // codesign's designated requirement for the same certificate.
        assert_eq!(
            blobs[1].1,
            hex(concat!(
                "fade0c0100000060000000010000000300000014fade0c000000004c00000001",
                "0000000600000002000000136465762e6f6d6e696275696c642e7368656c6c00",
                "00000004ffffffff0000001439d7175faefa14230862c6160c685cedab766248"
            ))
        );

        let cms = ContentInfo::from_der(&blobs[4].1[8..]).unwrap();
        let signed_data = cms.content.decode_as::<SignedData>().unwrap();
        let signer = signed_data.signer_infos.0.get(0).unwrap();
        let attributes = signer.signed_attrs.as_ref().unwrap();
        let digest = Sha256::digest(directory);
        let values: Vec<Vec<u8>> = attributes
            .iter()
            .map(|attribute| attribute.values.get(0).unwrap().value().to_vec())
            .collect();
        assert!(values.contains(&digest.to_vec()), "messageDigest");
        let plist = values
            .iter()
            .find(|value| value.starts_with(b"<?xml"))
            .unwrap();
        let cdhashes = plist::Value::from_reader(std::io::Cursor::new(plist))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(
            cdhashes["cdhashes"].as_array().unwrap()[0]
                .as_data()
                .unwrap(),
            &digest[..20]
        );
        assert!(verifies(
            key.public_key(),
            &attributes.to_der().unwrap(),
            signer.signature.as_bytes()
        ));
        assert_eq!(signed_data.certificates.unwrap().0.len(), 1);
    }

    #[test]
    fn signs_code_for_older_systems_with_a_sha1_code_directory_too() {
        let key = identity();
        let entitlements = shell_entitlements();
        let bytes = vec![7; 100];
        let signer = Signer {
            identifier: "dev.omnibuild.shell.watchkitapp",
            info_plist: b"Info.plist",
            code_resources: b"CodeResources",
            entitlements: Some(&entitlements),
            key: Some(&key),
            main: true,
        };

        let signature = signer.sign(&Code {
            sha1: true,
            ..code(&bytes)
        });

        // SHA-1's first, then SHA-256's after the entitlements, as codesign
        // orders them.
        let blobs = blobs(&signature);
        let slots: Vec<u32> = blobs.iter().map(|(slot, _)| *slot).collect();
        assert_eq!(slots, [0, 2, 5, 7, 0x1000, 0x1_0000]);
        let (sha1, sha256) = (&blobs[0].1, &blobs[4].1);
        // Each of its own hashes, of the same code and special slots: the
        // 20-byte SHA-1 (type 1) and the 32-byte SHA-256 (type 2).
        assert_eq!((sha1[36], sha1[37]), (20, 1));
        assert_eq!((sha256[36], sha256[37]), (32, 2));
        let identifier = 88 + "dev.omnibuild.shell.watchkitapp".len() + 1;
        assert_eq!(field(sha1, 16) as usize, identifier + 7 * 20);
        assert_eq!(field(sha256, 16) as usize, identifier + 7 * 32);
        assert_eq!(
            sha1[identifier..identifier + 20],
            *Sha1::digest(&blobs[3].1).as_slice()
        );
        assert_eq!(sha1[identifier + 20..identifier + 40], [0; 20]);
        assert_eq!(
            sha1[identifier + 7 * 20..],
            *Sha1::digest(&bytes).as_slice()
        );
        // The rest of their headers, and their identifiers, the same: but
        // for their lengths, where their hashes start, and their kinds.
        for range in [8..16, 20..36, 38..identifier] {
            assert_eq!(sha1[range.clone()], sha256[range]);
        }

        // The CMS signs the first, and names both by their own hashes.
        let cms = ContentInfo::from_der(&blobs[5].1[8..]).unwrap();
        let signed_data = cms.content.decode_as::<SignedData>().unwrap();
        let signer = signed_data.signer_infos.0.get(0).unwrap();
        let attributes = signer.signed_attrs.as_ref().unwrap();
        let values = |oid: &str| -> Vec<Vec<u8>> {
            attributes
                .iter()
                .find(|attribute| attribute.oid.to_string() == oid)
                .unwrap()
                .values
                .iter()
                .map(|value| value.to_der().unwrap())
                .collect()
        };
        let octets = |bytes: &[u8]| OctetString::new(bytes).unwrap().to_der().unwrap();
        assert_eq!(
            values("1.2.840.113549.1.9.4"),
            [octets(&Sha256::digest(sha1))]
        );
        // Each with its hash's OID, SHA-1's (1.3.14.3.2.26) first, as DER
        // orders a set.
        let sequence = |oid: &[u8], digest: &[u8]| {
            [
                &[0x30, u8::try_from(oid.len() + 2 + digest.len()).unwrap()][..],
                oid,
                &octets(digest),
            ]
            .concat()
        };
        assert_eq!(
            values("1.2.840.113635.100.9.2"),
            [
                sequence(&hex("06052b0e03021a"), &Sha1::digest(sha1)),
                sequence(&hex("0609608648016503040201"), &Sha256::digest(sha256)),
            ]
        );
        let plist = &values("1.2.840.113635.100.9.1")[0];
        let cdhashes = plist::Value::from_reader(std::io::Cursor::new(&plist[4..]))
            .unwrap()
            .into_dictionary()
            .unwrap();
        let cdhashes: Vec<&[u8]> = cdhashes["cdhashes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|cdhash| cdhash.as_data().unwrap())
            .collect();
        assert_eq!(
            cdhashes,
            [&Sha1::digest(sha1)[..], &Sha256::digest(sha256)[..20]]
        );
        assert!(verifies(
            key.public_key(),
            &attributes.to_der().unwrap(),
            signer.signature.as_bytes()
        ));
    }

    #[test]
    fn writes_apples_requirement_team_and_chain_for_a_certificate_apple_issued() {
        // Issued by Apple's WWDR G3, by its name in the certificate
        // OmniBuild carries.
        let wwdr = Certificate::from_der(WWDR_G3).unwrap();
        let key = issued_key(
            wwdr.tbs_certificate.subject.clone(),
            "CN=Apple Development: Jane Doe (ABCDE12345),OU=ABCDE12345,O=Jane Doe,C=US",
        );
        let signer = Signer {
            identifier: "dev.omnibuild.shell",
            info_plist: b"",
            code_resources: b"",
            entitlements: None,
            key: Some(&key),
            main: true,
        };

        let signature = signer.sign(&code(&[]));

        let blobs = blobs(&signature);
        let directory = &blobs[0].1;
        assert_eq!(field(directory, 48), 108);
        assert_eq!(&directory[88..119], b"dev.omnibuild.shell\0ABCDE12345\0");
        // What csreq compiles the same requirement to.
        assert_eq!(
            blobs[1].1,
            hex(concat!(
                "fade0c01000000b0000000010000000300000014fade0c000000009c00000001",
                "00000006000000060000000600000002000000136465762e6f6d6e696275696c",
                "642e7368656c6c000000000f0000000b000000000000000a7375626a6563742e",
                "434e000000000001000000284170706c6520446576656c6f706d656e743a204a",
                "616e6520446f65202841424344453132333435290000000e000000010000000a",
                "2a864886f76364060201000000000000"
            ))
        );
        // The CMS signature carries Apple's chain: the key's certificate,
        // WWDR G3 and Apple's root.
        let cms = ContentInfo::from_der(&blobs[2].1[8..]).unwrap();
        let signed_data = cms.content.decode_as::<SignedData>().unwrap();
        let subjects: Vec<String> = signed_data
            .certificates
            .unwrap()
            .0
            .iter()
            .map(|choice| {
                let certificate = Certificate::from_der(&choice.to_der().unwrap()).unwrap();
                certificate.tbs_certificate.subject.to_string()
            })
            .collect();
        assert_eq!(subjects.len(), 3);
        assert!(
            subjects
                .iter()
                .any(|subject| subject.contains("CN=Apple Root CA"))
        );
        assert!(subjects.iter().any(|subject| subject.contains("OU=G3")));
    }
}
