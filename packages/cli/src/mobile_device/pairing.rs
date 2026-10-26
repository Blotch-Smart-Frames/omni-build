//! Pair records, which a computer proves itself to an iPhone or iPad with
//! once the device's user has trusted it. Pairing makes three certificates,
//! as libimobiledevice and Apple's tools make them, with empty names: a
//! root, which the device trusts from then on, one the root issues for this
//! computer's key, which the computer shows in each session, and one it
//! issues for the device's own key, which the device keeps. The usbmux
//! daemon keeps the record, as a property list, where the system's own
//! tools find it too.

use std::fmt::Write as _;
use std::io::Cursor;
use std::time::Duration;

use anyhow::{Context, Result};
use plist::{Dictionary, Value};
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::pkcs1v15;
use rsa::pkcs8::{DecodePublicKey, EncodePrivateKey, LineEnding};
use rsa::rand_core::{OsRng, RngCore};
use rsa::sha2::Sha256;
use rsa::{RsaPrivateKey, RsaPublicKey};
use x509_cert::Certificate;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::der::EncodePem;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfoOwned;
use x509_cert::time::Validity;

use crate::key;

/// How long the certificates last: ten years, as libimobiledevice's.
const VALIDITY: Duration = Duration::from_hours(10 * 365 * 24);
/// The keys of a pair record that stay on this computer, which a pairing
/// request leaves out.
const PRIVATE_KEYS: [&str; 2] = ["HostPrivateKey", "RootPrivateKey"];

/// A pair record, as the usbmux daemon keeps one.
#[derive(Clone, Debug, PartialEq)]
pub struct PairRecord {
    /// This computer's ID for the pairing, which a session names.
    pub host_id: String,
    /// The usbmux daemon's ID for this computer.
    pub system_buid: String,
    /// The certificate this computer shows in a session, and its key, in
    /// PEM.
    pub host_certificate: Vec<u8>,
    pub host_private_key: Vec<u8>,
    /// All of it, the keys OmniBuild doesn't read included.
    record: Dictionary,
}

impl PairRecord {
    /// The record in `bytes`, a property list.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let record = Value::from_reader(Cursor::new(bytes))
            .ok()
            .and_then(Value::into_dictionary)
            .context("the pair record isn't a property list")?;
        let string = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_string)
                .map(str::to_owned)
                .with_context(|| format!("the pair record has no {key}"))
        };
        let data = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_data)
                .map(<[u8]>::to_vec)
                .with_context(|| format!("the pair record has no {key}"))
        };
        Ok(Self {
            host_id: string("HostID")?,
            system_buid: string("SystemBUID")?,
            host_certificate: data("HostCertificate")?,
            host_private_key: data("HostPrivateKey")?,
            record,
        })
    }

    /// A new record for pairing with the device whose public key is
    /// `device_key`, an RSA key in PEM, as lockdownd gives it
    /// (`DevicePublicKey`), for this computer, which the usbmux daemon
    /// knows as `system_buid`.
    pub fn generate(device_key: &[u8], system_buid: &str) -> Result<Self> {
        let pem = std::str::from_utf8(device_key).unwrap_or_default();
        // PKCS #1, as lockdownd gives it, or else SubjectPublicKeyInfo.
        let device_key = RsaPublicKey::from_pkcs1_pem(pem)
            .or_else(|_| RsaPublicKey::from_public_key_pem(pem))
            .ok()
            .context("the device's public key isn't an RSA key in PEM")?;
        let root_key = key::new_key();
        let root = pkcs1v15::SigningKey::<Sha256>::new(root_key.clone());
        let root_certificate = certificate(Profile::Root, &root_key.to_public_key(), &root);
        let host_key = key::new_key();
        let host_certificate = certificate(leaf(), &host_key.to_public_key(), &root);
        let device_certificate = certificate(leaf(), &device_key, &root);
        let host_private_key = pkcs8_pem(&host_key);
        let host_id = host_id();
        let mut record = Dictionary::new();
        for (key, value) in [
            ("DeviceCertificate", Value::Data(device_certificate)),
            ("HostCertificate", Value::Data(host_certificate.clone())),
            ("HostPrivateKey", Value::Data(host_private_key.clone())),
            ("RootCertificate", Value::Data(root_certificate)),
            ("RootPrivateKey", Value::Data(pkcs8_pem(&root_key))),
            ("HostID", host_id.as_str().into()),
            ("SystemBUID", system_buid.into()),
        ] {
            record.insert(key.to_owned(), value);
        }
        Ok(Self {
            host_id,
            system_buid: system_buid.to_owned(),
            host_certificate,
            host_private_key,
            record,
        })
    }

    /// What a pairing request sends the device: the record without its
    /// private keys, which stay on this computer.
    pub fn public(&self) -> Dictionary {
        let mut public = self.record.clone();
        for key in PRIVATE_KEYS {
            public.remove(key);
        }
        public
    }

    /// Adds `value` to the record as `key`: what the device gives when it
    /// pairs, or says of itself.
    pub fn insert(&mut self, key: &str, value: Value) {
        self.record.insert(key.to_owned(), value);
    }

    /// The record as a property list, as the usbmux daemon keeps it.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        Value::Dictionary(self.record.clone())
            .to_writer_xml(&mut bytes)
            .expect("a dictionary writes to memory");
        bytes
    }
}

/// The profile of the certificates the root issues: an end entity's, whose
/// key signs and encrypts keys in TLS, issued by the root, whose name is
/// empty too.
fn leaf() -> Profile {
    Profile::Leaf {
        issuer: Name::default(),
        enable_key_agreement: false,
        enable_key_encipherment: true,
    }
}

