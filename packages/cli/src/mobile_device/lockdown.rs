//! lockdownd, the service on port 62078 of every iPhone and iPad, which a
//! computer talks to first: it says what the device is, pairs with a
//! computer the device's user trusts, and, in a session encrypted with the
//! pair record's keys, starts the device's other services for it.

use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use super::pairing::PairRecord;
use super::{Stream, receive, send};
use crate::tls::AnyDevice;

/// Where lockdownd listens.
pub const PORT: u16 = 62078;
/// What OmniBuild calls itself in its requests, which the device logs.
const LABEL: &str = "omni";
/// The version of pairing that iOS 7 and later speak.
const PROTOCOL_VERSION: &str = "2";

/// How long OmniBuild waits for a device's user to answer whether to
/// trust this computer, and how often it asks the device whether they have.
pub struct Patience {
    pub poll: Duration,
    pub limit: Duration,
}

/// A connection to lockdownd.
pub struct Lockdown {
    stream: Box<dyn Stream>,
}

impl Lockdown {
    /// lockdownd, at the other end of `stream`.
    pub fn new(stream: Box<dyn Stream>) -> Result<Self> {
        let mut lockdown = Self { stream };
        let answer = lockdown.request("QueryType", Dictionary::new())?;
        // Another service answers there while the device is in recovery,
        // or being restored.
        if answer.get("Type").and_then(Value::as_string) != Some("com.apple.mobile.lockdown") {
            bail!("the device isn't running iOS: lockdownd doesn't answer");
        }
        Ok(lockdown)
    }

    /// What the device says of itself, before it's paired: its name, its
    /// public key, its Wi-Fi address and more.
    pub fn values(&mut self) -> Result<Dictionary> {
        let answer = self.request("GetValue", Dictionary::new())?;
        answer
            .get("Value")
            .and_then(Value::as_dictionary)
            .cloned()
            .context("lockdownd answered GetValue with no values")
    }

    /// Pairs this computer with the device with `record`, which the device
    /// asks its user whether to trust: calls `trusting` when it does, then
    /// asks the device again every `patience.poll`, until the user answers
    /// or `patience.limit` runs out. Returns what the device gives for the
    /// record: its escrow bag, if any.
    pub fn pair(
        &mut self,
        record: &PairRecord,
        trusting: &dyn Fn(),
        patience: &Patience,
    ) -> Result<Option<Vec<u8>>> {
        let mut options = Dictionary::new();
        options.insert("ExtendedPairingErrors".to_owned(), true.into());
        let mut fields = Dictionary::new();
        fields.insert("PairRecord".to_owned(), record.public().into());
        fields.insert("ProtocolVersion".to_owned(), PROTOCOL_VERSION.into());
        fields.insert("PairingOptions".to_owned(), options.into());
        let started = Instant::now();
        let mut asked = false;
        loop {
            let answer = self.exchange("Pair", fields.clone())?;
            match error(&answer) {
                None => {
                    return Ok(answer
                        .get("EscrowBag")
                        .and_then(Value::as_data)
                        .map(<[u8]>::to_vec));
                }
                Some("PairingDialogResponsePending") => {}
                Some(error) => bail!(refusal("Pair", error)),
            }
            if !asked {
                trusting();
                asked = true;
            }
            if started.elapsed() >= patience.limit {
                bail!("the device's user didn't answer whether to trust this computer");
            }
            thread::sleep(patience.poll);
        }
    }

    /// Starts a session, in which the device takes this computer for the
    /// one `record` pairs it with, and encrypts the connection if the
    /// device asks, as it does. Returns false if the device doesn't know
    /// the record: it has forgotten this computer, as it does when its user
    /// resets its trust settings.
    pub fn start_session(&mut self, record: &PairRecord) -> Result<bool> {
        let mut fields = Dictionary::new();
        fields.insert("HostID".to_owned(), record.host_id.as_str().into());
        fields.insert("SystemBUID".to_owned(), record.system_buid.as_str().into());
        let answer = self.exchange("StartSession", fields)?;
        match error(&answer) {
            None => {}
            Some("InvalidHostID") => return Ok(false),
            Some(error) => bail!(refusal("StartSession", error)),
        }
        if answer.get("EnableSessionSSL").and_then(Value::as_boolean) == Some(true) {
            let stream = std::mem::replace(&mut self.stream, Box::new(io::empty()));
            self.stream = encrypt(stream, record)?;
        }
        Ok(true)
    }

