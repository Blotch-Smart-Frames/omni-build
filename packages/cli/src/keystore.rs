//! PKCS #12 keystores (`.p12`, and the `.jks` files keytool has made since
//! Java 9): a private key and its certificates, encrypted with a password.
//! Keychain Access exports an Apple signing identity as one, and Android
//! developers keep their upload keys in one.
//!
//! A keystore is a list of "safe contents", plain or encrypted, each a list
//! of bags: certificates, and keys encrypted on their own. A MAC over the
//! whole list, keyed with the password, says whether the password is the
//! right one. Two families of algorithms are in use:
//!
//! - Modern, as OpenSSL 3 and keytool write it: PBES2 (PBKDF2 with
//!   HMAC-SHA256, then AES-256-CBC) for keys and certificates, and an
//!   HMAC-SHA256 MAC.
//! - Legacy, as Keychain Access writes it: 3DES for keys and 40-bit RC2 for
//!   certificates, with keys derived by PKCS #12's own function, and an
//!   HMAC-SHA1 MAC.
//!
//! OmniBuild reads both. It writes the modern one for Android upload keys,
//! and for Apple signing keys a third, PKCS #12's own 3DES for keys and
//! certificates alike, with an HMAC-SHA1 MAC: the only one macOS's Keychain
//! imports, Keychain Access and `security import` alike, which refuse the
//! modern one's MAC and RC2.

use anyhow::{Context, Result, bail};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, InnerIvInit, KeyIvInit};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::encrypted_data::EncryptedData;
use cms::enveloped_data::EncryptedContentInfo;
use const_oid::ObjectIdentifier;
use const_oid::db::rfc5911::{ID_DATA, ID_ENCRYPTED_DATA};
use const_oid::db::rfc5912::{ID_SHA_1, ID_SHA_256};
use des::TdesEde3;
use hmac::{Hmac, Mac};
use pkcs8::pkcs5::{EncryptionScheme, pbes2};
use pkcs12::cert_type::CertBag;
use pkcs12::digest_info::DigestInfo;
use pkcs12::kdf::{Pkcs12KeyType, derive_key_bmp};
use pkcs12::mac_data::MacData;
use pkcs12::pbe_params::{EncryptedPrivateKeyInfo, Pkcs12PbeParams};
use pkcs12::pfx::{Pfx, Version};
use pkcs12::safe_bag::{SafeBag, SafeContents};
use pkcs12::{
    PKCS_12_CERT_BAG_OID, PKCS_12_KEY_BAG_OID, PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC,
    PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC, PKCS_12_PKCS8_KEY_BAG_OID, PKCS_12_X509_CERT_OID,
};
use rc2::Rc2;
use rsa::rand_core::{OsRng, RngCore};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use x509_cert::der::asn1::{BmpString, OctetString, SetOfVec};
use x509_cert::der::{Any, AnyRef, Decode, Encode, EncodeValue, Tagged};
use x509_cert::spki::AlgorithmIdentifierOwned;

/// A Java keystore (JKS), keytool's own format before Java 9, starts with
/// this.
const JKS_MAGIC: &[u8] = &[0xfe, 0xed, 0xfe, 0xed];
/// The bag attributes that name a key and its certificate, and pair them.
const FRIENDLY_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.20");
const LOCAL_KEY_ID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.21");
/// What OmniBuild writes with: keytool's iterations, and lengths for
/// salts that OpenSSL and keytool use.
const ITERATIONS: u32 = 10_000;
const SALT_SIZE: usize = 20;