/// A certificate with an empty name, of `profile`, for the key `subject`,
/// signed by `root`, in PEM.
fn certificate(
    profile: Profile,
    subject: &RsaPublicKey,
    root: &pkcs1v15::SigningKey<Sha256>,
) -> Vec<u8> {
    let subject =
        SubjectPublicKeyInfoOwned::from_key(subject.clone()).expect("an RSA public key encodes");
    let certificate: Certificate = CertificateBuilder::new(
        profile,
        SerialNumber::from(OsRng.next_u32()),
        Validity::from_now(VALIDITY).expect("the validity fits a certificate"),
        Name::default(),
        subject,
        root,
    )
    .and_then(Builder::build::<pkcs1v15::Signature>)
    .expect("a certificate with valid fields builds");
    certificate
        .to_pem(LineEnding::LF)
        .expect("a certificate encodes")
        .into_bytes()
}

/// `key` in PEM, as PKCS #8.
fn pkcs8_pem(key: &RsaPrivateKey) -> Vec<u8> {
    key.to_pkcs8_pem(LineEnding::LF)
        .expect("an RSA key encodes")
        .as_bytes()
        .to_vec()
}

/// A new ID for a pairing: a random UUID, in capitals, as
/// libimobiledevice makes one.
fn host_id() -> String {
    let mut bytes = [0; 16];
    OsRng.fill_bytes(&mut bytes);
    // Version 4, random, of the variant RFC 4122 describes.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02X}");
        hex
    });
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
pub mod tests {
    use plist::Value;
    use rsa::pkcs1::EncodeRsaPublicKey;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use x509_cert::Certificate;
    use x509_cert::der::DecodePem;
    use x509_cert::name::Name;

    use super::{PairRecord, host_id};
    use crate::adb::tls::tests::DEVICE_KEY;

    /// A device's key: the made-up Android device's, as good as any.
    pub fn device_key() -> RsaPrivateKey {
        RsaPrivateKey::from_pkcs8_der(&DEVICE_KEY.private_key()).unwrap()
    }

    /// The device's public key, as lockdownd gives it: PKCS #1, in PEM.
    pub fn device_public_key() -> Vec<u8> {
        RsaPublicKey::from(device_key())
            .to_pkcs1_pem(LineEnding::LF)
            .unwrap()
            .into_bytes()
    }

    fn certificate(record: &PairRecord, key: &str) -> Certificate {
        Certificate::from_pem(record.public()[key].as_data().unwrap()).unwrap()
    }

    #[test]
    fn makes_a_record_whose_root_issues_this_computers_and_the_devices_certificates() {
        let record = PairRecord::generate(&device_public_key(), "BUID-1").unwrap();

        let root = certificate(&record, "RootCertificate");
        let device = certificate(&record, "DeviceCertificate");
        let host = certificate(&record, "HostCertificate");
        for issued in [&device, &host] {
            assert_eq!(issued.tbs_certificate.issuer, root.tbs_certificate.subject);
            assert_eq!(issued.tbs_certificate.subject, Name::default());
        }
        assert_eq!(
            device.tbs_certificate.subject_public_key_info,
            x509_cert::spki::SubjectPublicKeyInfoOwned::from_key(RsaPublicKey::from(device_key()))
                .unwrap()
        );
        assert_eq!(record.system_buid, "BUID-1");
        assert_eq!(record.public()["HostID"], record.host_id.as_str().into());
        // The private keys stay here.
        assert!(!record.public().contains_key("HostPrivateKey"));
        assert!(!record.public().contains_key("RootPrivateKey"));
        assert!(
            String::from_utf8(record.host_private_key.clone())
                .unwrap()
                .starts_with("-----BEGIN PRIVATE KEY-----")
        );
    }

    #[test]
    fn takes_the_devices_key_as_spki_too() {
        let spki = RsaPublicKey::from(device_key())
            .to_public_key_pem(LineEnding::LF)
            .unwrap();

        assert!(PairRecord::generate(spki.as_bytes(), "BUID-1").is_ok());
        assert_eq!(
            PairRecord::generate(b"\xff", "BUID-1")
                .unwrap_err()
                .to_string(),
            "the device's public key isn't an RSA key in PEM"
        );
    }

    #[test]
    fn keeps_a_record_as_the_daemon_does() {
        let mut record = PairRecord::generate(&device_public_key(), "BUID-1").unwrap();
        record.insert("EscrowBag", Value::Data(b"bag".to_vec()));

        let read = PairRecord::parse(&record.to_bytes()).unwrap();

        assert_eq!(read, record);
        assert_eq!(read.public()["EscrowBag"], Value::Data(b"bag".to_vec()));
    }

    #[test]
    fn says_what_a_record_lacks() {
        let record = PairRecord::generate(&device_public_key(), "BUID-1").unwrap();
        let without = |key: &str| {
            let mut bytes = Vec::new();
            let mut dictionary = record.public();
            dictionary.insert(
                "HostPrivateKey".to_owned(),
                Value::Data(record.host_private_key.clone()),
            );
            dictionary.remove(key);
            Value::Dictionary(dictionary)
                .to_writer_xml(&mut bytes)
                .unwrap();
            PairRecord::parse(&bytes).unwrap_err().to_string()
        };

        for key in ["HostID", "SystemBUID", "HostCertificate", "HostPrivateKey"] {
            assert_eq!(without(key), format!("the pair record has no {key}"));
        }
        assert_eq!(
            PairRecord::parse(b"record").unwrap_err().to_string(),
            "the pair record isn't a property list"
        );
    }

    #[test]
    fn makes_a_random_uuid_for_each_pairing() {
        let id = host_id();

        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(
            id.chars()
                .all(|c| c == '-' || c.is_ascii_digit() || c.is_ascii_uppercase())
        );
        assert_ne!(id, host_id());
    }
}
