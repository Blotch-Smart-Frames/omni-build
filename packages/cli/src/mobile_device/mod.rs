//! Installing apps on iPhones and iPads with no Xcode, Finder or iTunes,
//! through the services those use, as libimobiledevice does: the usbmux
//! daemon reaches the device, lockdownd pairs this computer with it, once
//! the device's user trusts it, and starts its services in an encrypted
//! session, AFC stages the app on the device, and the installation proxy
//! installs it. These services aren't the developer ones that iOS 17 moved
//! behind a tunnel of its own, so no tunnel is needed.

mod afc;
mod installer;
mod lockdown;
mod pairing;
mod usbmux;

use std::fmt;
use std::io::{self, Cursor, Read, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

use afc::Afc;
use lockdown::Lockdown;
pub use lockdown::Patience;
use pairing::PairRecord;
pub use usbmux::{Address, Usbmux};

/// How long OmniBuild waits for each read and write on a connection to a
/// device, or to its usbmux daemon: long enough for the device to check a
/// large app, which it reports on as it goes.
pub const TIMEOUT: Duration = Duration::from_secs(120);
/// How long OmniBuild waits for the device's user to trust this computer,
/// and how often it asks the device whether they have.
pub const PATIENCE: Patience = Patience {
    poll: Duration::from_secs(1),
    limit: TIMEOUT,
};
/// The services OmniBuild uses.
const AFC: &str = "com.apple.afc";
const INSTALLER: &str = "com.apple.mobile.installation_proxy";
/// Where the app goes on the device for the installer to install it from:
/// the folder that AFC writes to and installers stage apps in.
const STAGING: &str = "PublicStaging";
const STAGED: &str = "PublicStaging/omni-build.ipa";
/// The most OmniBuild reads of a message from lockdownd or a service.
const MAX_MESSAGE: usize = 16 << 20;

/// A connection to a device, or to its usbmux daemon.
pub trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// An iPhone or iPad OmniBuild is in a session with.
pub struct Device {
    usbmux: Usbmux,
    /// Its entry in the daemon's list.
    entry: usbmux::Device,
    /// The name its user gave it, if it says.
    name: Option<String>,
    record: PairRecord,
    lockdown: Lockdown,
}

impl Device {
    /// Connects to `wanted`, a device the usbmux daemon reaches, by its
    /// UDID, or without one to the only device plugged in, or else to the
    /// only one it reaches over Wi-Fi. Pairs with it if this computer
    /// hasn't, or the device has forgotten it: calls `trusting` with the
    /// device's name when the device asks its user whether to trust this
    /// computer, and waits for the answer with `patience`.
    pub fn connect(
        usbmux: Usbmux,
        wanted: Option<&str>,
        trusting: &dyn Fn(&str),
        patience: &Patience,
    ) -> Result<Self> {
        let device = choose(&usbmux.devices()?, wanted)?;
        let udid = device.udid.clone();
        let (name, record, lockdown) = session(&usbmux, &device, trusting, patience)
            .with_context(|| format!("couldn't connect to {udid}"))?;
        Ok(Self {
            usbmux,
            entry: device,
            name,
            record,
            lockdown,
        })
    }

    /// Its UDID, which `--device` names it by.
    pub fn udid(&self) -> &str {
        &self.entry.udid
    }

    /// The name its user gave it, if it says.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Installs `ipa`, an app built for devices, replacing any with its ID:
    /// stages it on the device through AFC, and has the installation proxy
    /// install it from there.
    pub fn install(&mut self, ipa: &[u8]) -> Result<()> {
        let result = self.stage_and_install(ipa);
        result.with_context(|| format!("couldn't install the app on {self}"))
    }

    fn stage_and_install(&mut self, ipa: &[u8]) -> Result<()> {
        let mut afc = Afc::new(self.service(AFC)?);
        afc.make_directory(STAGING)?;
        afc.write_file(STAGED, ipa)?;
        let installed = self
            .service(INSTALLER)
            .and_then(|installer| installer::install(installer, STAGED));
        // Removed whether it installed or not, so it takes no room on the
        // device. The installer may have moved it already.
        let _ = afc.remove(STAGED);
        installed
    }

    /// A connection to the service `name`, which lockdownd starts.
    fn service(&mut self, name: &str) -> Result<Box<dyn Stream>> {
        let (port, encrypted) = self.lockdown.start_service(name)?;
        let stream = self.usbmux.connect(&self.entry, port)?;
        if encrypted {
            lockdown::encrypt(stream, &self.record)
        } else {
            Ok(stream)
        }
    }
}

impl fmt::Display for Device {
    /// Its name, then its UDID: "Jane's iPhone (00008030-001A2B3C4D5E6F7A)".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name} ({})", self.entry.udid),
            None => write!(f, "{}", self.entry.udid),
        }
    }
}