/// The private key (PKCS #8, DER) and the certificates in a keystore. It
/// must hold one key.
pub fn read(keystore: &[u8], password: &str) -> Result<(Vec<u8>, Vec<Certificate>)> {
    if keystore.starts_with(JKS_MAGIC) {
        bail!(
            "it's a Java keystore (JKS), which OmniBuild doesn't read. keytool converts it to PKCS #12: keytool -importkeystore -srckeystore old.jks -destkeystore new.p12 -deststoretype pkcs12"
        );
    }
    let bmp = BmpString::from_utf8(password)
        .ok()
        .context("its password has characters PKCS #12 can't hold")?;
    let pfx = Pfx::from_der(keystore).ok();
    let contents = pfx
        .as_ref()
        .filter(|pfx| pfx.auth_safe.content_type == ID_DATA)
        .and_then(|pfx| pfx.auth_safe.content.decode_as::<OctetString>().ok())
        .context("it isn't a PKCS #12 keystore")?;
    if let Some(mac) = pfx.as_ref().and_then(|pfx| pfx.mac_data.as_ref()) {
        verify(mac, contents.as_bytes(), &bmp)?;
    }

    let mut keys = Vec::new();
    let mut certificates = Vec::new();
    for content in Vec::<ContentInfo>::from_der(contents.as_bytes()).unwrap_or_default() {
        let bags = if content.content_type == ID_ENCRYPTED_DATA {
            let data = content.content.decode_as::<EncryptedData>().ok();
            let info = data.as_ref().map(|data| &data.enc_content_info);
            let algorithm = info.map(|info| &info.content_enc_alg);
            let encrypted = info.and_then(|info| info.encrypted_content.as_ref());
            match algorithm.zip(encrypted) {
                Some((algorithm, encrypted)) => {
                    decrypt(algorithm, encrypted.as_bytes(), password, &bmp)?
                }
                None => Vec::new(),
            }
        } else {
            content
                .content
                .decode_as::<OctetString>()
                .map(OctetString::into_bytes)
                .unwrap_or_default()
        };
        // Bags OmniBuild doesn't use, such as secrets and CRLs, are left.
        for bag in SafeContents::from_der(&bags).unwrap_or_default() {
            // pkcs12 reads a bag's value with the explicit tag around it.
            let value = AnyRef::from_der(&bag.bag_value)
                .map(|tagged| tagged.value().to_vec())
                .unwrap_or_default();
            if bag.bag_id == PKCS_12_CERT_BAG_OID {
                certificates.extend(
                    CertBag::from_der(&value)
                        .ok()
                        .filter(|bag| bag.cert_id == PKCS_12_X509_CERT_OID)
                        .and_then(|bag| Certificate::from_der(bag.cert_value.as_bytes()).ok()),
                );
            } else if bag.bag_id == PKCS_12_PKCS8_KEY_BAG_OID {
                let key = EncryptedPrivateKeyInfo::from_der(&value)
                    .ok()
                    .context("it has a key that isn't encrypted as PKCS #12 says")?;
                keys.push(decrypt(
                    &key.encryption_algorithm,
                    key.encrypted_data.as_bytes(),
                    password,
                    &bmp,
                )?);
            } else if bag.bag_id == PKCS_12_KEY_BAG_OID {
                keys.push(value);
            }
        }
    }
    match keys.len() {
        1 => Ok((keys.remove(0), certificates)),
        0 => bail!("it has no private key"),
        count => bail!("it has {count} private keys, and OmniBuild takes a keystore with one"),
    }
}

/// Checks the MAC over a keystore's contents, which only the right
/// password reproduces.
fn verify(mac: &MacData, contents: &[u8], password: &BmpString) -> Result<()> {
    let salt = mac.mac_salt.as_bytes();
    let expected = mac.mac.digest.as_bytes();
    let matches = match mac.mac.algorithm.oid {
        ID_SHA_1 => {
            let key = derive_key_bmp::<Sha1>(
                password.clone(),
                salt,
                Pkcs12KeyType::Mac,
                mac.iterations,
                20,
            );
            Hmac::<Sha1>::new_from_slice(&key)
                .expect("HMAC takes a key of any size")
                .chain_update(contents)
                .verify_slice(expected)
                .is_ok()
        }
        ID_SHA_256 => {
            let key = derive_key_bmp::<Sha256>(
                password.clone(),
                salt,
                Pkcs12KeyType::Mac,
                mac.iterations,
                32,
            );
            Hmac::<Sha256>::new_from_slice(&key)
                .expect("HMAC takes a key of any size")
                .chain_update(contents)
                .verify_slice(expected)
                .is_ok()
        }
        oid => bail!("its MAC is made with {oid}, which OmniBuild doesn't know"),
    };
    if !matches {
        bail!("the password isn't its password (OMNI_BUILD_KEY_PASSWORD)");
    }
    Ok(())
}

