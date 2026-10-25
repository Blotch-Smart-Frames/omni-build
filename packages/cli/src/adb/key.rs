//! The key OmniBuild proves itself to Android devices with: adb's own, in
//! `~/.android/adbkey`, so a device that trusts the user's adb trusts
//! OmniBuild too, and the other way round.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::rand_core::OsRng;
use rsa::signature::Keypair;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Sign, RsaPrivateKey};
use sha1::Sha1;
use sha2::Sha256;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::der::Encode;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfoOwned;
use x509_cert::time::Validity;

use crate::key;

/// The size of every key a device takes: Android's public key format
/// (`android_pubkey.h`) has room for 2048-bit moduli only.
const BITS: usize = 2048;
const WORDS: usize = BITS / 32;
/// The name adb gives the certificate it makes for its key, as subject and
/// issuer both.
const CERTIFICATE_NAME: &str = "CN=Adb,O=Android,C=US";
/// How long adb's certificate lasts: ten years.
const CERTIFICATE_VALIDITY: Duration = Duration::from_hours(10 * 365 * 24);

/// An RSA key, and its public half as a device takes it.
pub struct AdbKey {
    key: RsaPrivateKey,
    /// The public key in Android's format, in base64, and after a space who
    /// it belongs to, as adb writes `adbkey.pub`.
    public_key: String,
}

impl AdbKey {
    /// The key at `path`, which OmniBuild creates there, as adb does, if
    /// it's not there yet, with its public key beside it, in
    /// `<path>.pub`. `owner` names whoever's key it is, for devices to
    /// show. Returns whether it created the key.
    pub fn at(path: &Path, owner: &str) -> Result<(Self, bool)> {
        match fs::read_to_string(path) {
            Ok(pem) => {
                let key = from_pem(&pem)
                    .filter(|key| key.size() * 8 == BITS)
                    .with_context(|| {
                        format!(
                            "{} isn't a {BITS}-bit RSA key, which is all that Android devices take. Delete it to have OmniBuild create a new one",
                            path.display()
                        )
                    })?;
                Ok((Self::new(key, owner), false))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let key = Self::new(
                    RsaPrivateKey::new(&mut OsRng, BITS).expect("the system's random source works"),
                    owner,
                );
                let pem = key
                    .key
                    .to_pkcs8_pem(LineEnding::LF)
                    .expect("an RSA key encodes");
                key::save(path, pem.as_bytes())
                    .and_then(|()| fs::write(public_key_path(path), &key.public_key))
                    .with_context(|| format!("couldn't save the ADB key to {}", path.display()))?;
                Ok((key, true))
            }
            Err(error) => Err(error).with_context(|| format!("couldn't read {}", path.display())),
        }
    }

    fn new(key: RsaPrivateKey, owner: &str) -> Self {
        let public_key = format!("{} {owner}", STANDARD.encode(android_public_key(&key)));
        Self { key, public_key }
    }

    /// The public key, as a device takes it to ask its user whether to
    /// trust it.
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// Signs the token a device sends to check that the host has a key it
    /// trusts. adb signs the token as if it were a SHA-1 digest
    /// (`RSA_sign(NID_sha1, ...)`), so the token's 20 bytes take the
    /// digest's place in a PKCS #1 v1.5 signature. Nothing for any other
    /// size of token.
    pub fn sign(&self, token: &[u8]) -> Option<Vec<u8>> {
        self.key
            .sign_with_rng(&mut OsRng, Pkcs1v15Sign::new::<Sha1>(), token)
            .ok()
    }

    /// A certificate for the key, as DER, which proves this computer's
    /// identity in wireless debugging's TLS. Made as adb makes its own
    /// (`x509_generator.cpp`): self-signed, under adb's name. Devices look
    /// only at the public key in it, which they check they trust.
    pub fn certificate(&self) -> Vec<u8> {
        let signer = pkcs1v15::SigningKey::<Sha256>::new(self.key.clone());
        let name = Name::from_str(CERTIFICATE_NAME).expect("adb's name parses");
        CertificateBuilder::new(
            // An end entity whose key makes digital signatures, as TLS has
            // it sign the handshake.
            Profile::Leaf {
                issuer: name.clone(),
                enable_key_agreement: false,
                enable_key_encipherment: false,
            },
            SerialNumber::from(1u32),
            Validity::from_now(CERTIFICATE_VALIDITY).expect("the validity fits a certificate"),
            name,
            SubjectPublicKeyInfoOwned::from_key(signer.verifying_key())
                .expect("an RSA public key encodes"),
            &signer,
        )
        .and_then(Builder::build::<pkcs1v15::Signature>)
        .expect("a self-signed certificate with valid fields builds")
        .to_der()
        .expect("a certificate encodes")
    }

