//! The usbmux daemon, through which OmniBuild reaches iPhones and iPads: it
//! holds the USB connection to each one plugged in, and to those that sync
//! over Wi-Fi, carries TCP-like connections to the services on them, and
//! keeps the pair records a computer proves itself to each with. Every
//! system with the iPhone's driver runs one: macOS Apple's, at
//! `/var/run/usbmuxd`, Windows Apple's Mobile Device Service, which Apple
//! Devices and iTunes install with the driver, on TCP port 27015, and Linux
//! libimobiledevice's `usbmuxd`. OmniBuild speaks the daemon's protocol, as
//! libimobiledevice does, in property lists, rather than take the USB device
//! from it, which on macOS only Apple's daemon may have.

use std::fmt;
use std::io::{self, Cursor, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

use super::Stream;

/// The version of the daemon's protocol that carries property lists, and
/// the kind of message they're all sent as.
const VERSION: u32 = 1;
const PLIST: u32 = 8;
/// A message's header: its length, the version, its kind and its tag, each
/// four bytes, little-endian.
const HEADER: usize = 16;
/// The most OmniBuild reads of a message from the daemon: a list of
/// devices, or a pair record, is a few kilobytes.
const MAX_MESSAGE: usize = 1 << 20;
/// What `Connect` asks for.
const CONNECT: &str = "Connect";

/// Where the daemon listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
    /// A Unix socket's path, as on macOS and Linux.
    Unix(PathBuf),
    /// An address and port, as on Windows.
    Tcp(SocketAddr),
}

impl Address {
    /// Where the daemon is: `configured`, as libusbmuxd reads
    /// `USBMUXD_SOCKET_ADDRESS`, an IP address and port, or a socket's path,
    /// after `UNIX:` or not. Without it, where the system's daemon is.
    pub fn new(configured: Option<&str>) -> Self {
        match configured {
            Some(configured) => configured.parse().map_or_else(
                |_| Self::Unix(configured.trim_start_matches("UNIX:").into()),
                Self::Tcp,
            ),
            None => Self::system(),
        }
    }

    /// Where the system's daemon is.
    #[cfg(unix)]
    fn system() -> Self {
        Self::Unix("/var/run/usbmuxd".into())
    }

    /// Where the system's daemon is.
    #[cfg(not(unix))]
    fn system() -> Self {
        Self::Tcp(([127, 0, 0, 1], 27015).into())
    }

    /// A connection to the daemon, which waits up to `timeout` for each
    /// read and write.
    fn open(&self, timeout: Duration) -> io::Result<Box<dyn Stream>> {
        match self {
            Self::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, timeout)?;
                stream
                    .set_read_timeout(Some(timeout))
                    .and_then(|()| stream.set_write_timeout(Some(timeout)))
                    // Small messages, each answered before the next.
                    .and_then(|()| stream.set_nodelay(true))
                    .map(|()| Box::new(stream) as Box<dyn Stream>)
            }
            #[cfg(unix)]
            Self::Unix(path) => {
                let stream = std::os::unix::net::UnixStream::connect(path)?;
                stream
                    .set_read_timeout(Some(timeout))
                    .and_then(|()| stream.set_write_timeout(Some(timeout)))
                    .map(|()| Box::new(stream) as Box<dyn Stream>)
            }
            #[cfg(not(unix))]
            Self::Unix(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this system has no Unix sockets",
            )),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "{}", path.display()),
            Self::Tcp(address) => write!(f, "{address}"),
        }
    }
}

/// A device the daemon has a connection to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The daemon's number for its connection to the device.
    pub id: u64,
    /// The device's UDID, which the daemon calls its serial number.
    pub udid: String,
    /// Whether it's plugged in by USB, rather than reached over Wi-Fi.
    pub usb: bool,
}

/// The usbmux daemon, at `address`.
pub struct Usbmux {
    address: Address,
    /// How long it waits for each read and write: of the daemon's answers,
    /// and on a connection to a device, of the device's.
    timeout: Duration,
}