    /// Starts the service `name` on the device. Returns the port it
    /// listens on, and whether a connection to it is to be encrypted.
    pub fn start_service(&mut self, name: &str) -> Result<(u16, bool)> {
        let mut fields = Dictionary::new();
        fields.insert("Service".to_owned(), name.into());
        let answer = self.request("StartService", fields)?;
        let port = answer
            .get("Port")
            .and_then(Value::as_unsigned_integer)
            .and_then(|port| u16::try_from(port).ok())
            .with_context(|| format!("lockdownd started {name} without saying where"))?;
        let encrypted = answer.get("EnableServiceSSL").and_then(Value::as_boolean) == Some(true);
        Ok((port, encrypted))
    }

    /// Sends the request `request`, with `fields`, and returns lockdownd's
    /// answer, or why it refused.
    fn request(&mut self, request: &str, fields: Dictionary) -> Result<Dictionary> {
        let answer = self.exchange(request, fields)?;
        if let Some(error) = error(&answer) {
            bail!(refusal(request, error));
        }
        Ok(answer)
    }

    /// Sends the request `request`, with `fields`, and returns lockdownd's
    /// answer, whatever it says.
    fn exchange(&mut self, request: &str, mut fields: Dictionary) -> Result<Dictionary> {
        fields.insert("Label".to_owned(), LABEL.into());
        fields.insert("Request".to_owned(), request.into());
        send(&mut self.stream, &fields)
            .map_err(anyhow::Error::from)
            .and_then(|()| receive(&mut self.stream))
            .with_context(|| format!("lockdownd didn't answer {request}"))
    }
}

/// Why lockdownd refused, if it did.
fn error(answer: &Dictionary) -> Option<&str> {
    answer.get("Error").and_then(Value::as_string)
}

/// Why lockdownd refused `request`, given the error it answered with.
fn refusal(request: &str, error: &str) -> String {
    match error {
        "PasswordProtected" => "the device is locked: unlock it, and try again".to_owned(),
        "UserDeniedPairing" => "the device's user didn't trust this computer".to_owned(),
        error => format!("lockdownd refused {request}: {error}"),
    }
}