    /// The private key as PKCS #8 DER, which TLS signs with.
    pub fn private_key(&self) -> Vec<u8> {
        self.key
            .to_pkcs8_der()
            .expect("an RSA key encodes")
            .as_bytes()
            .to_vec()
    }
}

/// Where adb keeps its key: in `.android` in the user's home folder.
pub fn key_path() -> PathBuf {
    // A part at a time, so that the path OmniBuild reports has the
    // system's separators throughout, `\` on Windows.
    std::env::home_dir()
        .unwrap_or_default()
        .join(".android")
        .join("adbkey")
}

/// The public key's file, beside the private key's.
fn public_key_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".pub");
    name.into()
}

/// Who a key belongs to, as adb names them in its public key, from the
/// environment `variable` reads: `user@host`, each `unknown` when the
/// environment doesn't say. Android lists the computers it pairs with for
/// wireless debugging by it.
pub fn owner(variable: impl Fn(&str) -> Option<String>) -> String {
    let first = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| variable(name).filter(|value| !value.is_empty()))
            .unwrap_or_else(|| "unknown".to_owned())
    };
    format!(
        "{}@{}",
        first(&["LOGNAME", "USER", "USERNAME"]),
        first(&["ADBHOSTNAME", "HOSTNAME", "COMPUTERNAME"])
    )
}

/// The private key in a PEM file: PKCS #8, as adb writes it, or PKCS #1,
/// as adb wrote it before.
fn from_pem(pem: &str) -> Option<RsaPrivateKey> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .ok()
}

/// The public key as Android's `RSAPublicKey` structure, which devices keep
/// in `/data/misc/adb/adb_keys`: the modulus' length in 32-bit words,
/// -1/n mod 2^32, the modulus, R² mod n with R = 2^2048, and the exponent,
/// the numbers little-endian. The two precomputed numbers are for the
/// Montgomery multiplication Android verifies signatures with.
fn android_public_key(key: &RsaPrivateKey) -> Vec<u8> {
    let modulus = key.n();
    let low_word = u32::from_le_bytes(little_endian(modulus, 4).try_into().expect("4 bytes"));
    // Newton's iteration for the inverse of the odd low word mod 2^32:
    // the first guess is right to 3 bits, and each step doubles them.
    let mut inverse = low_word;
    for _ in 0..4 {
        inverse = inverse.wrapping_mul(2u32.wrapping_sub(low_word.wrapping_mul(inverse)));
    }
    let rr = (BigUint::from(1u32) << (2 * BITS)) % modulus;
    let exponent = u32::from_le_bytes(little_endian(key.e(), 4).try_into().expect("4 bytes"));

    let mut out = Vec::with_capacity(4 * (3 + 2 * WORDS));
    out.extend(u32::try_from(WORDS).expect("64 words").to_le_bytes());
    out.extend(inverse.wrapping_neg().to_le_bytes());
    out.extend(little_endian(modulus, BITS / 8));
    out.extend(little_endian(&rr, BITS / 8));
    out.extend(exponent.to_le_bytes());
    out
}