impl Usbmux {
    pub fn new(address: Address, timeout: Duration) -> Self {
        Self { address, timeout }
    }

    /// The devices the daemon has a connection to.
    pub fn devices(&self) -> Result<Vec<Device>> {
        let (_, answer) = self.ask("ListDevices", Dictionary::new())?;
        let devices = answer
            .get("DeviceList")
            .and_then(Value::as_array)
            .context("the usbmux daemon answered with no list of devices")?;
        Ok(devices.iter().filter_map(device).collect())
    }

    /// The daemon's ID for this computer, which a pair record names it by.
    pub fn buid(&self) -> Result<String> {
        let (_, answer) = self.ask("ReadBUID", Dictionary::new())?;
        answer
            .get("BUID")
            .and_then(Value::as_string)
            .map(str::to_owned)
            .context("the usbmux daemon answered with no ID for this computer")
    }

    /// The pair record the daemon keeps for the device `udid`, as a
    /// property list, if this computer has paired with it.
    pub fn pair_record(&self, udid: &str) -> Result<Option<Vec<u8>>> {
        let (_, answer) = self.ask("ReadPairRecord", fields([("PairRecordID", udid.into())]))?;
        Ok(answer
            .get("PairRecordData")
            .and_then(Value::as_data)
            .map(<[u8]>::to_vec))
    }

    /// Has the daemon keep `record`, a property list, as the pair record
    /// for `device`, where the system and its other tools find it too.
    pub fn save_pair_record(&self, device: &Device, record: &[u8]) -> Result<()> {
        let (_, answer) = self.ask(
            "SavePairRecord",
            fields([
                ("PairRecordID", device.udid.as_str().into()),
                ("PairRecordData", Value::Data(record.to_vec())),
                ("DeviceID", device.id.into()),
            ]),
        )?;
        result(&answer).context("the usbmux daemon didn't keep the pair record")
    }

    /// A connection to the service listening on `port` on `device`.
    pub fn connect(&self, device: &Device, port: u16) -> Result<Box<dyn Stream>> {
        // In network order, as libusbmuxd sends it: the daemon puts the
        // number in the packets it sends the device as it is.
        let port_number = u16::from_ne_bytes(port.to_be_bytes());
        let (stream, answer) = self.ask(
            CONNECT,
            fields([
                ("DeviceID", device.id.into()),
                ("PortNumber", port_number.into()),
            ]),
        )?;
        result(&answer)
            .with_context(|| format!("couldn't connect to port {port} on the device"))?;
        Ok(stream)
    }

    /// Sends the request `kind`, with `fields`, on a connection of its own,
    /// as the daemon takes one request on each, and returns the connection
    /// and the daemon's answer. After a `Connect`, the connection is to the
    /// device.
    fn ask(&self, kind: &str, fields: Dictionary) -> Result<(Box<dyn Stream>, Dictionary)> {
        let mut stream = self.address.open(self.timeout).with_context(|| {
            format!(
                "couldn't reach the usbmux daemon at {}, which talks to iPhones and iPads: macOS runs it, Windows with Apple Devices or iTunes installed, and Linux with the usbmuxd package",
                self.address
            )
        })?;
        let mut message = Dictionary::new();
        message.insert("MessageType".to_owned(), kind.into());
        message.insert("ProgName".to_owned(), "omni".into());
        message.insert(
            "ClientVersionString".to_owned(),
            concat!("omni ", env!("CARGO_PKG_VERSION")).into(),
        );
        message.insert("kLibUSBMuxVersion".to_owned(), 3.into());
        message.extend(fields);
        let answer = send(&mut stream, &message)
            .map_err(anyhow::Error::from)
            .and_then(|()| receive(&mut stream))
            .with_context(|| format!("the usbmux daemon didn't answer {kind}"))?;
        Ok((stream, answer))
    }
}

