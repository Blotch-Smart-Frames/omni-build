//! JAR signing, as `jarsigner` does it, which is what an Android App Bundle
//! is signed with: Google Play checks an AAB's upload key this way, then
//! signs the APKs it makes from the bundle with the app's own key.
//!
//! A JAR signature is three files in `META-INF/`: a manifest with a digest
//! of each entry, a signature file with a digest of each of the manifest's
//! sections, and a PKCS #7 signature of the signature file.

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use const_oid::db::rfc5911::{ID_DATA, ID_SIGNED_DATA};
use const_oid::db::rfc5912::{ID_SHA_256, RSA_ENCRYPTION};
use sha2::{Digest, Sha256};
use x509_cert::der::asn1::{OctetString, SetOfVec};
use x509_cert::der::{Any, Encode};
use x509_cert::spki::AlgorithmIdentifierOwned;

use crate::key::SigningKey;
use crate::zip::{Entry, Method};

/// Lines in a JAR manifest are at most 72 bytes.
const LINE: usize = 72;

/// The entries with a JAR signature in front of them, signed by `key`.
pub fn sign(entries: Vec<Entry>, key: &SigningKey) -> Result<Vec<Entry>> {
    let main_attributes = [
        header("Manifest-Version", "1.0"),
        header("Created-By", "OmniBuild"),
        "\r\n".to_owned(),
    ]
    .concat();
    let mut manifest = main_attributes.clone();
    let mut sections = String::new();
    for entry in &entries {
        let section = [
            header("Name", &entry.name),
            header("SHA-256-Digest", &digest(&entry.contents()?)),
            "\r\n".to_owned(),
        ]
        .concat();
        sections.push_str(&header("Name", &entry.name));
        sections.push_str(&header("SHA-256-Digest", &digest(section.as_bytes())));
        sections.push_str("\r\n");
        manifest.push_str(&section);
    }
    let signature_file = [
        header("Signature-Version", "1.0"),
        header("Created-By", "OmniBuild"),
        header("SHA-256-Digest-Manifest", &digest(manifest.as_bytes())),
        header(
            "SHA-256-Digest-Manifest-Main-Attributes",
            &digest(main_attributes.as_bytes()),
        ),
        "\r\n".to_owned(),
        sections,
    ]
    .concat();
    let block = signature_block(key, signature_file.as_bytes());
    // The manifest first, as java.util.jar.JarInputStream expects.
    Ok([
        Entry::new(
            "META-INF/MANIFEST.MF",
            Method::Deflated,
            manifest.as_bytes(),
        ),
        Entry::new(
            "META-INF/CERT.SF",
            Method::Deflated,
            signature_file.as_bytes(),
        ),
        Entry::new("META-INF/CERT.RSA", Method::Deflated, &block),
    ]
    .into_iter()
    .chain(entries)
    .collect())
}

/// A header line, wrapped as the JAR format requires: at most 72 bytes a
/// line, each line after the first starting with a space.
fn header(name: &str, value: &str) -> String {
    let mut text = String::new();
    let mut width = 0;
    for character in format!("{name}: {value}").chars() {
        if width + character.len_utf8() > LINE {
            text.push_str("\r\n ");
            width = 1;
        }
        text.push(character);
        width += character.len_utf8();
    }
    text + "\r\n"
}

fn digest(data: &[u8]) -> String {
    STANDARD.encode(Sha256::digest(data))
}

