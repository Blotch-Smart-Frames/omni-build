//! The keys OmniBuild signs apps with: one the user gives it, in PEM or a
//! PKCS #12 keystore, one it creates for them as an Android upload key, or,
//! for Android, a debug key it makes itself.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rsa::pkcs1v15;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};

use crate::keystore;
use rsa::rand_core::{OsRng, RngCore};
use rsa::signature::{Keypair, SignatureEncoding, Signer};
use rsa::{RsaPrivateKey, sha2::Sha256};
use x509_cert::Certificate;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::der::{Decode, Encode, EncodePem, pem};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfoOwned;
use x509_cert::time::Validity;

/// The debug key's subject. Google Play rejects apps signed with the Android
/// SDK's debug certificate by its name ("CN=Android Debug"), and should
/// reject this one too, but that's for Play to decide: the name at least
/// says where the key came from.
const DEBUG_SUBJECT: &str = "CN=OmniBuild Debug,O=OmniBuild";
/// How long a key OmniBuild creates lasts: as long as the Android SDK's
/// debug key, and well past 2033, which Google Play requires of an upload
/// key's certificate.
const VALIDITY: Duration = Duration::from_hours(30 * 365 * 24);
/// The alias of an upload key in its keystore, which Gradle's signing
/// configuration names, as keytool's documentation does.
pub const ALIAS: &str = "upload";
/// keytool and Android Studio refuse a keystore password shorter than this.
const MIN_PASSWORD: usize = 6;

/// An RSA key and its certificate. RSA because it's what Google Play
/// requires of upload keys, what every Android version verifies, and what
/// Apple issues signing certificates for.
pub struct SigningKey {
    signer: pkcs1v15::SigningKey<Sha256>,
    certificate: Certificate,
    /// The certificates that issued it, which an iOS app's signature
    /// carries so a device can trace it to Apple's root.
    chain: Vec<Certificate>,
    /// The certificate as DER, which a signed app carries.
    certificate_der: Vec<u8>,
    /// The public key as a DER `SubjectPublicKeyInfo`, from the certificate:
    /// Android checks that the two match byte for byte.
    public_key: Vec<u8>,
}

impl SigningKey {
    fn new(key: RsaPrivateKey, certificate: Certificate, chain: Vec<Certificate>) -> Self {
        Self {
            signer: pkcs1v15::SigningKey::new(key),
            chain,
            certificate_der: certificate.to_der().expect("a parsed certificate encodes"),
            public_key: certificate
                .tbs_certificate
                .subject_public_key_info
                .to_der()
                .expect("a parsed public key encodes"),
            certificate,
        }
    }

    /// A new key with a self-signed certificate.
    pub fn generate(subject: &Name, validity: Duration) -> Self {
        let key = RsaPrivateKey::new(&mut OsRng, 2048).expect("the system's random source works");
        let signer = pkcs1v15::SigningKey::<Sha256>::new(key.clone());
        let certificate = CertificateBuilder::new(
            // An end entity whose key usage is digital signatures, issued by
            // itself. A CA's certificate is for signing certificates only,
            // and Java's JAR verifier, which checks an AAB, refuses it.
            Profile::Leaf {
                issuer: subject.clone(),
                enable_key_agreement: false,
                enable_key_encipherment: false,
            },
            SerialNumber::from(OsRng.next_u64()),
            Validity::from_now(validity).expect("the validity fits a certificate"),
            subject.clone(),
            SubjectPublicKeyInfoOwned::from_key(signer.verifying_key())
                .expect("an RSA public key encodes"),
            &signer,
        )
        .and_then(Builder::build::<pkcs1v15::Signature>)
        .expect("a self-signed certificate with valid fields builds");
        Self::new(key, certificate, Vec::new())
    }