/// A number's lowest `len` bytes, little-endian, padded with zeros.
fn little_endian(number: &BigUint, len: usize) -> Vec<u8> {
    let mut bytes = number.to_bytes_le();
    bytes.resize(len, 0);
    bytes
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::rand_core::OsRng;
    use rsa::{Pkcs1v15Sign, RsaPrivateKey};
    use sha1::Sha1;
    use sha2::{Digest, Sha256};
    use x509_cert::Certificate;
    use x509_cert::der::{Decode, Encode};

    use super::{AdbKey, key_path, owner};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/adb")
            .join(name)
    }

    #[test]
    fn writes_the_public_key_as_adb_does() {
        // adbkey and adbkey.pub are a pair `adb keygen` made.
        let (key, created) = AdbKey::at(&fixture("adbkey"), "omnibuild@test").unwrap();

        assert!(!created);
        assert_eq!(
            key.public_key(),
            fs::read_to_string(fixture("adbkey.pub")).unwrap()
        );
    }

    #[test]
    fn signs_a_token_in_place_of_a_sha1_digest() {
        let (key, _) = AdbKey::at(&fixture("adbkey"), "omnibuild@test").unwrap();
        let token = [7; 20];

        let signature = key.sign(&token).unwrap();

        key.key
            .to_public_key()
            .verify(Pkcs1v15Sign::new::<Sha1>(), &token, &signature)
            .unwrap();
        assert_eq!(key.sign(&[7; 19]), None);
    }

    #[test]
    fn makes_a_certificate_for_the_key_as_adb_does() {
        let (key, _) = AdbKey::at(&fixture("adbkey"), "omnibuild@test").unwrap();

        let certificate = Certificate::from_der(&key.certificate()).unwrap();

        let tbs = &certificate.tbs_certificate;
        assert_eq!(tbs.subject.to_string(), "CN=Adb,O=Android,C=US");
        assert_eq!(tbs.issuer, tbs.subject);
        assert_eq!(tbs.serial_number.as_bytes(), [1]);
        let lifetime = tbs
            .validity
            .not_after
            .to_unix_duration()
            .checked_sub(tbs.validity.not_before.to_unix_duration())
            .unwrap();
        assert_eq!(lifetime.as_secs(), 10 * 365 * 24 * 60 * 60);
        // The key's public half, and signed with it.
        let public_key = key.key.to_public_key();
        assert_eq!(
            tbs.subject_public_key_info.to_der().unwrap(),
            public_key.to_public_key_der().unwrap().as_bytes()
        );
        public_key
            .verify(
                Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(tbs.to_der().unwrap()),
                certificate.signature.raw_bytes(),
            )
            .unwrap();
        // And the private key, for TLS to sign with.
        assert_eq!(
            RsaPrivateKey::from_pkcs8_der(&key.private_key()).unwrap(),
            key.key
        );
    }

    #[test]
    fn reads_a_key_in_pkcs1_as_adb_wrote_it_before() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adbkey");
        let (key, _) = AdbKey::at(&fixture("adbkey"), "omnibuild@test").unwrap();
        fs::write(&path, key.key.to_pkcs1_pem(LineEnding::LF).unwrap()).unwrap();

        let (read, created) = AdbKey::at(&path, "omnibuild@test").unwrap();

        assert!(!created);
        assert_eq!(read.public_key(), key.public_key());
    }

    #[test]
    fn creates_a_key_once_with_its_public_key_beside_it() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".android/adbkey");

        let (created, was_created) = AdbKey::at(&path, "jane@laptop").unwrap();
        let (read, read_created) = AdbKey::at(&path, "jane@laptop").unwrap();

        assert!(was_created);
        assert!(!read_created);
        assert_eq!(read.public_key(), created.public_key());
        assert!(created.public_key().ends_with(" jane@laptop"));
        assert_eq!(
            fs::read_to_string(home.path().join(".android/adbkey.pub")).unwrap(),
            created.public_key()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn refuses_a_key_a_device_wouldnt_take() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small");
        let key = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        fs::write(&small, key.to_pkcs1_pem(LineEnding::LF).unwrap()).unwrap();
        let garbage = dir.path().join("garbage");
        fs::write(&garbage, "not a key").unwrap();

        for path in [small, garbage] {
            let error = AdbKey::at(&path, "omnibuild@test").err().unwrap();
            assert!(
                format!("{error:#}").contains("isn't a 2048-bit RSA key"),
                "{error:#}"
            );
        }
    }

    #[test]
    fn says_why_it_cant_read_or_save_the_key() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the key would be can't be read as one.
        let unreadable = AdbKey::at(dir.path(), "omnibuild@test").err().unwrap();
        // Nor can a public key be saved where a folder is.
        fs::create_dir(dir.path().join("adbkey.pub")).unwrap();
        let unsaved = AdbKey::at(&dir.path().join("adbkey"), "omnibuild@test")
            .err()
            .unwrap();

        assert!(
            format!("{unreadable:#}").starts_with("couldn't read"),
            "{unreadable:#}"
        );
        assert!(
            format!("{unsaved:#}").starts_with("couldn't save the ADB key"),
            "{unsaved:#}"
        );
    }

    #[test]
    fn names_the_owner_from_the_environment() {
        let environment = |values: &'static [(&str, &str)]| {
            move |name: &str| {
                values
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };

        assert_eq!(
            owner(environment(&[("USER", "jane"), ("HOSTNAME", "laptop")])),
            "jane@laptop"
        );
        assert_eq!(
            owner(environment(&[("USERNAME", "jane"), ("LOGNAME", "")])),
            "jane@unknown"
        );
        assert_eq!(owner(environment(&[])), "unknown@unknown");
    }

    #[test]
    fn keeps_the_key_in_the_android_folder_at_home() {
        assert!(key_path().ends_with(".android/adbkey"));
    }
}