/// The signature file's signature: PKCS #7 signed data with the
/// certificate, and an RSA signature of the file's SHA-256 digest. Without
/// signed attributes, as jarsigner long wrote it and every Android and Java
/// version reads it.
fn signature_block(key: &SigningKey, signature_file: &[u8]) -> Vec<u8> {
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
        signed_attrs: None,
        signature_algorithm: AlgorithmIdentifierOwned {
            oid: RSA_ENCRYPTION,
            parameters: Some(Any::null()),
        },
        signature: OctetString::new(key.sign(signature_file))
            .expect("a signature fits an octet string"),
        unsigned_attrs: None,
    };
    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![sha256]).expect("one algorithm is a set"),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: ID_DATA,
            econtent: None,
        },
        certificates: Some(CertificateSet(
            SetOfVec::try_from(vec![CertificateChoices::Certificate(certificate.clone())])
                .expect("one certificate is a set"),
        )),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer]).expect("one signer is a set")),
    };
    ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: Any::encode_from(&signed_data).expect("signed data encodes"),
    }
    .to_der()
    .expect("a content info encodes")
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use cms::cert::IssuerAndSerialNumber;
    use cms::content_info::ContentInfo;
    use cms::signed_data::{SignedData, SignerIdentifier};
    use sha2::{Digest, Sha256};
    use x509_cert::der::Decode;

    use super::{header, sign};
    use crate::key::tests::{KEY, verifies};
    use crate::zip::{Entry, Method};

    fn text(entry: &Entry) -> String {
        String::from_utf8(entry.contents().unwrap()).unwrap()
    }

    #[test]
    fn signs_every_entry() {
        let entries = vec![
            Entry::new(
                "base/manifest/AndroidManifest.xml",
                Method::Deflated,
                b"manifest",
            ),
            Entry::new("base/assets/payload.txt", Method::Stored, b"payload"),
        ];

        let jar = sign(entries.clone(), &KEY).unwrap();

        let names: Vec<&str> = jar.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "META-INF/MANIFEST.MF",
                "META-INF/CERT.SF",
                "META-INF/CERT.RSA",
                "base/manifest/AndroidManifest.xml",
                "base/assets/payload.txt",
            ]
        );
        assert_eq!(jar[3..], entries);
        let manifest = text(&jar[0]);
        let payload_digest = STANDARD.encode(Sha256::digest(b"payload"));
        let payload_section =
            format!("Name: base/assets/payload.txt\r\nSHA-256-Digest: {payload_digest}\r\n\r\n");
        assert!(manifest.starts_with("Manifest-Version: 1.0\r\nCreated-By: OmniBuild\r\n\r\n"));
        assert!(manifest.ends_with(&payload_section));

        // The signature file has a digest of the manifest, and of its
        // payload section.
        let signature_file = text(&jar[1]);
        let manifest_digest = STANDARD.encode(Sha256::digest(manifest.as_bytes()));
        let section_digest = STANDARD.encode(Sha256::digest(payload_section.as_bytes()));
        assert!(
            signature_file.contains(&format!("SHA-256-Digest-Manifest: {manifest_digest}\r\n"))
        );
        assert!(signature_file.ends_with(&format!(
            "Name: base/assets/payload.txt\r\nSHA-256-Digest: {section_digest}\r\n\r\n"
        )));

        // The block signs the signature file with the key, and names its
        // certificate.
        let block = ContentInfo::from_der(&jar[2].contents().unwrap()).unwrap();
        let data = block.content.decode_as::<SignedData>().unwrap();
        let signer = data.signer_infos.0.get(0).unwrap();
        let issuer = IssuerAndSerialNumber {
            issuer: KEY.x509().tbs_certificate.issuer.clone(),
            serial_number: KEY.x509().tbs_certificate.serial_number.clone(),
        };
        assert_eq!(signer.sid, SignerIdentifier::IssuerAndSerialNumber(issuer));
        assert!(verifies(
            KEY.public_key(),
            signature_file.as_bytes(),
            signer.signature.as_bytes()
        ));
    }

    #[test]
    fn wraps_long_header_lines() {
        let name = format!("base/assets/{}", "é".repeat(40));

        let line = header("Name", &name);

        let lines: Vec<&str> = line.trim_end().split("\r\n").collect();
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= 72));
        assert!(lines[1..].iter().all(|line| line.starts_with(' ')));
        let joined: String = lines
            .iter()
            .enumerate()
            .map(|(index, line)| if index == 0 { *line } else { &line[1..] })
            .collect();
        assert_eq!(joined, format!("Name: {name}"));
    }

    #[test]
    fn reports_an_entry_it_cant_read() {
        let mut corrupt = Entry::new("base/dex/classes.dex", Method::Deflated, b"dex");
        corrupt.data = vec![0xff; 8];

        assert_eq!(
            sign(vec![corrupt], &KEY).unwrap_err().to_string(),
            "base/dex/classes.dex is corrupt"
        );
    }
}