/// Decrypts what's encrypted with `algorithm`: PBES2, which takes the
/// password as UTF-8, or one of PKCS #12's own schemes, which take it as
/// UTF-16.
fn decrypt(
    algorithm: &AlgorithmIdentifierOwned,
    encrypted: &[u8],
    password: &str,
    bmp: &BmpString,
) -> Result<Vec<u8>> {
    let pkcs12 = || {
        algorithm
            .parameters
            .as_ref()
            .and_then(|parameters| parameters.decode_as::<Pkcs12PbeParams>().ok())
    };
    // Each derives a key and an IV from the password, then decrypts in CBC
    // mode. A wrong password usually shows as padding that doesn't check.
    let decrypted = match algorithm.oid {
        pbes2::PBES2_OID => algorithm.to_der().ok().and_then(|der| {
            EncryptionScheme::try_from(der.as_slice())
                .ok()
                .and_then(|scheme| scheme.decrypt(password, encrypted).ok())
        }),
        PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC => pkcs12().and_then(|parameters| {
            let (key, iv) = derive(&parameters, bmp, 24);
            cbc::Decryptor::<TdesEde3>::new_from_slices(&key, &iv)
                .expect("the key and IV are 3DES's sizes")
                .decrypt_padded_vec_mut::<Pkcs7>(encrypted)
                .ok()
        }),
        PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC => pkcs12().and_then(|parameters| {
            let (key, iv) = derive(&parameters, bmp, 5);
            let cipher = Rc2::new_with_eff_key_len(&key, 40);
            cbc::Decryptor::<Rc2>::inner_iv_slice_init(cipher, &iv)
                .expect("the IV is RC2's size")
                .decrypt_padded_vec_mut::<Pkcs7>(encrypted)
                .ok()
        }),
        oid => bail!("it's encrypted with {oid}, which OmniBuild doesn't know"),
    };
    decrypted.context("couldn't decrypt it: the password is wrong, or it's corrupt")
}

/// A key of `size` bytes and an 8-byte IV, by PKCS #12's own derivation.
fn derive(parameters: &Pkcs12PbeParams, password: &BmpString, size: usize) -> (Vec<u8>, Vec<u8>) {
    let salt = parameters.salt.as_bytes();
    let rounds = parameters.iterations;
    (
        derive_key_bmp::<Sha1>(
            password.clone(),
            salt,
            Pkcs12KeyType::EncryptionKey,
            rounds,
            size,
        ),
        derive_key_bmp::<Sha1>(password.clone(), salt, Pkcs12KeyType::Iv, rounds, 8),
    )
}

/// How a keystore OmniBuild writes is encrypted.
#[derive(Clone, Copy)]
pub enum Encryption {
    /// PBES2 (PBKDF2 with HMAC-SHA256, then AES-256-CBC) and an HMAC-SHA256
    /// MAC, as keytool and OpenSSL 3 write a keystore.
    Modern,
    /// PKCS #12's own 3DES and an HMAC-SHA1 MAC, as `openssl pkcs12 -export
    /// -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1` writes a
    /// keystore: what macOS's Keychain imports. OpenSSL 3 reads it without
    /// its legacy provider, which Keychain Access's RC2 would need.
    Keychain,
}