    /// The key at `path`, which the user gave, with its certificate and any
    /// that issued it: in a PKCS #12 keystore with `password`, or in a PEM
    /// file with the private key as PKCS #8.
    pub fn read(path: &Path, password: &str) -> Result<Self> {
        let bytes = fs::read(path)
            .with_context(|| format!("couldn't read the signing key {}", path.display()))?;
        // PEM is text in armour, perhaps after other text, as `openssl
        // pkcs12` writes each block's attributes. A keystore is DER.
        let key = match std::str::from_utf8(&bytes) {
            Ok(pem) if pem.contains("-----BEGIN") => Self::from_pem(pem),
            _ => {
                let (key, certificates) = keystore::read(&bytes, password)
                    .with_context(|| format!("couldn't read the keystore {}", path.display()))?;
                RsaPrivateKey::from_pkcs8_der(&key)
                    .ok()
                    .and_then(|key| Self::from_parts(key, certificates))
            }
        };
        key.with_context(|| {
            format!(
                "{} doesn't have an RSA private key and its certificate",
                path.display()
            )
        })
    }

    /// A new upload key, with a self-signed certificate for `subject`,
    /// saved at `path`, which mustn't exist yet, in a keystore with
    /// `password`.
    pub fn create(path: &Path, subject: &Name, password: &str) -> Result<Self> {
        if password.chars().count() < MIN_PASSWORD {
            bail!(
                "the keystore's password (OMNI_BUILD_KEY_PASSWORD) has to have at least {MIN_PASSWORD} characters, as keytool requires"
            );
        }
        let key = Self::generate(subject, VALIDITY);
        let private_key = key
            .signer
            .as_ref()
            .to_pkcs8_der()
            .expect("an RSA key encodes");
        let keystore = keystore::write(private_key.as_bytes(), &key.certificate, ALIAS, password)?;
        save(path, &keystore)
            .with_context(|| format!("couldn't save the keystore to {}", path.display()))?;
        Ok(key)
    }

    /// OmniBuild's debug key, at `path`, which it creates there the first
    /// time. The Android SDK's debug key is in a Java keystore; OmniBuild
    /// keeps its own as PEM. Returns whether it created the key.
    pub fn debug(path: &Path) -> Result<(Self, bool)> {
        match fs::read_to_string(path) {
            Ok(pem) => {
                let key = Self::from_pem(&pem).with_context(|| {
                    format!(
                        "{} isn't a key and certificate OmniBuild can read. Delete it to have OmniBuild create a new debug key",
                        path.display()
                    )
                })?;
                Ok((key, false))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let subject = DEBUG_SUBJECT.parse().expect("the subject is a valid name");
                let key = Self::generate(&subject, VALIDITY);
                save(path, key.to_pem().as_bytes()).with_context(|| {
                    format!("couldn't save the debug key to {}", path.display())
                })?;
                Ok((key, true))
            }
            Err(error) => Err(error).with_context(|| format!("couldn't read {}", path.display())),
        }
    }

    /// The private key as PKCS #8 and the certificate, both PEM.
    fn to_pem(&self) -> String {
        let key = self
            .signer
            .as_ref()
            .to_pkcs8_pem(LineEnding::LF)
            .expect("an RSA key encodes");
        let certificate = self
            .certificate
            .to_pem(LineEnding::LF)
            .expect("a parsed certificate encodes");
        format!("{}{certificate}", *key)
    }

    /// The key and its certificates, from PEM.
    fn from_pem(pem: &str) -> Option<Self> {
        let key = RsaPrivateKey::from_pkcs8_pem(pem_block(pem, "PRIVATE KEY")).ok()?;
        let certificates = pem_blocks(pem, "CERTIFICATE")
            .map(|block| {
                // Decoded whole first: der's streaming PEM reader, which
                // `Certificate::from_pem` uses, never returns from some
                // malformed blocks.
                pem::decode_vec(block.as_bytes())
                    .ok()
                    .and_then(|(_, der)| Certificate::from_der(&der).ok())
            })
            .collect::<Option<Vec<_>>>()?;
        Self::from_parts(key, certificates)
    }

    /// The key and its certificate, the one with the key's public key. The
    /// other certificates are its chain, whatever their order.
    fn from_parts(key: RsaPrivateKey, mut certificates: Vec<Certificate>) -> Option<Self> {
        let public_key = key
            .to_public_key()
            .to_public_key_der()
            .expect("an RSA public key encodes");
        let leaf = certificates.iter().position(|certificate| {
            certificate.tbs_certificate.subject_public_key_info.to_der()
                == Ok(public_key.as_bytes().to_vec())
        })?;
        let certificate = certificates.remove(leaf);
        Some(Self::new(key, certificate, certificates))
    }