/// A dictionary of `fields`.
fn fields<const N: usize>(fields: [(&str, Value); N]) -> Dictionary {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

/// A device in the daemon's list, as it describes one.
fn device(entry: &Value) -> Option<Device> {
    let entry = entry.as_dictionary()?;
    let properties = entry.get("Properties")?.as_dictionary()?;
    Some(Device {
        id: entry.get("DeviceID")?.as_unsigned_integer()?,
        udid: properties.get("SerialNumber")?.as_string()?.to_owned(),
        usb: properties.get("ConnectionType").and_then(Value::as_string) == Some("USB"),
    })
}

/// Whether the daemon's `answer` says it did what it was asked.
fn result(answer: &Dictionary) -> Result<()> {
    match answer.get("Number").and_then(Value::as_unsigned_integer) {
        Some(0) => Ok(()),
        // The daemon's numbers for what went wrong, as libusbmuxd has them.
        Some(2) => bail!("the device isn't connected any more"),
        Some(3) => bail!("the device refused the connection"),
        Some(number) => bail!("the usbmux daemon refused it ({number})"),
        None => bail!("the usbmux daemon answered with no result"),
    }
}

/// Sends `message` to the daemon, with the tag 1: OmniBuild sends one
/// message on each connection.
fn send(stream: &mut impl Write, message: &Dictionary) -> io::Result<()> {
    let mut body = Vec::new();
    Value::Dictionary(message.clone())
        .to_writer_xml(&mut body)
        .expect("a dictionary writes to memory");
    let length = u32::try_from(HEADER + body.len()).expect("a request fits the daemon's header");
    let mut packet = Vec::with_capacity(HEADER + body.len());
    for field in [length, VERSION, PLIST, 1] {
        packet.extend(field.to_le_bytes());
    }
    packet.extend(body);
    stream.write_all(&packet)
}

/// The daemon's answer, a dictionary.
fn receive(stream: &mut impl Read) -> Result<Dictionary> {
    let mut header = [0; HEADER];
    stream.read_exact(&mut header)?;
    let field = |at: usize| {
        u32::from_le_bytes(
            header[at..at + 4]
                .try_into()
                .expect("a field is four bytes"),
        )
    };
    let length = usize::try_from(field(0)).unwrap_or(usize::MAX);
    if field(4) != VERSION || field(8) != PLIST || !(HEADER..=MAX_MESSAGE).contains(&length) {
        bail!("it answered with what isn't a message of property lists");
    }
    let mut body = vec![0; length - HEADER];
    stream.read_exact(&mut body)?;
    Value::from_reader(Cursor::new(body))
        .ok()
        .and_then(Value::into_dictionary)
        .context("it answered with what isn't a property list")
}

#[cfg(test)]
pub mod tests {
    use std::io::{Cursor, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use plist::{Dictionary, Value};

    use super::{Address, Device, Usbmux};

    const TIMEOUT: Duration = Duration::from_secs(10);

    /// A request the made-up daemon read: its fields, and its header's
    /// version, kind and tag.
    pub fn read_message(stream: &mut impl Read) -> (Dictionary, [u32; 3]) {
        let mut header = [0; 16];
        stream.read_exact(&mut header).unwrap();
        let field = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        let mut body = vec![0; field(0) as usize - 16];
        stream.read_exact(&mut body).unwrap();
        let message = Value::from_reader(Cursor::new(body))
            .unwrap()
            .into_dictionary()
            .unwrap();
        (message, [field(4), field(8), field(12)])
    }

    /// Writes `message` as the daemon answers, with the tag 1.
    pub fn write_message(stream: &mut impl Write, message: &Dictionary) {
        let mut body = Vec::new();
        Value::Dictionary(message.clone())
            .to_writer_xml(&mut body)
            .unwrap();
        let mut packet = Vec::new();
        for field in [16 + u32::try_from(body.len()).unwrap(), 1, 8, 1] {
            packet.extend(field.to_le_bytes());
        }
        packet.extend(body);
        stream.write_all(&packet).unwrap();
    }

    /// A dictionary from a property list's XML.
    pub fn dictionary(xml: &str) -> Dictionary {
        Value::from_reader(Cursor::new(format!("<plist>{xml}</plist>")))
            .unwrap()
            .into_dictionary()
            .unwrap()
    }

    /// A made-up daemon on a port of its own, answering each request with
    /// what `answer` gives it, and then, after a `Connect` it answers
    /// with success, handing the connection to `answer` as the device's.
    /// Returns its address.
    pub fn daemon(
        answer: impl Fn(Dictionary, TcpStream) -> Option<Dictionary> + Send + 'static,
    ) -> Address {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let (request, header) = read_message(&mut stream);
                assert_eq!(header, [1, 8, 1]);
                if let Some(reply) = answer(request, stream.try_clone().unwrap()) {
                    write_message(&mut stream, &reply);
                }
            }
        });
        Address::Tcp(address)
    }

    fn usbmux(address: Address) -> Usbmux {
        Usbmux::new(address, TIMEOUT)
    }

    fn iphone() -> Device {
        Device {
            id: 7,
            udid: "00008030-001A2B3C4D5E6F7A".to_owned(),
            usb: true,
        }
    }

    #[test]
    fn finds_the_daemon_where_libusbmuxd_does() {
        assert_eq!(
            Address::new(Some("127.0.0.1:27015")),
            Address::Tcp(([127, 0, 0, 1], 27015).into())
        );
        assert_eq!(
            Address::new(Some("UNIX:/tmp/usbmuxd")),
            Address::Unix("/tmp/usbmuxd".into())
        );
        assert_eq!(
            Address::new(Some("/tmp/usbmuxd")),
            Address::Unix("/tmp/usbmuxd".into())
        );
        assert_eq!(Address::new(None), Address::system());
        assert_eq!(
            Address::new(Some("/tmp/usbmuxd")).to_string(),
            "/tmp/usbmuxd"
        );
    }

    #[test]
    fn lists_the_devices() {
        let address = daemon(|request, _| {
            assert_eq!(request["MessageType"], "ListDevices".into());
            assert_eq!(request["ProgName"], "omni".into());
            assert_eq!(request["kLibUSBMuxVersion"], 3.into());
            Some(dictionary(
                "<dict><key>DeviceList</key><array>
                    <dict><key>DeviceID</key><integer>7</integer><key>MessageType</key><string>Attached</string>
                        <key>Properties</key><dict><key>ConnectionType</key><string>USB</string><key>SerialNumber</key><string>00008030-001A2B3C4D5E6F7A</string></dict></dict>
                    <dict><key>DeviceID</key><integer>8</integer>
                        <key>Properties</key><dict><key>ConnectionType</key><string>Network</string><key>SerialNumber</key><string>00008101-000A1B2C3D4E5F60</string></dict></dict>
                    <dict><key>DeviceID</key><integer>9</integer></dict>
                    <string>a device</string>
                    <dict><key>DeviceID</key><integer>10</integer><key>Properties</key><string>none</string></dict>
                    <dict><key>Properties</key><dict><key>SerialNumber</key><string>A</string></dict></dict>
                    <dict><key>DeviceID</key><string>11</string><key>Properties</key><dict><key>SerialNumber</key><string>B</string></dict></dict>
                    <dict><key>DeviceID</key><integer>12</integer><key>Properties</key><dict></dict></dict>
                    <dict><key>DeviceID</key><integer>13</integer><key>Properties</key><dict><key>SerialNumber</key><integer>13</integer></dict></dict>
                </array></dict>",
            ))
        });

        let devices = usbmux(address).devices().unwrap();

        assert_eq!(
            devices,
            [
                iphone(),
                Device {
                    id: 8,
                    udid: "00008101-000A1B2C3D4E5F60".to_owned(),
                    usb: false,
                },
            ]
        );
    }

    #[test]
    fn reads_and_keeps_pair_records() {
        let address = daemon(|request, _| {
            Some(match request["MessageType"].as_string().unwrap() {
                "ReadBUID" => dictionary("<dict><key>BUID</key><string>BUID-1</string></dict>"),
                "ReadPairRecord" => {
                    if request["PairRecordID"] == "00008030-001A2B3C4D5E6F7A".into() {
                        dictionary("<dict><key>PairRecordData</key><data>cmVjb3Jk</data></dict>")
                    } else {
                        dictionary(
                            "<dict><key>MessageType</key><string>Result</string><key>Number</key><integer>2</integer></dict>",
                        )
                    }
                }
                _ => {
                    assert_eq!(request["PairRecordID"], "00008030-001A2B3C4D5E6F7A".into());
                    assert_eq!(request["PairRecordData"], Value::Data(b"new".to_vec()));
                    assert_eq!(request["DeviceID"], 7.into());
                    dictionary(
                        "<dict><key>MessageType</key><string>Result</string><key>Number</key><integer>0</integer></dict>",
                    )
                }
            })
        });
        let usbmux = usbmux(address);

        assert_eq!(usbmux.buid().unwrap(), "BUID-1");
        assert_eq!(
            usbmux.pair_record("00008030-001A2B3C4D5E6F7A").unwrap(),
            Some(b"record".to_vec())
        );
        assert_eq!(
            usbmux.pair_record("00008101-000A1B2C3D4E5F60").unwrap(),
            None
        );
        usbmux.save_pair_record(&iphone(), b"new").unwrap();
    }

    #[test]
    fn connects_to_a_port_on_a_device() {
        let address = daemon(|request, mut stream| {
            assert_eq!(request["DeviceID"], 7.into());
            // 62078 in network order.
            let port = u16::from_ne_bytes(62078u16.to_be_bytes());
            assert_eq!(request["PortNumber"], port.into());
            write_message(
                &mut stream,
                &dictionary(
                    "<dict><key>MessageType</key><string>Result</string><key>Number</key><integer>0</integer></dict>",
                ),
            );
            // Then it's the device's: an echo.
            let mut buf = [0; 5];
            stream.read_exact(&mut buf).unwrap();
            stream.write_all(&buf).unwrap();
            None
        });

        let mut device = usbmux(address).connect(&iphone(), 62078).unwrap();
        device.write_all(b"hello").unwrap();
        let mut echo = [0; 5];
        device.read_exact(&mut echo).unwrap();

        assert_eq!(&echo, b"hello");
    }

    #[test]
    fn says_why_the_daemon_refused() {
        let result = |number: Option<u32>| {
            let address = daemon(move |_, _| {
                Some(match number {
                    Some(number) => dictionary(&format!(
                        "<dict><key>MessageType</key><string>Result</string><key>Number</key><integer>{number}</integer></dict>"
                    )),
                    None => Dictionary::new(),
                })
            });
            let usbmux = usbmux(address);
            let connect = format!("{:#}", usbmux.connect(&iphone(), 62078).err().unwrap());
            let save = format!("{:#}", usbmux.save_pair_record(&iphone(), b"").unwrap_err());
            (connect, save)
        };

        assert_eq!(
            result(Some(2)).0,
            "couldn't connect to port 62078 on the device: the device isn't connected any more"
        );
        assert_eq!(
            result(Some(3)).0,
            "couldn't connect to port 62078 on the device: the device refused the connection"
        );
        assert_eq!(
            result(Some(6)).1,
            "the usbmux daemon didn't keep the pair record: the usbmux daemon refused it (6)"
        );
        assert_eq!(
            result(None).1,
            "the usbmux daemon didn't keep the pair record: the usbmux daemon answered with no result"
        );
    }

    #[test]
    fn says_what_it_couldnt_read_of_the_daemons_answers() {
        let empty = || daemon(|_, _| Some(Dictionary::new()));
        let answering = |bytes: &'static [u8]| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = stream.unwrap();
                    read_message(&mut stream);
                    stream.write_all(bytes).unwrap();
                }
            });
            usbmux(Address::Tcp(address))
        };
        let message = |kind: u32, body: &[u8]| -> &'static [u8] {
            let mut packet = Vec::new();
            for field in [16 + u32::try_from(body.len()).unwrap(), 1, kind, 1] {
                packet.extend(field.to_le_bytes());
            }
            packet.extend(body);
            packet.leak()
        };

        assert_eq!(
            format!("{:#}", usbmux(empty()).devices().unwrap_err()),
            "the usbmux daemon answered with no list of devices"
        );
        assert_eq!(
            format!("{:#}", usbmux(empty()).buid().unwrap_err()),
            "the usbmux daemon answered with no ID for this computer"
        );
        // The binary protocol's result, which an old daemon answers with.
        assert_eq!(
            format!("{:#}", answering(message(1, &[0; 4])).buid().unwrap_err()),
            "the usbmux daemon didn't answer ReadBUID: it answered with what isn't a message of property lists"
        );
        assert_eq!(
            format!("{:#}", answering(message(8, b"<html>")).buid().unwrap_err()),
            "the usbmux daemon didn't answer ReadBUID: it answered with what isn't a property list"
        );
        assert_eq!(
            format!(
                "{:#}",
                answering(message(8, b"<plist><string>BUID</string></plist>"))
                    .buid()
                    .unwrap_err()
            ),
            "the usbmux daemon didn't answer ReadBUID: it answered with what isn't a property list"
        );
        assert!(
            format!("{:#}", answering(&[1, 2]).buid().unwrap_err())
                .starts_with("the usbmux daemon didn't answer ReadBUID: ")
        );
        // A header that promises more than comes.
        assert!(
            format!(
                "{:#}",
                answering(&message(8, b"<plist>")[..20]).buid().unwrap_err()
            )
            .starts_with("the usbmux daemon didn't answer ReadBUID: ")
        );
    }

    #[test]
    fn says_when_it_cant_reach_the_daemon() {
        // A port nothing listens on any more.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let address = Address::Tcp(([127, 0, 0, 1], port).into());

        let error = format!("{:#}", usbmux(address).devices().unwrap_err());

        assert!(
            error.starts_with(&format!(
                "couldn't reach the usbmux daemon at 127.0.0.1:{port}, which talks to iPhones and iPads: macOS runs it, Windows with Apple Devices or iTunes installed, and Linux with the usbmuxd package: "
            )),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reaches_the_daemon_over_a_unix_socket() {
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usbmuxd");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            read_message(&mut stream);
            write_message(
                &mut stream,
                &dictionary("<dict><key>BUID</key><string>BUID-1</string></dict>"),
            );
        });

        let buid = usbmux(Address::Unix(path.clone())).buid().unwrap();
        let gone = format!(
            "{:#}",
            usbmux(Address::Unix(dir.path().join("gone")))
                .buid()
                .unwrap_err()
        );

        assert_eq!(buid, "BUID-1");
        assert!(
            gone.starts_with("couldn't reach the usbmux daemon at "),
            "{gone}"
        );
    }

    #[test]
    fn says_when_the_daemon_hangs_up() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming() {
                drop(stream);
            }
        });

        let error = usbmux(Address::Tcp(address)).buid().unwrap_err();

        assert!(format!("{error:#}").starts_with("the usbmux daemon didn't answer ReadBUID: "));
    }

    #[test]
    fn writes_requests_as_the_daemon_reads_them() {
        let mut written = Vec::new();
        let mut message = Dictionary::new();
        message.insert("MessageType".to_owned(), "ReadBUID".into());

        super::send(&mut written, &message).unwrap();
        let (read, header) = read_message(&mut Cursor::new(written));

        assert_eq!(read, message);
        assert_eq!(header, [1, 8, 1]);
        // Reading it back as the daemon's answer.
        let mut answer = Vec::new();
        write_message(&mut answer, &message);
        assert_eq!(super::receive(&mut Cursor::new(answer)).unwrap(), message);
    }
}