/// A keystore with `key` (PKCS #8, DER) and its certificate, if it has one
/// yet, under `alias`: the certificate and the key each encrypted, and a MAC
/// over both. Java, Android Studio and OpenSSL read it, and with
/// `Encryption::Keychain`, macOS's Keychain.
pub fn write(
    key: &[u8],
    certificate: Option<&Certificate>,
    alias: &str,
    password: &str,
    encryption: Encryption,
) -> Result<Vec<u8>> {
    let bmp = BmpString::from_utf8(password)
        .ok()
        .context("the password has characters PKCS #12 can't hold")?;
    let mut attributes = Vec::new();
    // A name PKCS #12 can't hold, such as one with an emoji, is left out:
    // nothing needs it.
    if let Ok(alias) = BmpString::from_utf8(alias) {
        attributes.push(attribute(FRIENDLY_NAME, &alias));
    }
    // Both bags carry the same ID, which keytool pairs the key and its
    // certificate by.
    let certificate = certificate.map(der);
    if let Some(certificate) = &certificate {
        let local_key_id = Sha1::digest(certificate).to_vec();
        attributes.push(attribute(
            LOCAL_KEY_ID,
            &OctetString::new(local_key_id).expect("a digest fits"),
        ));
    }
    let attributes = SetOfVec::try_from(attributes).expect("the attributes are distinct");

    let mut contents = Vec::new();
    if let Some(certificate) = certificate {
        let certificates = der(&vec![SafeBag {
            bag_id: PKCS_12_CERT_BAG_OID,
            bag_value: der(&CertBag {
                cert_id: PKCS_12_X509_CERT_OID,
                cert_value: OctetString::new(certificate).expect("a certificate fits"),
            }),
            bag_attributes: Some(attributes.clone()),
        }]);
        let (algorithm, encrypted) = encrypt(encryption, &certificates, password, &bmp);
        let encrypted_certificates = EncryptedData {
            version: CmsVersion::V0,
            enc_content_info: EncryptedContentInfo {
                content_type: ID_DATA,
                content_enc_alg: algorithm,
                encrypted_content: Some(OctetString::new(encrypted).expect("the certificates fit")),
            },
            unprotected_attrs: None,
        };
        contents.push(ContentInfo {
            content_type: ID_ENCRYPTED_DATA,
            content: Any::encode_from(&encrypted_certificates).expect("it encodes"),
        });
    }
    let (algorithm, encrypted) = encrypt(encryption, key, password, &bmp);
    let encrypted_key = EncryptedPrivateKeyInfo {
        encryption_algorithm: algorithm,
        encrypted_data: OctetString::new(encrypted).expect("the key fits"),
    };
    let keys = der(&vec![SafeBag {
        bag_id: PKCS_12_PKCS8_KEY_BAG_OID,
        bag_value: der(&encrypted_key),
        bag_attributes: Some(attributes),
    }]);
    contents.push(ContentInfo {
        content_type: ID_DATA,
        content: Any::encode_from(&OctetString::new(keys).expect("the key fits"))
            .expect("it encodes"),
    });

    let contents = der(&contents);
    let salt = random::<SALT_SIZE>();
    let iterations = i32::try_from(ITERATIONS).expect("the iterations fit");
    let (oid, mac) = match encryption {
        Encryption::Modern => {
            let mac_key = derive_key_bmp::<Sha256>(bmp, &salt, Pkcs12KeyType::Mac, iterations, 32);
            let mac = Hmac::<Sha256>::new_from_slice(&mac_key)
                .expect("HMAC takes a key of any size")
                .chain_update(&contents)
                .finalize();
            (ID_SHA_256, mac.into_bytes().to_vec())
        }
        Encryption::Keychain => {
            let mac_key = derive_key_bmp::<Sha1>(bmp, &salt, Pkcs12KeyType::Mac, iterations, 20);
            let mac = Hmac::<Sha1>::new_from_slice(&mac_key)
                .expect("HMAC takes a key of any size")
                .chain_update(&contents)
                .finalize();
            (ID_SHA_1, mac.into_bytes().to_vec())
        }
    };
    Ok(der(&Pfx {
        version: Version::V3,
        auth_safe: ContentInfo {
            content_type: ID_DATA,
            content: Any::encode_from(&OctetString::new(contents).expect("the contents fit"))
                .expect("it encodes"),
        },
        mac_data: Some(MacData {
            mac: DigestInfo {
                algorithm: AlgorithmIdentifierOwned {
                    oid,
                    parameters: Some(Any::null()),
                },
                digest: OctetString::new(mac).expect("a MAC fits"),
            },
            mac_salt: OctetString::new(salt.to_vec()).expect("a salt fits"),
            iterations,
        }),
    }))
}