/// The device to connect to, of `devices`: `wanted`, by its UDID, plugged
/// in if it's both plugged in and on Wi-Fi, or the only one plugged in, or
/// else the only one on Wi-Fi.
fn choose(devices: &[usbmux::Device], wanted: Option<&str>) -> Result<usbmux::Device> {
    // UDIDs as the daemon gives them, with their dash or without.
    let udid = |udid: &str| udid.replace('-', "").to_ascii_uppercase();
    if let Some(wanted) = wanted {
        return devices
            .iter()
            .filter(|device| udid(&device.udid) == udid(wanted))
            .max_by_key(|device| device.usb)
            .cloned()
            .with_context(|| format!("no iPhone or iPad {wanted} is connected to this computer"));
    }
    let plugged_in: Vec<_> = devices.iter().filter(|device| device.usb).collect();
    let candidates = if plugged_in.is_empty() {
        devices.iter().collect()
    } else {
        plugged_in
    };
    match candidates.as_slice() {
        [] => bail!("no iPhone or iPad is connected to this computer: plug one in, and unlock it"),
        [device] => Ok((*device).clone()),
        several => bail!(
            "several iPhones and iPads are connected to this computer: say which with --device, by its UDID: {}",
            several
                .iter()
                .map(|device| device.udid.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A session with `device`, with the pair record the usbmux daemon keeps
/// for it, or a new one, with which this computer pairs with it. Returns
/// the device's name, the record and the session.
fn session(
    usbmux: &Usbmux,
    device: &usbmux::Device,
    trusting: &dyn Fn(&str),
    patience: &Patience,
) -> Result<(Option<String>, PairRecord, Lockdown)> {
    let mut lockdown = Lockdown::new(usbmux.connect(device, lockdown::PORT)?)?;
    let values = lockdown.values()?;
    let name = values
        .get("DeviceName")
        .and_then(Value::as_string)
        .map(str::to_owned);
    let saved = usbmux
        .pair_record(&device.udid)?
        .map(|record| PairRecord::parse(&record))
        .transpose()?;
    if let Some(record) = saved
        && lockdown.start_session(&record)?
    {
        return Ok((name, record, lockdown));
    }
    let key = values
        .get("DevicePublicKey")
        .and_then(Value::as_data)
        .context("the device didn't give its public key, which pairing needs")?;
    let mut record = PairRecord::generate(key, &usbmux.buid()?)?;
    let shown = match &name {
        Some(name) => format!("{name} ({})", device.udid),
        None => device.udid.clone(),
    };
    if let Some(bag) = lockdown.pair(&record, &|| trusting(&shown), patience)? {
        record.insert("EscrowBag", Value::Data(bag));
    }
    // The address it's known by on Wi-Fi, where the system's tools find
    // it with the record, as libimobiledevice keeps it.
    if let Some(address) = values.get("WiFiAddress").and_then(Value::as_string) {
        record.insert("WiFiMACAddress", address.into());
    }
    usbmux.save_pair_record(device, &record.to_bytes())?;
    if !lockdown.start_session(&record)? {
        bail!("the device doesn't know the pair record it has just taken");
    }
    Ok((name, record, lockdown))
}

/// Sends `message` as lockdownd and the services it starts take one: its
/// length, four bytes big-endian, then the property list, as XML.
fn send(stream: &mut impl Write, message: &Dictionary) -> io::Result<()> {
    let mut body = Vec::new();
    Value::Dictionary(message.clone())
        .to_writer_xml(&mut body)
        .expect("a dictionary writes to memory");
    let length = u32::try_from(body.len()).expect("a request fits its length");
    let mut framed = length.to_be_bytes().to_vec();
    framed.extend(body);
    stream.write_all(&framed)
}

/// A message from lockdownd or a service: a dictionary, as XML or binary.
fn receive(stream: &mut impl Read) -> Result<Dictionary> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length)).unwrap_or(usize::MAX);
    if length > MAX_MESSAGE {
        bail!("the device sent a message larger than any it sends");
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Value::from_reader(Cursor::new(body))
        .ok()
        .and_then(Value::into_dictionary)
        .context("the device sent what isn't a property list")
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;
    use std::io::{Cursor, Read, Write};
    use std::net::TcpStream;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use plist::{Dictionary, Value};
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

    use super::afc::tests::{Files, serve_afc};
    use super::installer::tests::serve_installer;
    use super::pairing::tests::{device_key, device_public_key};
    use super::usbmux::tests::{daemon, dictionary, write_message};
    use super::usbmux::{self, Address, Usbmux};
    use super::{Device, Patience, Stream, choose, receive, send};

    pub const UDID: &str = "00008030-001A2B3C4D5E6F7A";
    /// The ports the made-up iPhone's AFC and installation proxy listen on.
    const AFC_PORT: u16 = 49_152;
    const INSTALLER_PORT: u16 = 49_153;
    /// No waiting, for tests.
    pub const IMPATIENT: Patience = Patience {
        poll: Duration::ZERO,
        limit: Duration::from_secs(5),
    };

    /// A connection that fails every read and write, as one does once the
    /// device is unplugged.
    pub struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A message lockdownd or a service read.
    pub fn read_plist(stream: &mut impl Read) -> Dictionary {
        receive(stream).unwrap()
    }

    pub fn write_plist(stream: &mut impl Write, message: &Dictionary) {
        send(stream, message).unwrap();
    }

    /// The certificates of a pair record a phone knows: its root and the
    /// phone's own.
    type Certificates = (Vec<u8>, Vec<u8>);

    /// What a made-up phone does that others don't.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Quirk {
        /// Its sessions are left unencrypted.
        Plain,
        /// It gives no name, Wi-Fi address or escrow bag.
        Reticent,
        /// The public key it gives is one pairing can't use.
        OddKey,
        /// It gives no public key.
        Keyless,
        /// Its daemon refuses connections to its services.
        ServicesClosed,
        /// Its services encrypt their connections.
        ServicesEncrypted,
    }

    /// A made-up iPhone, and what it does.
    #[derive(Default)]
    pub struct Phone {
        /// The pair records it knows: each one's certificates, by its
        /// `HostID`.
        pub trusted: BTreeMap<String, Certificates>,
        /// The pair record its usbmux daemon keeps, if any.
        pub saved: Option<Vec<u8>>,
        /// How many times it says it's waiting for its user to answer
        /// whether to trust this computer, before they trust it.
        pub undecided: u32,
        /// What lockdownd answers a request with instead, by its name.
        pub errors: BTreeMap<&'static str, &'static str>,
        pub quirks: Vec<Quirk>,
        /// The requests its usbmux daemon hangs up on, by kind.
        pub hung_up: Vec<&'static str>,
        /// The service lockdownd doesn't start.
        pub missing_service: Option<&'static str>,
        /// What AFC holds, and the operation it refuses, if any.
        pub files: Files,
        pub afc_refuses: Option<u64>,
        /// What the installation proxy answers with.
        pub installed: Vec<Dictionary>,
        /// The requests lockdownd and the installer read.
        pub requests: Vec<Dictionary>,
    }

    impl Phone {
        fn has(&self, quirk: Quirk) -> bool {
            self.quirks.contains(&quirk)
        }
    }

    pub type Shared = Arc<Mutex<Phone>>;

    /// A phone that installs what it's given.
    pub fn phone() -> Shared {
        let phone: Shared = Arc::default();
        phone.lock().unwrap().installed = vec![complete()];
        phone
    }

    fn complete() -> Dictionary {
        dictionary("<dict><key>Status</key><string>Complete</string></dict>")
    }

    fn result(number: u32) -> Dictionary {
        dictionary(&format!(
            "<dict><key>MessageType</key><string>Result</string><key>Number</key><integer>{number}</integer></dict>"
        ))
    }

    /// The made-up iPhone, `UDID`, plugged in, behind a made-up usbmux
    /// daemon. Returns the daemon's address.
    pub fn plugged_in(phone: &Shared) -> Address {
        let phone = Arc::clone(phone);
        daemon(move |request, mut stream| {
            let kind = request["MessageType"].as_string().unwrap().to_owned();
            let mut phone_now = phone.lock().unwrap();
            if phone_now.hung_up.contains(&kind.as_str()) {
                return None;
            }
            match kind.as_str() {
                "ListDevices" => Some(dictionary(&format!(
                    "<dict><key>DeviceList</key><array><dict><key>DeviceID</key><integer>3</integer><key>Properties</key><dict><key>ConnectionType</key><string>USB</string><key>SerialNumber</key><string>{UDID}</string></dict></dict></array></dict>"
                ))),
                "ReadBUID" => Some(dictionary(
                    "<dict><key>BUID</key><string>BUID-1</string></dict>",
                )),
                "ReadPairRecord" => Some(match &phone_now.saved {
                    Some(record) => {
                        let mut answer = Dictionary::new();
                        answer.insert("PairRecordData".to_owned(), Value::Data(record.clone()));
                        answer
                    }
                    None => result(2),
                }),
                "SavePairRecord" => {
                    phone_now.saved = request["PairRecordData"].as_data().map(<[u8]>::to_vec);
                    Some(result(0))
                }
                _ => {
                    let port = u16::from_be_bytes(
                        u16::try_from(request["PortNumber"].as_unsigned_integer().unwrap())
                            .unwrap()
                            .to_ne_bytes(),
                    );
                    if port != 62078 && phone_now.has(Quirk::ServicesClosed) {
                        return Some(result(3));
                    }
                    drop(phone_now);
                    write_message(&mut stream, &result(0));
                    let phone = Arc::clone(&phone);
                    std::thread::spawn(move || serve(&phone, port, stream));
                    None
                }
            }
        })
    }

    /// Answers on `stream`, a connection to `port` on the phone, as the
    /// service there.
    fn serve(phone: &Shared, port: u16, stream: TcpStream) {
        if port == 62078 {
            return serve_lockdown(phone, Box::new(stream));
        }
        let mut stream: Box<dyn Stream> = Box::new(stream);
        if phone.lock().unwrap().has(Quirk::ServicesEncrypted) {
            let (root, device) = phone
                .lock()
                .unwrap()
                .trusted
                .values()
                .next()
                .unwrap()
                .clone();
            stream = Box::new(accept(stream, &root, &device));
        }
        if port == AFC_PORT {
            let (files, refused) = {
                let phone = phone.lock().unwrap();
                (Arc::clone(&phone.files), phone.afc_refuses)
            };
            serve_afc(stream, &files, refused);
        } else {
            let answers = phone.lock().unwrap().installed.clone();
            serve_installer(stream, &answers, &mut |request| {
                phone.lock().unwrap().requests.push(request);
            });
        }
    }

    /// Answers lockdownd's requests on `stream`, as the phone's, until the
    /// stream ends.
    fn serve_lockdown(phone: &Shared, mut stream: Box<dyn Stream>) {
        while let Ok(request) = receive(&mut stream) {
            let (answer, encrypt) = answer_lockdown(&mut phone.lock().unwrap(), &request);
            write_plist(&mut stream, &answer);
            if let Some((root, device)) = encrypt {
                stream = Box::new(accept(stream, &root, &device));
            }
        }
    }

    /// lockdownd's answer to `request`, and the certificates of the session
    /// it starts encrypting, if it does.
    fn answer_lockdown(
        phone: &mut Phone,
        request: &Dictionary,
    ) -> (Dictionary, Option<Certificates>) {
        phone.requests.push(request.clone());
        let name = request["Request"].as_string().unwrap();
        let mut answer = Dictionary::new();
        answer.insert("Request".to_owned(), name.into());
        let mut set = |key: &str, value: Value| {
            answer.insert(key.to_owned(), value);
        };
        if let Some(error) = phone.errors.get(name) {
            set("Error", (*error).into());
            return (answer, None);
        }
        let mut encrypt = None;
        match name {
            "QueryType" => set("Type", "com.apple.mobile.lockdown".into()),
            "GetValue" => {
                let key = if phone.has(Quirk::OddKey) {
                    b"a key".to_vec()
                } else {
                    device_public_key()
                };
                let mut values = Dictionary::new();
                if !phone.has(Quirk::Keyless) {
                    values.insert("DevicePublicKey".to_owned(), Value::Data(key));
                }
                values.insert("UniqueDeviceID".to_owned(), UDID.into());
                if !phone.has(Quirk::Reticent) {
                    values.insert("DeviceName".to_owned(), "Jane's iPhone".into());
                    values.insert("WiFiAddress".to_owned(), "a0:b1:c2:d3:e4:f5".into());
                }
                set("Value", values.into());
            }
            "Pair" if phone.undecided > 0 => {
                phone.undecided -= 1;
                set("Error", "PairingDialogResponsePending".into());
            }
            "Pair" => {
                let record = request["PairRecord"].as_dictionary().unwrap();
                let data = |key: &str| record[key].as_data().unwrap().to_vec();
                // The private keys stay on the computer.
                assert!(!record.contains_key("HostPrivateKey"));
                assert!(!record.contains_key("RootPrivateKey"));
                assert_eq!(request["ProtocolVersion"], "2".into());
                phone.trusted.insert(
                    record["HostID"].as_string().unwrap().to_owned(),
                    (data("RootCertificate"), data("DeviceCertificate")),
                );
                if !phone.has(Quirk::Reticent) {
                    set("EscrowBag", Value::Data(b"bag".to_vec()));
                }
            }
            "StartSession" => match phone.trusted.get(request["HostID"].as_string().unwrap()) {
                Some(certificates) => {
                    set("SessionID", "S1".into());
                    set("EnableSessionSSL", (!phone.has(Quirk::Plain)).into());
                    if !phone.has(Quirk::Plain) {
                        encrypt = Some(certificates.clone());
                    }
                }
                None => set("Error", "InvalidHostID".into()),
            },
            _ => {
                let service = request["Service"].as_string().unwrap();
                if phone.missing_service == Some(service) {
                    set("Error", "InvalidService".into());
                } else {
                    let port = if service == "com.apple.afc" {
                        AFC_PORT
                    } else {
                        INSTALLER_PORT
                    };
                    set("Port", u64::from(port).into());
                    set(
                        "EnableServiceSSL",
                        phone.has(Quirk::ServicesEncrypted).into(),
                    );
                }
            }
        }
        (answer, encrypt)
    }

    /// Takes the TLS connection the computer makes over `stream`, as
    /// lockdownd does: with the device certificate pairing made for its
    /// key, and only from a computer whose certificate `root`, the root the
    /// computer paired with, issued. TLS 1.2, which older iOS speaks.
    pub fn accept(
        stream: Box<dyn Stream>,
        root: &[u8],
        certificate: &[u8],
    ) -> StreamOwned<ServerConnection, Box<dyn Stream>> {
        let provider = Arc::new(rustls_rustcrypto::provider());
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from_pem_slice(root).unwrap())
            .unwrap();
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
                .unwrap();
        let key = device_key().to_pkcs8_pem(LineEnding::LF).unwrap();
        let config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(certificate).unwrap()],
                PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
            )
            .unwrap();
        StreamOwned::new(ServerConnection::new(Arc::new(config)).unwrap(), stream)
    }

    pub fn usbmux(address: Address) -> Usbmux {
        Usbmux::new(address, Duration::from_secs(10))
    }

    /// A `trusting` that records each name it's called with in `asked`.
    fn recording(asked: &Mutex<Vec<String>>) -> impl Fn(&str) + '_ {
        move |name| asked.lock().unwrap().push(name.to_owned())
    }

    /// Connects to `phone`. Returns the device, and the names its user was
    /// asked whether to trust this computer by.
    fn connect(phone: &Shared) -> (anyhow::Result<Device>, Vec<String>) {
        let asked = Mutex::new(Vec::new());
        let device = Device::connect(
            usbmux(plugged_in(phone)),
            None,
            &recording(&asked),
            &IMPATIENT,
        );
        (device, asked.into_inner().unwrap())
    }

    /// Connects to `phone` and installs an app, and returns why that
    /// failed.
    fn failure(phone: &Shared) -> String {
        let error = connect(phone)
            .0
            .and_then(|mut device| device.install(b"an app"));
        format!("{:#}", error.unwrap_err())
    }

    #[test]
    fn pairs_with_an_iphone_its_user_trusts_and_installs_an_app() {
        let phone = phone();
        phone.lock().unwrap().undecided = 2;

        let (device, asked) = connect(&phone);
        let mut device = device.unwrap();
        device.install(b"an app").unwrap();

        assert_eq!(device.udid(), UDID);
        assert_eq!(device.name(), Some("Jane's iPhone"));
        assert_eq!(device.to_string(), format!("Jane's iPhone ({UDID})"));
        // Asked once, however long its user took.
        assert_eq!(asked, [format!("Jane's iPhone ({UDID})")]);
        let phone = phone.lock().unwrap();
        // The record it saved is the one the device trusts, with what the
        // device gave.
        let saved = super::PairRecord::parse(phone.saved.as_ref().unwrap()).unwrap();
        assert!(phone.trusted.contains_key(&saved.host_id));
        assert_eq!(saved.system_buid, "BUID-1");
        assert_eq!(saved.public()["EscrowBag"], Value::Data(b"bag".to_vec()));
        assert_eq!(saved.public()["WiFiMACAddress"], "a0:b1:c2:d3:e4:f5".into());
        // The app was staged, then installed from there, and removed.
        let files = phone.files.lock().unwrap();
        assert!(files.contains_key("PublicStaging"));
        assert!(!files.contains_key("PublicStaging/omni-build.ipa"));
        let install = phone.requests.last().unwrap();
        assert_eq!(
            install["PackagePath"],
            "PublicStaging/omni-build.ipa".into()
        );
        let requests: Vec<_> = phone
            .requests
            .iter()
            .filter_map(|request| request.get("Request").and_then(Value::as_string))
            .collect();
        assert_eq!(
            requests,
            [
                "QueryType",
                "GetValue",
                "Pair",
                "Pair",
                "Pair",
                "StartSession",
                "StartService",
                "StartService"
            ]
        );
    }

    #[test]
    fn uses_the_pair_record_it_has_and_pairs_again_once_the_device_forgets_it() {
        let phone = phone();
        let record = connect(&phone).0.unwrap().record;
        let trusted = |phone: &Shared| phone.lock().unwrap().trusted.len();

        let (again, asked) = connect(&phone);
        assert_eq!(again.unwrap().record, record);
        assert_eq!(asked, Vec::<String>::new());
        assert_eq!(trusted(&phone), 1);
        // Its user resets its trust settings.
        phone.lock().unwrap().trusted.clear();
        let repaired = connect(&phone).0.unwrap();

        assert_ne!(repaired.record.host_id, record.host_id);
        assert_eq!(trusted(&phone), 1);
    }

    #[test]
    fn talks_in_the_clear_or_encrypted_as_the_device_does() {
        for quirk in [Quirk::Plain, Quirk::ServicesEncrypted] {
            let phone = phone();
            phone.lock().unwrap().quirks = vec![quirk, Quirk::Reticent];

            let mut device = connect(&phone).0.unwrap();
            device.install(b"an app").unwrap();

            assert_eq!(device.to_string(), UDID);
            let phone = phone.lock().unwrap();
            let saved = super::PairRecord::parse(phone.saved.as_ref().unwrap()).unwrap();
            assert!(!saved.public().contains_key("WiFiMACAddress"));
            assert!(!saved.public().contains_key("EscrowBag"));
        }
    }

    #[test]
    fn says_why_it_couldnt_connect() {
        let failing = |change: &dyn Fn(&mut Phone)| {
            let phone = phone();
            change(&mut phone.lock().unwrap());
            failure(&phone)
        };
        let couldnt = |reason: &str| format!("couldn't connect to {UDID}: {reason}");

        assert_eq!(
            failing(&|phone| {
                phone.errors.insert("Pair", "PasswordProtected");
            }),
            couldnt("the device is locked: unlock it, and try again")
        );
        assert_eq!(
            failing(&|phone| {
                phone.errors.insert("Pair", "UserDeniedPairing");
            }),
            couldnt("the device's user didn't trust this computer")
        );
        assert_eq!(
            failing(&|phone| {
                phone.errors.insert("StartSession", "InvalidHostID");
            }),
            couldnt("the device doesn't know the pair record it has just taken")
        );
        assert_eq!(
            failing(&|phone| {
                phone.errors.insert("StartSession", "SessionActive");
            }),
            couldnt("lockdownd refused StartSession: SessionActive")
        );
        assert_eq!(
            failing(&|phone| {
                phone.errors.insert("QueryType", "InvalidConnection");
            }),
            couldnt("lockdownd refused QueryType: InvalidConnection")
        );
        assert_eq!(
            failing(&|phone| {
                phone.errors.insert("GetValue", "GetProhibited");
            }),
            couldnt("lockdownd refused GetValue: GetProhibited")
        );
        assert_eq!(
            failing(&|phone| phone.quirks.push(Quirk::OddKey)),
            couldnt("the device's public key isn't an RSA key in PEM")
        );
        assert_eq!(
            failing(&|phone| phone.quirks.push(Quirk::Keyless)),
            couldnt("the device didn't give its public key, which pairing needs")
        );
        assert_eq!(
            failing(&|phone| phone.saved = Some(b"a record".to_vec())),
            couldnt("the pair record isn't a property list")
        );
        for (kind, reason) in [
            ("Connect", "the usbmux daemon didn't answer Connect: "),
            (
                "ReadPairRecord",
                "the usbmux daemon didn't answer ReadPairRecord: ",
            ),
            ("ReadBUID", "the usbmux daemon didn't answer ReadBUID: "),
            (
                "SavePairRecord",
                "the usbmux daemon didn't answer SavePairRecord: ",
            ),
        ] {
            let error = failing(&|phone| phone.hung_up.push(kind));
            assert!(error.starts_with(&couldnt(reason)), "{error}");
        }
    }

    #[test]
    fn says_when_a_session_with_a_known_device_fails() {
        let phone = phone();
        connect(&phone).0.unwrap();
        phone
            .lock()
            .unwrap()
            .errors
            .insert("StartSession", "SessionActive");

        assert_eq!(
            failure(&phone),
            format!("couldn't connect to {UDID}: lockdownd refused StartSession: SessionActive")
        );
    }

    #[test]
    fn says_why_it_couldnt_install() {
        let failing = |change: &dyn Fn(&mut Phone)| {
            let phone = phone();
            change(&mut phone.lock().unwrap());
            let error = failure(&phone);
            // The staged app doesn't stay on the device.
            let files = Arc::clone(&phone.lock().unwrap().files);
            assert!(
                !files
                    .lock()
                    .unwrap()
                    .contains_key("PublicStaging/omni-build.ipa")
            );
            error
        };
        let couldnt =
            |reason: &str| format!("couldn't install the app on Jane's iPhone ({UDID}): {reason}");

        assert_eq!(
            failing(&|phone| {
                phone.installed = vec![dictionary(
                    "<dict><key>Error</key><string>ApplicationVerificationFailed</string></dict>",
                )];
            }),
            couldnt("the device refused the app (ApplicationVerificationFailed)")
        );
        assert_eq!(
            failing(&|phone| phone.missing_service = Some("com.apple.afc")),
            couldnt("lockdownd refused StartService: InvalidService")
        );
        assert_eq!(
            failing(&|phone| phone.missing_service = Some("com.apple.mobile.installation_proxy")),
            couldnt("lockdownd refused StartService: InvalidService")
        );
        assert_eq!(
            failing(&|phone| phone.quirks.push(Quirk::ServicesClosed)),
            couldnt(
                "couldn't connect to port 49152 on the device: the device refused the connection"
            )
        );
        assert_eq!(
            failing(&|phone| phone.afc_refuses = Some(0x09)),
            couldnt(
                "couldn't make the folder PublicStaging on the device: permission was denied (AFC status 10)"
            )
        );
        assert_eq!(
            failing(&|phone| phone.afc_refuses = Some(0x0d)),
            couldnt(
                "couldn't write PublicStaging/omni-build.ipa on the device: permission was denied (AFC status 10)"
            )
        );
    }

    #[test]
    fn says_when_it_cant_reach_the_usbmux_daemon() {
        // A port nothing listens on any more.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let address = Address::Tcp(([127, 0, 0, 1], port).into());

        let asked = Mutex::new(Vec::new());
        let error = Device::connect(usbmux(address), None, &recording(&asked), &IMPATIENT)
            .err()
            .unwrap();
        let unknown = Device::connect(
            usbmux(plugged_in(&phone())),
            Some("gone"),
            &recording(&asked),
            &IMPATIENT,
        )
        .err()
        .unwrap();

        assert!(format!("{error:#}").starts_with("couldn't reach the usbmux daemon at "));
        assert_eq!(
            unknown.to_string(),
            "no iPhone or iPad gone is connected to this computer"
        );
    }

    fn device(id: u64, udid: &str, usb: bool) -> usbmux::Device {
        usbmux::Device {
            id,
            udid: udid.to_owned(),
            usb,
        }
    }

    #[test]
    fn chooses_the_device_plugged_in_or_the_one_it_was_asked_for() {
        let phone = device(1, UDID, true);
        let phone_on_wifi = device(2, UDID, false);
        let ipad_on_wifi = device(3, "00008101-000A1B2C3D4E5F60", false);
        let all = [phone_on_wifi.clone(), phone.clone(), ipad_on_wifi.clone()];

        assert_eq!(choose(&all, None).unwrap(), phone);
        assert_eq!(
            choose(&all, Some("00008030001a2b3c4d5e6f7a")).unwrap(),
            phone
        );
        assert_eq!(
            choose(&all, Some("00008101-000A1B2C3D4E5F60")).unwrap(),
            ipad_on_wifi
        );
        assert_eq!(
            choose(std::slice::from_ref(&ipad_on_wifi), None).unwrap(),
            ipad_on_wifi
        );
        assert_eq!(
            choose(&[], None).unwrap_err().to_string(),
            "no iPhone or iPad is connected to this computer: plug one in, and unlock it"
        );
        assert_eq!(
            choose(&[phone_on_wifi, ipad_on_wifi], None)
                .unwrap_err()
                .to_string(),
            format!(
                "several iPhones and iPads are connected to this computer: say which with --device, by its UDID: {UDID}, 00008101-000A1B2C3D4E5F60"
            )
        );
    }

    #[test]
    fn reads_and_writes_messages_as_lockdownd_does() {
        let mut message = Dictionary::new();
        message.insert("Request".to_owned(), "QueryType".into());
        let mut written = Vec::new();
        // Each read from the same kind of reader, which llvm-cov needs to
        // count one copy of a generic function as covered.
        let read = |bytes: &[u8]| receive(&mut Cursor::new(bytes.to_vec()));

        send(&mut written, &message).unwrap();

        assert_eq!(
            &written[..4],
            &(u32::try_from(written.len()).unwrap() - 4).to_be_bytes()
        );
        assert_eq!(read(&written).unwrap(), message);
        // A binary property list, as services send some.
        let mut binary = Vec::new();
        Value::Dictionary(message.clone())
            .to_writer_binary(&mut binary)
            .unwrap();
        let mut framed = u32::try_from(binary.len()).unwrap().to_be_bytes().to_vec();
        framed.extend(binary);
        assert_eq!(read(&framed).unwrap(), message);
        assert_eq!(
            read(&u32::MAX.to_be_bytes()).unwrap_err().to_string(),
            "the device sent a message larger than any it sends"
        );
        assert_eq!(
            read(b"\0\0\0\x02hi").unwrap_err().to_string(),
            "the device sent what isn't a property list"
        );
        assert!(read(b"\0\0").is_err());
        // A length that promises more than comes.
        assert!(read(b"\0\0\0\x05hi").is_err());
        assert!(receive(&mut Broken).is_err());
    }
}