/// `stream`, encrypted as lockdownd and the services it starts encrypt
/// theirs: with TLS 1.2 or 1.3, in which this computer shows the host
/// certificate of `record`, which the device trusts since pairing.
pub fn encrypt(mut stream: Box<dyn Stream>, record: &PairRecord) -> Result<Box<dyn Stream>> {
    let certificate = CertificateDer::from_pem_slice(&record.host_certificate)
        .ok()
        .context("the pair record's host certificate isn't one in PEM")?;
    let key = PrivateKeyDer::from_pem_slice(&record.host_private_key)
        .ok()
        .context("the pair record's host key isn't one in PEM")?;
    let provider = rustls_rustcrypto::provider();
    let device = AnyDevice(provider.signature_verification_algorithms);
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("the provider has TLS 1.2's and 1.3's cipher suites")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(device))
        .with_client_auth_cert(vec![certificate], key)
        .ok()
        .context("the pair record's host key isn't one that signs, as an RSA key does")?;
    // The device has no name to check, and libimobiledevice sends none:
    // rustls sends none for an address.
    let name = ServerName::IpAddress(Ipv4Addr::UNSPECIFIED.into());
    let mut connection =
        ClientConnection::new(Arc::new(config), name).expect("the configuration is complete");
    while connection.is_handshaking() {
        connection
            .complete_io(&mut stream)
            .context("couldn't make an encrypted connection to the device")?;
    }
    Ok(Box::new(StreamOwned::new(connection, stream)))
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use plist::Dictionary;

    use super::{Lockdown, Patience, encrypt};
    use crate::mobile_device::pairing::PairRecord;
    use crate::mobile_device::pairing::tests::device_public_key;
    use crate::mobile_device::tests::{read_plist, write_plist};
    use crate::mobile_device::usbmux::tests::dictionary;

    /// A lockdownd on a connection of its own, which answers each request
    /// with the next of `answers`, then hangs up.
    fn answering(answers: Vec<Dictionary>) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            for answer in answers {
                read_plist(&mut stream);
                write_plist(&mut stream, &answer);
            }
        });
        TcpStream::connect(address).unwrap()
    }

    fn lockdown() -> Dictionary {
        dictionary("<dict><key>Type</key><string>com.apple.mobile.lockdown</string></dict>")
    }

    /// A lockdownd that answers `QueryType`, then `answers`.
    fn connected(answers: Vec<Dictionary>) -> Lockdown {
        let answers = std::iter::once(lockdown()).chain(answers).collect();
        Lockdown::new(Box::new(answering(answers))).unwrap()
    }

    /// A `trusting` that does nothing: the tests' users answer at once.
    fn nobody() {}

    fn record() -> PairRecord {
        PairRecord::generate(&device_public_key(), "BUID-1").unwrap()
    }

    #[test]
    fn says_when_the_device_isnt_running_ios() {
        let restoring =
            dictionary("<dict><key>Type</key><string>com.apple.mobile.restored</string></dict>");

        let error = Lockdown::new(Box::new(answering(vec![restoring])))
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "the device isn't running iOS: lockdownd doesn't answer"
        );
    }

    #[test]
    fn says_what_lockdownd_left_out() {
        let empty = || Dictionary::new();

        assert_eq!(
            connected(vec![empty()]).values().unwrap_err().to_string(),
            "lockdownd answered GetValue with no values"
        );
        assert_eq!(
            connected(vec![empty()])
                .start_service("com.apple.afc")
                .unwrap_err()
                .to_string(),
            "lockdownd started com.apple.afc without saying where"
        );
        let started = connected(vec![dictionary(
            "<dict><key>Port</key><integer>49152</integer><key>EnableServiceSSL</key><true/></dict>",
        )])
        .start_service("com.apple.afc")
        .unwrap();
        assert_eq!(started, (49152, true));
    }

    #[test]
    fn stops_waiting_for_a_user_who_doesnt_answer() {
        let pending = dictionary(
            "<dict><key>Error</key><string>PairingDialogResponsePending</string></dict>",
        );
        let mut lockdown = connected(vec![pending.clone(), pending]);
        let patience = Patience {
            poll: Duration::ZERO,
            limit: Duration::ZERO,
        };

        let error = lockdown.pair(&record(), &nobody, &patience).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the device's user didn't answer whether to trust this computer"
        );
    }

    #[test]
    fn says_when_lockdownd_hangs_up() {
        let patience = Patience {
            poll: Duration::ZERO,
            limit: Duration::ZERO,
        };
        let session = dictionary("<dict><key>EnableSessionSSL</key><true/></dict>");

        let values = connected(Vec::new()).values().unwrap_err();
        let pairing = connected(Vec::new())
            .pair(&record(), &nobody, &patience)
            .unwrap_err();
        let starting = connected(Vec::new()).start_session(&record()).unwrap_err();
        // It starts a session, then hangs up before its TLS handshake.
        let encrypting = connected(vec![session])
            .start_session(&record())
            .unwrap_err();

        assert!(format!("{values:#}").starts_with("lockdownd didn't answer GetValue: "));
        assert!(format!("{pairing:#}").starts_with("lockdownd didn't answer Pair: "));
        assert!(format!("{starting:#}").starts_with("lockdownd didn't answer StartSession: "));
        assert!(
            format!("{encrypting:#}")
                .starts_with("couldn't make an encrypted connection to the device: ")
        );
    }

    #[test]
    fn says_what_it_cant_encrypt_with_or_through() {
        let generated = record();
        let with = |key: &str, value: &[u8]| {
            let mut record = generated.clone();
            record.insert(key, plist::Value::Data(value.to_vec()));
            PairRecord::parse(&record.to_bytes()).unwrap()
        };
        // An X25519 key, which only agrees on keys, and signs nothing.
        let x25519 = b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VuBCIEICg0Lo3F1Ajs1PlWoLcuDfcR+nyg2NMEP+f0jaSvvO5D\n-----END PRIVATE KEY-----\n";
        let error = |record: &PairRecord| {
            format!(
                "{:#}",
                encrypt(Box::new(answering(Vec::new())), record)
                    .err()
                    .unwrap()
            )
        };

        assert_eq!(
            error(&with("HostCertificate", b"a certificate")),
            "the pair record's host certificate isn't one in PEM"
        );
        assert_eq!(
            error(&with("HostPrivateKey", b"a key")),
            "the pair record's host key isn't one in PEM"
        );
        assert_eq!(
            error(&with("HostPrivateKey", x25519)),
            "the pair record's host key isn't one that signs, as an RSA key does"
        );
        // Something that hangs up at once.
        assert!(
            error(&generated).starts_with("couldn't make an encrypted connection to the device: ")
        );
    }
}