/// `data` encrypted with a new salt, and the algorithm that decrypts it.
fn encrypt(
    encryption: Encryption,
    data: &[u8],
    password: &str,
    bmp: &BmpString,
) -> (AlgorithmIdentifierOwned, Vec<u8>) {
    let salt = random::<SALT_SIZE>();
    match encryption {
        Encryption::Modern => {
            let iv = random::<16>();
            let scheme = EncryptionScheme::Pbes2(
                pbes2::Parameters::pbkdf2_sha256_aes256cbc(ITERATIONS, &salt, &iv)
                    .expect("the parameters are valid"),
            );
            (
                AlgorithmIdentifierOwned::from_der(&der(&scheme))
                    .expect("PBES2's parameters are an algorithm identifier"),
                scheme
                    .encrypt(password, data)
                    .expect("encrypting into memory can't fail"),
            )
        }
        Encryption::Keychain => {
            let parameters = Pkcs12PbeParams {
                salt: OctetString::new(salt.to_vec()).expect("a salt fits"),
                iterations: i32::try_from(ITERATIONS).expect("the iterations fit"),
            };
            let (key, iv) = derive(&parameters, bmp, 24);
            (
                AlgorithmIdentifierOwned {
                    oid: PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC,
                    parameters: Some(Any::encode_from(&parameters).expect("it encodes")),
                },
                cbc::Encryptor::<TdesEde3>::new_from_slices(&key, &iv)
                    .expect("the key and IV are 3DES's sizes")
                    .encrypt_padded_vec_mut::<Pkcs7>(data),
            )
        }
    }
}

