//! TLS as wireless debugging has it, for pairing and for the connections
//! after: TLS 1.3, in which this computer proves its identity with a
//! certificate for its ADB key, and the device with one for a key of its
//! own, which nobody vouches for.

use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use super::connection::Stream;
use super::key::AdbKey;
use crate::tls::AnyDevice;

/// A TLS connection to a device.
pub type Tls<'a> = StreamOwned<ClientConnection, Box<dyn Stream + 'a>>;

/// Makes a TLS connection to the device at the other end of `stream`,
/// proving OmniBuild's identity with `key`, and returns it once the
/// handshake is done.
pub fn connect<'a>(mut stream: Box<dyn Stream + 'a>, key: &AdbKey) -> Result<Tls<'a>> {
    let provider = rustls_rustcrypto::provider();
    let device = AnyDevice(provider.signature_verification_algorithms);
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("the provider has TLS 1.3's cipher suites")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(device))
        .with_client_auth_cert(
            vec![CertificateDer::from(key.certificate())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.private_key())),
        )
        .expect("the provider signs with RSA keys");
    // A device has no name to check, and adb sends none: rustls sends none
    // for an address.
    let name = ServerName::IpAddress(Ipv4Addr::UNSPECIFIED.into());
    let mut connection =
        ClientConnection::new(Arc::new(config), name).expect("the configuration is complete");
    while connection.is_handshaking() {
        connection
            .complete_io(&mut stream)
            .context("couldn't make an encrypted connection to the device")?;
    }
    Ok(StreamOwned::new(connection, stream))
}

#[cfg(test)]
pub mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::{Arc, LazyLock};
    use std::thread::JoinHandle;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

    use super::connect;
    use crate::adb::key::AdbKey;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb");
    static KEY: LazyLock<AdbKey> = LazyLock::new(|| fixture("adbkey"));
    /// A key of a device's own, which it makes its certificate with.
    pub static DEVICE_KEY: LazyLock<AdbKey> = LazyLock::new(|| fixture("devicekey"));

    fn fixture(name: &str) -> AdbKey {
        AdbKey::at(&Path::new(FIXTURES).join(name), "omnibuild@test")
            .unwrap()
            .0
    }

    /// A device's end of a TLS connection.
    pub type DeviceTls = StreamOwned<ServerConnection, TcpStream>;

    /// Takes the TLS connection a computer makes over `stream`, as a device
    /// does, which goes on only if the computer proves it holds `trusted`,
    /// the one key the device trusts: rustls checks the computer's
    /// certificate is signed with it, as adbd checks its key is one it
    /// knows.
    pub fn accept(stream: TcpStream, trusted: &AdbKey) -> DeviceTls {
        let provider = Arc::new(rustls_rustcrypto::provider());
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(trusted.certificate()))
            .unwrap();
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
                .unwrap();
        let config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![CertificateDer::from(DEVICE_KEY.certificate())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(DEVICE_KEY.private_key())),
            )
            .unwrap();
        StreamOwned::new(ServerConnection::new(Arc::new(config)).unwrap(), stream)
    }

    /// A device on `listener` that takes one TLS connection, trusting
    /// `trusted`, and answers what it reads with the same in capitals,
    /// however the reads split it, until reading fails. Returns what it
    /// read, and why reading failed.
    fn capitalizing(
        listener: TcpListener,
        trusted: &'static AdbKey,
    ) -> JoinHandle<(Vec<u8>, String)> {
        std::thread::spawn(move || {
            let mut tls = accept(listener.accept().unwrap().0, trusted);
            let mut read = Vec::new();
            let mut buf = [0; 5];
            loop {
                match tls.read(&mut buf) {
                    Ok(len) => {
                        read.extend(&buf[..len]);
                        tls.write_all(&buf[..len].to_ascii_uppercase()).unwrap();
                    }
                    Err(error) => return (read, error.to_string()),
                }
            }
        })
    }

    #[test]
    fn talks_to_a_device_that_trusts_the_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let device = capitalizing(listener, &KEY);

        let mut tls = connect(Box::new(TcpStream::connect(address).unwrap()), &KEY).unwrap();
        tls.write_all(b"stressed").unwrap();
        let mut answer = [0; 8];
        tls.read_exact(&mut answer).unwrap();
        drop(tls);

        assert_eq!(&answer, b"STRESSED");
        // OmniBuild hangs up as adb does, with no TLS goodbye.
        let (read, error) = device.join().unwrap();
        assert_eq!(read, b"stressed");
        assert!(
            error.contains("without sending TLS close_notify"),
            "{error}"
        );
    }

    #[test]
    fn hears_from_a_device_that_doesnt() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let device = capitalizing(listener, &DEVICE_KEY);

        // The handshake ends before the device checks the certificate, as
        // in TLS 1.3, so the device's refusal comes after.
        let mut tls = connect(Box::new(TcpStream::connect(address).unwrap()), &KEY).unwrap();
        let error = tls.read(&mut [0; 1]).err().unwrap();

        assert!(
            error.to_string().starts_with("received fatal alert"),
            "{error}"
        );
        let (read, error) = device.join().unwrap();
        assert_eq!(read, b"");
        assert!(error.starts_with("invalid peer certificate"), "{error}");
    }

    #[test]
    fn says_when_the_handshake_fails() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        // Something that hangs up at once.
        let device = std::thread::spawn(move || drop(listener.accept().unwrap()));

        let error = connect(Box::new(TcpStream::connect(address).unwrap()), &KEY)
            .err()
            .unwrap();

        device.join().unwrap();
        assert!(
            format!("{error:#}")
                .starts_with("couldn't make an encrypted connection to the device: "),
            "{error:#}"
        );
    }
}