    pub fn certificate(&self) -> &[u8] {
        &self.certificate_der
    }

    pub fn x509(&self) -> &Certificate {
        &self.certificate
    }

    pub fn chain(&self) -> &[Certificate] {
        &self.chain
    }

    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// An RSASSA-PKCS1-v1_5 signature of the data's SHA-256 digest.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        self.signer.sign(data).to_vec()
    }
}

/// Where OmniBuild keeps its debug key: in the user's home folder, as the
/// Android SDK keeps its own in `~/.android`.
pub fn debug_key_path() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_default()
        .join(".omni-build/android-debug-key.pem")
}

/// The first PEM block with this label, or nothing.
fn pem_block<'a>(pem: &'a str, label: &str) -> &'a str {
    pem_blocks(pem, label).next().unwrap_or_default()
}

/// Each PEM block with this label, in order.
fn pem_blocks<'a>(pem: &'a str, label: &str) -> impl Iterator<Item = &'a str> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut rest = pem;
    std::iter::from_fn(move || {
        let start = rest.find(&begin)?;
        let len = rest[start..].find(&end)? + end.len();
        let block = &rest[start..start + len];
        rest = &rest[start + len..];
        Some(block)
    })
}

/// Writes a key where only the user can read it, and never over another
/// file: anyone with the key can sign updates to the apps signed with it.
pub fn save(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    fs::create_dir_all(path.parent().unwrap_or(path))
        .and_then(|()| options.open(path))
        .and_then(|mut file| file.write_all(contents))
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::LazyLock;
    use std::time::Duration;

    use const_oid::db::rfc5280::ID_CE_KEY_USAGE;
    use rsa::pkcs1v15::{self, Signature, VerifyingKey};
    use rsa::pkcs8::{DecodePublicKey, LineEnding};
    use rsa::sha2::Sha256;
    use rsa::signature::{Keypair, Verifier};
    use x509_cert::Certificate;
    use x509_cert::builder::{Builder, CertificateBuilder, Profile};
    use x509_cert::der::{Decode, EncodePem};
    use x509_cert::ext::pkix::KeyUsage;
    use x509_cert::name::Name;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::spki::SubjectPublicKeyInfoOwned;
    use x509_cert::time::Validity;

    use super::{SigningKey, debug_key_path};

    /// One key for every test: generating an RSA key takes a moment.
    pub static KEY: LazyLock<SigningKey> =
        LazyLock::new(|| SigningKey::generate(&name("CN=Test"), Duration::from_secs(3600)));

    /// The test identity in tests/fixtures: another key.
    static IDENTITY: LazyLock<SigningKey> =
        LazyLock::new(|| SigningKey::read(&fixture("identity.pem"), "").unwrap());

    fn name(text: &str) -> Name {
        text.parse().unwrap()
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    /// The test key with a certificate `issuer` issued for `subject`. It's
    /// signed with the key itself, which is all the tests need of it.
    pub fn issued_key(issuer: Name, subject: &str) -> SigningKey {
        let certificate = CertificateBuilder::new(
            Profile::Leaf {
                issuer,
                enable_key_agreement: false,
                enable_key_encipherment: false,
            },
            SerialNumber::from(1u32),
            Validity::from_now(Duration::from_secs(3600)).unwrap(),
            subject.parse().unwrap(),
            SubjectPublicKeyInfoOwned::from_key(KEY.signer.verifying_key()).unwrap(),
            &KEY.signer,
        )
        .and_then(Builder::build::<pkcs1v15::Signature>)
        .unwrap();
        SigningKey::new(KEY.signer.as_ref().clone(), certificate, Vec::new())
    }

    /// Whether `signature` is the key's signature of `data`.
    pub fn verifies(public_key: &[u8], data: &[u8], signature: &[u8]) -> bool {
        let key = VerifyingKey::<Sha256>::from_public_key_der(public_key).unwrap();
        key.verify(data, &Signature::try_from(signature).unwrap())
            .is_ok()
    }

    #[test]
    fn signs_with_the_key_its_certificate_names() {
        let certificate = Certificate::from_der(KEY.certificate()).unwrap();

        assert_eq!(certificate.tbs_certificate.subject.to_string(), "CN=Test");
        assert_eq!(certificate.tbs_certificate.issuer.to_string(), "CN=Test");
        let key_usage = certificate
            .tbs_certificate
            .extensions
            .unwrap()
            .into_iter()
            .find(|extension| extension.extn_id == ID_CE_KEY_USAGE)
            .unwrap();
        let key_usage = KeyUsage::from_der(key_usage.extn_value.as_bytes()).unwrap();
        assert!(key_usage.digital_signature());
        assert!(!key_usage.key_cert_sign());
        assert!(verifies(KEY.public_key(), b"data", &KEY.sign(b"data")));
        assert!(!verifies(
            KEY.public_key(),
            b"other data",
            &KEY.sign(b"data")
        ));
    }

    #[test]
    fn creates_the_debug_key_once_and_reads_it_back() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("new/key.pem");

        let (created, was_created) = SigningKey::debug(&path).unwrap();
        let (read, was_read_created) = SigningKey::debug(&path).unwrap();

        assert!(was_created);
        assert!(!was_read_created);
        assert_eq!(read.certificate(), created.certificate());
        let certificate = Certificate::from_der(created.certificate()).unwrap();
        assert_eq!(
            certificate.tbs_certificate.subject.to_string(),
            "CN=OmniBuild Debug,O=OmniBuild"
        );
        assert!(verifies(read.public_key(), b"data", &created.sign(b"data")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn rejects_a_debug_key_file_it_cant_read() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("key.pem");
        let key_only = KEY
            .to_pem()
            .split("-----BEGIN CERTIFICATE")
            .next()
            .unwrap()
            .to_owned();

        let unterminated = "-----BEGIN PRIVATE KEY-----\nMIIE";
        for pem in ["not a key", unterminated, &key_only] {
            fs::write(&path, pem).unwrap();

            let error = SigningKey::debug(&path).err().unwrap();

            assert!(
                error
                    .to_string()
                    .ends_with("isn't a key and certificate OmniBuild can read. Delete it to have OmniBuild create a new debug key")
            );
        }
    }

    #[test]
    fn reports_a_debug_key_it_cant_read() {
        let home = tempfile::tempdir().unwrap();

        // A folder where the key should be.
        let error = SigningKey::debug(home.path()).err().unwrap();

        assert!(error.to_string().starts_with("couldn't read "));
    }

    #[cfg(unix)]
    #[test]
    fn reports_a_debug_key_it_cant_save() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("key.pem");
        // Reading follows the link to nothing, and creating the file
        // refuses to replace the link.
        std::os::unix::fs::symlink(home.path().join("nowhere"), &path).unwrap();

        let error = SigningKey::debug(&path).err().unwrap();

        assert!(
            error
                .to_string()
                .starts_with("couldn't save the debug key to ")
        );
    }

    #[test]
    fn reads_a_key_its_certificate_and_their_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");
        // Another certificate first, as some exports put the one that
        // issued the key's, after what openssl writes of each block.
        let issuer = IDENTITY.x509().to_pem(LineEnding::LF).unwrap();
        fs::write(
            &path,
            format!(
                "Bag Attributes\n    friendlyName: test\n{issuer}{}",
                KEY.to_pem()
            ),
        )
        .unwrap();

        let key = SigningKey::read(&path, "").unwrap();

        assert_eq!(key.certificate(), KEY.certificate());
        assert_eq!(key.chain().len(), 1);
        assert_eq!(key.chain()[0], *IDENTITY.x509());
        assert!(verifies(KEY.public_key(), b"data", &key.sign(b"data")));
    }

    #[test]
    fn reports_a_key_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");
        // A key with the certificate of another.
        let other = IDENTITY.to_pem();
        let key = KEY.to_pem();
        let certificate_at = other.find("-----BEGIN CERTIFICATE").unwrap();
        let key_at = key.find("-----BEGIN CERTIFICATE").unwrap();
        fs::write(
            &path,
            format!("{}{}", &key[..key_at], &other[certificate_at..]),
        )
        .unwrap();

        // A key with a certificate that isn't one.
        let broken = dir.path().join("broken.pem");
        fs::write(
            &broken,
            format!(
                "{}-----BEGIN CERTIFICATE-----\nMIIE\n-----END CERTIFICATE-----\n",
                &key[..key_at]
            ),
        )
        .unwrap();

        let missing = SigningKey::read(&dir.path().join("missing.pem"), "")
            .err()
            .unwrap();
        let mismatched = SigningKey::read(&path, "").err().unwrap();
        let unreadable = SigningKey::read(&broken, "").err().unwrap();
        let elliptic = SigningKey::read(&fixture("keystores/ec.p12"), "omnibuild")
            .err()
            .unwrap();
        let wrong_password = SigningKey::read(&fixture("keystores/openssl.p12"), "wrong")
            .err()
            .unwrap();

        assert!(
            missing
                .to_string()
                .starts_with("couldn't read the signing key ")
        );
        for (error, file) in [
            (mismatched, "key.pem"),
            (unreadable, "broken.pem"),
            (elliptic, "ec.p12"),
        ] {
            assert!(error.to_string().ends_with(&format!(
                "{file} doesn't have an RSA private key and its certificate"
            )));
        }
        let wrong_password = format!("{wrong_password:#}");
        assert!(wrong_password.starts_with("couldn't read the keystore "));
        assert!(
            wrong_password.ends_with(
                "openssl.p12: the password isn't its password (OMNI_BUILD_KEY_PASSWORD)"
            )
        );
    }

    #[test]
    fn reads_a_key_from_a_keystore() {
        let key = SigningKey::read(&fixture("keystores/keychain.p12"), "omnibuild").unwrap();

        assert_eq!(key.certificate(), IDENTITY.certificate());
        assert_eq!(key.chain(), &[] as &[Certificate]);
    }

    #[test]
    fn creates_an_upload_key_in_a_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upload.p12");

        let created =
            SigningKey::create(&path, &name("CN=Upload,O=Example"), "a password").unwrap();

        let read = SigningKey::read(&path, "a password").unwrap();
        assert_eq!(read.certificate(), created.certificate());
        assert_eq!(
            read.x509().tbs_certificate.subject.to_string(),
            "CN=Upload,O=Example"
        );
        assert!(verifies(created.public_key(), b"data", &read.sign(b"data")));
        // Valid for 30 years, well past 2033.
        let validity = &read.x509().tbs_certificate.validity;
        let years = validity
            .not_after
            .to_unix_duration()
            .checked_sub(validity.not_before.to_unix_duration())
            .unwrap()
            .as_secs()
            / (365 * 24 * 3600);
        assert_eq!(years, 30);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn reports_an_upload_key_it_cant_create() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.p12");
        fs::write(&existing, "a key").unwrap();
        let subject = name("CN=Upload");

        let short = SigningKey::create(&dir.path().join("short.p12"), &subject, "12345")
            .err()
            .unwrap();
        let emoji = SigningKey::create(&dir.path().join("emoji.p12"), &subject, "😀😀😀😀😀😀")
            .err()
            .unwrap();
        let replaced = SigningKey::create(&existing, &subject, "a password")
            .err()
            .unwrap();

        assert_eq!(
            short.to_string(),
            "the keystore's password (OMNI_BUILD_KEY_PASSWORD) has to have at least 6 characters, as keytool requires"
        );
        assert_eq!(
            emoji.to_string(),
            "the password has characters PKCS #12 can't hold"
        );
        assert!(
            replaced
                .to_string()
                .starts_with("couldn't save the keystore to ")
        );
        assert_eq!(fs::read_to_string(&existing).unwrap(), "a key");
    }

    #[test]
    fn keeps_the_debug_key_in_the_home_folder() {
        assert!(debug_key_path().ends_with(".omni-build/android-debug-key.pem"));
    }
}