fn attribute<T: Tagged + EncodeValue>(oid: ObjectIdentifier, value: &T) -> Attribute {
    Attribute {
        oid,
        values: SetOfVec::try_from(vec![Any::encode_from(value).expect("it encodes")])
            .expect("one value is a set"),
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn der(value: &impl Encode) -> Vec<u8> {
    value.to_der().expect("it encodes")
}

#[cfg(test)]
mod tests {
    use const_oid::ObjectIdentifier;
    use const_oid::db::rfc5911::{ID_DATA, ID_ENCRYPTED_DATA};
    use pkcs12::pbe_params::EncryptedPrivateKeyInfo;
    use pkcs12::pfx::Pfx;
    use pkcs12::safe_bag::SafeBag;
    use pkcs12::{PKCS_12_PKCS8_KEY_BAG_OID, PKCS_12_SECRET_BAG_OID};
    use rsa::RsaPrivateKey;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
    use x509_cert::Certificate;
    use x509_cert::der::asn1::OctetString;
    use x509_cert::der::{Any, AnyRef, Decode, Encode};
    use x509_cert::spki::AlgorithmIdentifierOwned;

    use super::{Encryption, read, write};
    use cms::content_info::ContentInfo;
    use cms::encrypted_data::EncryptedData;
    use const_oid::db::rfc5912::{ID_SHA_1, ID_SHA_256};
    use pkcs8::pkcs5::pbes2;
    use pkcs12::PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC;
    use pkcs12::safe_bag::SafeContents;

    /// Keystores made by OpenSSL, Keychain Access and keytool, as
    /// tests/fixtures/README.md says, with the password "omnibuild".
    macro_rules! keystore {
        ($name:literal) => {
            include_bytes!(concat!("../tests/fixtures/keystores/", $name)).as_slice()
        };
    }
    const PASSWORD: &str = "omnibuild";
    const TEST_IDENTITY: &str = "O=OmniBuild,OU=ABCDE12345,CN=OmniBuild Test";

    /// The subjects of the certificates, and whether the key is theirs.
    fn check((key, certificates): (Vec<u8>, Vec<Certificate>)) -> (Vec<String>, bool) {
        let public_key = RsaPrivateKey::from_pkcs8_der(&key)
            .unwrap()
            .to_public_key()
            .to_public_key_der()
            .unwrap();
        let theirs = certificates.iter().any(|certificate| {
            certificate
                .tbs_certificate
                .subject_public_key_info
                .to_der()
                .unwrap()
                == public_key.as_bytes()
        });
        let subjects = certificates
            .iter()
            .map(|certificate| certificate.tbs_certificate.subject.to_string())
            .collect();
        (subjects, theirs)
    }

    /// A keystore of these contents, with no MAC.
    fn keystore_of(contents: &[ContentInfo]) -> Vec<u8> {
        let mut pfx = Pfx::from_der(keystore!("unencrypted.p12")).unwrap();
        pfx.auth_safe.content =
            Any::encode_from(&OctetString::new(contents.to_vec().to_der().unwrap()).unwrap())
                .unwrap();
        pfx.mac_data = None;
        pfx.to_der().unwrap()
    }

    /// The contents of a keystore with no MAC.
    fn contents_of(keystore: &[u8]) -> Vec<ContentInfo> {
        let pfx = Pfx::from_der(keystore).unwrap();
        let contents = pfx.auth_safe.content.decode_as::<OctetString>().unwrap();
        Vec::<ContentInfo>::from_der(contents.as_bytes()).unwrap()
    }

    /// Plain contents with these bags.
    fn bags(bags: &[SafeBag]) -> ContentInfo {
        ContentInfo {
            content_type: ID_DATA,
            content: Any::encode_from(&OctetString::new(bags.to_vec().to_der().unwrap()).unwrap())
                .unwrap(),
        }
    }

    #[test]
    fn reads_keystores_from_openssl_keychain_access_and_keytool() {
        for (keystore, password, subject) in [
            (keystore!("openssl.p12"), PASSWORD, TEST_IDENTITY),
            (keystore!("keychain.p12"), PASSWORD, TEST_IDENTITY),
            (keystore!("no-password.p12"), "", TEST_IDENTITY),
            (keystore!("unencrypted.p12"), PASSWORD, TEST_IDENTITY),
            (keystore!("keytool.p12"), PASSWORD, "CN=Upload,O=OmniBuild"),
        ] {
            assert_eq!(
                check(read(keystore, password).unwrap()),
                (vec![subject.to_owned()], true),
                "{subject}"
            );
        }
    }

    #[test]
    fn writes_a_keystore_it_reads_back() {
        let (key, certificates) = read(keystore!("openssl.p12"), PASSWORD).unwrap();

        for encryption in [Encryption::Modern, Encryption::Keychain] {
            let keystore = write(
                &key,
                Some(&certificates[0]),
                "upload",
                "a password",
                encryption,
            )
            .unwrap();

            let (read_key, read_certificates) = read(&keystore, "a password").unwrap();
            assert_eq!(read_key, key);
            assert_eq!(read_certificates, certificates);
            // Not with another password.
            assert_eq!(
                read(&keystore, PASSWORD).unwrap_err().to_string(),
                "the password isn't its password (OMNI_BUILD_KEY_PASSWORD)"
            );
            assert_eq!(
                write(&key, Some(&certificates[0]), "upload", "😀", encryption)
                    .unwrap_err()
                    .to_string(),
                "the password has characters PKCS #12 can't hold"
            );
        }
    }

    #[test]
    fn writes_a_key_with_no_certificate_yet_and_names_it_if_it_can() {
        let (key, _) = read(keystore!("openssl.p12"), PASSWORD).unwrap();

        for alias in ["Jane Doe key", "😀"] {
            let keystore = write(&key, None, alias, "a password", Encryption::Keychain).unwrap();

            assert_eq!(
                read(&keystore, "a password").unwrap(),
                (key.clone(), vec![])
            );
        }
    }

    /// The algorithms a keystore's MAC, certificates and key are made with.
    fn algorithms(keystore: &[u8]) -> Vec<ObjectIdentifier> {
        let pfx = Pfx::from_der(keystore).unwrap();
        let mut algorithms = vec![pfx.mac_data.unwrap().mac.algorithm.oid];
        let contents = pfx.auth_safe.content.decode_as::<OctetString>().unwrap();
        for content in Vec::<ContentInfo>::from_der(contents.as_bytes()).unwrap() {
            if content.content_type == ID_ENCRYPTED_DATA {
                let data = content.content.decode_as::<EncryptedData>().unwrap();
                algorithms.push(data.enc_content_info.content_enc_alg.oid);
            } else {
                let bags = content.content.decode_as::<OctetString>().unwrap();
                for bag in SafeContents::from_der(bags.as_bytes()).unwrap() {
                    // With the explicit tag around it, as read() takes it.
                    let value = AnyRef::from_der(&bag.bag_value).unwrap();
                    let key = EncryptedPrivateKeyInfo::from_der(value.value()).unwrap();
                    algorithms.push(key.encryption_algorithm.oid);
                }
            }
        }
        algorithms
    }

    #[test]
    fn writes_what_keytool_writes_or_what_macos_keychain_imports() {
        let (key, certificates) = read(keystore!("openssl.p12"), PASSWORD).unwrap();
        let keystore = |encryption| {
            write(
                &key,
                Some(&certificates[0]),
                "key",
                "a password",
                encryption,
            )
        };

        // An HMAC-SHA256 MAC, and PBES2 for the certificate and the key.
        assert_eq!(
            algorithms(&keystore(Encryption::Modern).unwrap()),
            [ID_SHA_256, pbes2::PBES2_OID, pbes2::PBES2_OID]
        );
        // Keychain takes an HMAC-SHA1 MAC, and 3DES for both, alone.
        let triple_des = PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC;
        assert_eq!(
            algorithms(&keystore(Encryption::Keychain).unwrap()),
            [ID_SHA_1, triple_des, triple_des]
        );
    }

    #[test]
    fn skips_what_it_doesnt_use() {
        let mut contents = contents_of(keystore!("unencrypted.p12"));
        contents.push(ContentInfo {
            content_type: ID_ENCRYPTED_DATA,
            content: Any::null(),
        });
        contents.push(bags(&[SafeBag {
            bag_id: PKCS_12_SECRET_BAG_OID,
            bag_value: Any::null().to_der().unwrap(),
            bag_attributes: None,
        }]));

        let read = read(&keystore_of(&contents), PASSWORD).unwrap();

        assert_eq!(check(read), (vec![TEST_IDENTITY.to_owned()], true));
    }

    #[test]
    fn reports_a_keystore_it_cant_read() {
        let mut no_mac = Pfx::from_der(keystore!("openssl.p12")).unwrap();
        no_mac.mac_data = None;
        let mut md5_mac = Pfx::from_der(keystore!("openssl.p12")).unwrap();
        md5_mac.mac_data.as_mut().unwrap().mac.algorithm.oid =
            ObjectIdentifier::new_unwrap("1.2.840.113549.2.5");
        let key_bag = |value: Vec<u8>| {
            keystore_of(&[bags(&[SafeBag {
                bag_id: PKCS_12_PKCS8_KEY_BAG_OID,
                bag_value: value,
                bag_attributes: None,
            }])])
        };
        let unknown_cipher = EncryptedPrivateKeyInfo {
            encryption_algorithm: AlgorithmIdentifierOwned {
                oid: ObjectIdentifier::new_unwrap("1.2.3.4"),
                parameters: None,
            },
            encrypted_data: OctetString::new(vec![0; 16]).unwrap(),
        };
        let cases = [
            (
                vec![0xfe, 0xed, 0xfe, 0xed, 0, 0, 0, 2],
                PASSWORD,
                "it's a Java keystore (JKS), which OmniBuild doesn't read. ",
            ),
            (
                b"not a keystore".to_vec(),
                PASSWORD,
                "it isn't a PKCS #12 keystore",
            ),
            (
                keystore!("openssl.p12").to_vec(),
                "😀",
                "its password has characters PKCS #12 can't hold",
            ),
            (
                keystore!("openssl.p12").to_vec(),
                "wrong",
                "the password isn't its password (OMNI_BUILD_KEY_PASSWORD)",
            ),
            (
                keystore!("keychain.p12").to_vec(),
                "wrong",
                "the password isn't its password (OMNI_BUILD_KEY_PASSWORD)",
            ),
            (
                md5_mac.to_der().unwrap(),
                PASSWORD,
                "its MAC is made with 1.2.840.113549.2.5, which OmniBuild doesn't know",
            ),
            (
                no_mac.to_der().unwrap(),
                "wrong",
                "couldn't decrypt it: the password is wrong, or it's corrupt",
            ),
            (
                key_bag(Any::null().to_der().unwrap()),
                PASSWORD,
                "it has a key that isn't encrypted as PKCS #12 says",
            ),
            (
                key_bag(unknown_cipher.to_der().unwrap()),
                PASSWORD,
                "it's encrypted with 1.2.3.4, which OmniBuild doesn't know",
            ),
            (
                keystore!("no-key.p12").to_vec(),
                PASSWORD,
                "it has no private key",
            ),
            (
                keystore!("two-keys.p12").to_vec(),
                PASSWORD,
                "it has 2 private keys, and OmniBuild takes a keystore with one",
            ),
        ];

        for (keystore, password, message) in cases {
            let error = read(&keystore, password).unwrap_err().to_string();

            assert!(error.starts_with(message), "{error}");
        }
    }
}
