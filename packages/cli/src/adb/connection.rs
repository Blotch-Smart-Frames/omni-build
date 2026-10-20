//! ADB's wire protocol, as adb speaks it to a device: messages of a 24-byte
//! header and a payload, a handshake in which the device checks the host's
//! key, and streams to the device's services. AOSP describes it in
//! `packages/modules/adb/protocol.txt`.

use std::io::{ErrorKind, Read, Write};

use anyhow::{Context, Result, anyhow, bail};

use super::key::AdbKey;

const CNXN: u32 = u32::from_le_bytes(*b"CNXN");
const AUTH: u32 = u32::from_le_bytes(*b"AUTH");
const OPEN: u32 = u32::from_le_bytes(*b"OPEN");
const OKAY: u32 = u32::from_le_bytes(*b"OKAY");
const WRTE: u32 = u32::from_le_bytes(*b"WRTE");
const CLSE: u32 = u32::from_le_bytes(*b"CLSE");
/// A device's request for TLS, which Android 11's wireless debugging makes.
const STLS: u32 = u32::from_le_bytes(*b"STLS");

/// The kinds of AUTH message: the device's challenge, the host's answer,
/// and the host's public key, for the device's user to allow.
const AUTH_TOKEN: u32 = 1;
const AUTH_SIGNATURE: u32 = 2;
const AUTH_PUBLIC_KEY: u32 = 3;

/// The protocol version adb has spoken since Android 9, whose devices skip
/// the payload checksum. OmniBuild sends the checksum anyway, which the
/// earlier versions check.
const VERSION: u32 = 0x0100_0001;
/// The largest payload OmniBuild takes, as adb does: 1 MiB. A device says
/// in its CNXN how large a payload it takes.
const MAX_PAYLOAD: u32 = 1024 * 1024;

/// A message: its command, two arguments whose meaning depends on it, and
/// its payload.
#[derive(Debug, PartialEq)]
pub(super) struct Message {
    command: u32,
    arg0: u32,
    arg1: u32,
    data: Vec<u8>,
}

/// What a connection runs over, such as a TCP socket.
pub trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// A connection to a device, after the handshake.
pub struct Connection<'a> {
    stream: Box<dyn Stream + 'a>,
    /// The largest payload the device takes.
    max_payload: usize,
    /// What the device says about itself: its model and the features its
    /// adbd has, among others.
    properties: Vec<(String, String)>,
    /// The ID of the next stream OmniBuild opens.
    next_stream: u32,
}

/// How far the device's check of OmniBuild's key has gone.
enum Auth {
    Unsigned,
    Signed,
    KeySent,
}

/// What the device said next on a stream.
enum Event {
    /// It took the stream, or the last write, and gave its ID for the
    /// stream.
    Okay(u32),
    Closed,
}

impl<'a> Connection<'a> {
    /// Connects to the device at the other end of `stream`, proving
    /// OmniBuild's identity with `key`: by signing the device's token if
    /// the device trusts the key, or by sending the key for its user to
    /// allow if it doesn't, and calling `authorizing` as the device asks.
    pub fn open(stream: impl Stream + 'a, key: &AdbKey, authorizing: &dyn Fn()) -> Result<Self> {
        Self::handshake(Box::new(stream), key, authorizing)
    }

    fn handshake(
        mut stream: Box<dyn Stream + 'a>,
        key: &AdbKey,
        authorizing: &dyn Fn(),
    ) -> Result<Self> {
        send(&mut stream, CNXN, VERSION, MAX_PAYLOAD, b"host::\0")?;
        let mut auth = Auth::Unsigned;
        loop {
            let message = receive(&mut stream)?;
            let (kind, payload) = match (message.command, message.arg0, auth) {
                (CNXN, ..) => return Self::connected(stream, &message),
                (AUTH, AUTH_TOKEN, Auth::Unsigned) => {
                    auth = Auth::Signed;
                    let signature = key.sign(&message.data).with_context(|| {
                        format!(
                            "the device sent a token of {} bytes, and ADB's are 20",
                            message.data.len()
                        )
                    })?;
                    (AUTH_SIGNATURE, signature)
                }
                // Another token: the device doesn't know the key. Like adb,
                // send it, for the device to ask its user whether to allow
                // it, and then connect.
                (AUTH, AUTH_TOKEN, Auth::Signed) => {
                    auth = Auth::KeySent;
                    authorizing();
                    let mut public_key = key.public_key().as_bytes().to_vec();
                    public_key.push(0);
                    (AUTH_PUBLIC_KEY, public_key)
                }
                (AUTH, AUTH_TOKEN, Auth::KeySent) => {
                    bail!("the device didn't allow OmniBuild's key")
                }
                (STLS, ..) => bail!(
                    "the device asks for an encrypted connection, as wireless debugging does, which OmniBuild can't make yet"
                ),
                _ => bail!("the device sent {} in the handshake", name(message.command)),
            };
            send(&mut stream, AUTH, kind, 0, &payload)?;
        }
    }

    /// The connection, given the device's CNXN, whose payload is its banner:
    /// `device::` and its properties, `key=value` each and separated by
    /// semicolons.
    fn connected(stream: Box<dyn Stream + 'a>, message: &Message) -> Result<Self> {
        let banner = String::from_utf8_lossy(&message.data);
        let banner = banner.trim_end_matches('\0');
        let (state, properties) = banner.split_once("::").unwrap_or((banner, ""));
        if state != "device" {
            bail!("the device is in {state} mode, not running Android");
        }
        Ok(Self {
            stream,
            // Never 0, which would leave nothing to write.
            max_payload: message.arg1.clamp(1, MAX_PAYLOAD) as usize,
            properties: properties
                .split(';')
                .filter_map(|property| property.split_once('='))
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            next_stream: 1,
        })
    }

    /// One of the device's properties, from its banner.
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(property, _)| property == name)
            .map(|(_, value)| value.as_str())
    }

    /// Whether the device's adbd has a feature.
    pub fn has_feature(&self, feature: &str) -> bool {
        self.property("features")
            .is_some_and(|features| features.split(',').any(|name| name == feature))
    }

    /// Opens a stream to one of the device's services, such as
    /// `exec:<command>`, writes `input` to it, and returns what the service
    /// writes back until it closes the stream. Nothing when the device
    /// refuses the stream.
    pub fn run(&mut self, service: &str, input: &[u8]) -> Result<Vec<u8>> {
        let local = self.next_stream;
        self.next_stream += 1;
        let mut destination = service.as_bytes().to_vec();
        destination.push(0);
        send(&mut self.stream, OPEN, local, 0, &destination)?;

        // A write at a time, each once the device has taken the last: the
        // device acknowledges the stream, and then each write, with an
        // OKAY, which also carries its ID for the stream.
        let mut chunks = input.chunks(self.max_payload);
        let mut output = Vec::new();
        while let Event::Okay(remote) = self.next_event(local, &mut output)? {
            if let Some(chunk) = chunks.next() {
                send(&mut self.stream, WRTE, local, remote, chunk)?;
            }
        }
        Ok(output)
    }

    /// The device's next message on stream `local` but output: output is
    /// added to `output` and acknowledged.
    fn next_event(&mut self, local: u32, output: &mut Vec<u8>) -> Result<Event> {
        loop {
            let message = receive(&mut self.stream)?;
            match message.command {
                _ if message.arg1 != local => {}
                OKAY => return Ok(Event::Okay(message.arg0)),
                CLSE => return Ok(Event::Closed),
                WRTE => {
                    output.extend(&message.data);
                    send(&mut self.stream, OKAY, local, message.arg0, &[])?;
                    continue;
                }
                _ => {}
            }
            bail!(
                "the device sent {} for stream {}, in the middle of stream {local}",
                name(message.command),
                message.arg1
            );
        }
    }
}

/// Sends a message: its header, with the payload's length and checksum,
/// then the payload.
fn send(stream: &mut dyn Write, command: u32, arg0: u32, arg1: u32, data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).expect("payloads are at most 1 MiB");
    let checksum = data
        .iter()
        .fold(0u32, |sum, &byte| sum.wrapping_add(u32::from(byte)));
    let mut message = Vec::with_capacity(24 + data.len());
    for word in [command, arg0, arg1, len, checksum, !command] {
        message.extend(word.to_le_bytes());
    }
    message.extend(data);
    stream
        .write_all(&message)
        .and_then(|()| stream.flush())
        .context("couldn't send to the device")
}

/// Reads the next message. Its header's last word is its command's
/// complement, which tells ADB from whatever else might be listening.
fn receive(stream: &mut dyn Read) -> Result<Message> {
    let mut header = [0; 24];
    read(stream, &mut header)?;
    let word = |index: usize| {
        u32::from_le_bytes(
            header[index * 4..index * 4 + 4]
                .try_into()
                .expect("4 bytes"),
        )
    };
    let (command, len) = (word(0), word(3));
    if word(5) != !command || len > MAX_PAYLOAD {
        bail!("the device doesn't speak ADB");
    }
    let mut data = vec![0; len as usize];
    read(stream, &mut data)?;
    Ok(Message {
        command,
        arg0: word(1),
        arg1: word(2),
        data,
    })
}

fn read(stream: &mut dyn Read, buf: &mut [u8]) -> Result<()> {
    stream.read_exact(buf).map_err(|error| match error.kind() {
        ErrorKind::UnexpectedEof => anyhow!("the device closed the connection"),
        // What a socket's read timeout gives.
        ErrorKind::WouldBlock | ErrorKind::TimedOut => {
            anyhow!("the device stopped answering")
        }
        _ => anyhow!(error).context("couldn't read from the device"),
    })
}

/// A command's name: its four ASCII letters.
fn name(command: u32) -> String {
    String::from_utf8_lossy(&command.to_le_bytes()).into_owned()
}

#[cfg(test)]
pub mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::io::{self, Cursor, ErrorKind, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::LazyLock;
    use std::thread::JoinHandle;

    use rsa::pkcs8::DecodePrivateKey;
    use rsa::{Pkcs1v15Sign, RsaPrivateKey};
    use sha1::Sha1;

    use super::{
        AUTH, AUTH_PUBLIC_KEY, AUTH_SIGNATURE, AUTH_TOKEN, CLSE, CNXN, Connection, MAX_PAYLOAD,
        Message, OKAY, OPEN, STLS, VERSION, WRTE, name, receive, send,
    };
    use crate::adb::key::AdbKey;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb/adbkey");
    static KEY: LazyLock<AdbKey> =
        LazyLock::new(|| AdbKey::at(Path::new(FIXTURE), "omnibuild@test").unwrap().0);

    /// A banner as an emulator of Android 14 sends it, cut short.
    pub const BANNER: &[u8] =
        b"device::ro.product.name=sdk;ro.product.model=Pixel 8;features=shell_v2,cmd\0";

    /// What a device does with each message it receives: sends messages
    /// back, or with none, hangs up.
    pub trait Respond: FnMut(&Message) -> Option<Vec<Message>> + Send + 'static {}
    impl<F: FnMut(&Message) -> Option<Vec<Message>> + Send + 'static> Respond for F {}

    fn message(command: u32, arg0: u32, arg1: u32, data: &[u8]) -> Message {
        Message {
            command,
            arg0,
            arg1,
            data: data.to_vec(),
        }
    }

    /// A device's CNXN: it connected, and takes payloads of `max_payload`.
    fn connected(max_payload: u32) -> Message {
        message(CNXN, VERSION, max_payload, BANNER)
    }

    /// A device's challenge.
    fn token(len: usize) -> Message {
        message(AUTH, AUTH_TOKEN, 0, &vec![1; len])
    }

    /// A device's OKAY on stream `local`, whose ID for it is 100.
    fn okay(local: u32) -> Message {
        message(OKAY, 100, local, b"")
    }

    /// A device that connects without checking the key, with `banner`, and
    /// then answers `exec:<command> -S <size>` with `output` once it has
    /// taken `size` bytes of input, in writes of at most `max_payload`
    /// bytes. Without `output`, it hangs up when the command comes.
    pub fn exec_device(
        max_payload: u32,
        banner: &'static [u8],
        output: Option<&'static [u8]>,
    ) -> impl Respond {
        let mut expected = 0;
        move |received| match received.command {
            CNXN => Some(vec![message(CNXN, VERSION, max_payload, banner)]),
            OPEN => {
                let service = String::from_utf8_lossy(&received.data);
                let size = service.trim_end_matches('\0').rsplit(' ').next().unwrap();
                expected = size.parse().unwrap();
                output.map(|_| vec![okay(received.arg0)])
            }
            WRTE => {
                assert!(received.data.len() <= max_payload.max(1) as usize);
                expected -= received.data.len();
                let mut responses = vec![okay(received.arg0)];
                if expected == 0 {
                    let output = output.unwrap_or_default();
                    responses.push(message(WRTE, 100, received.arg0, output));
                    responses.push(message(CLSE, 100, received.arg0, b""));
                }
                Some(responses)
            }
            _ => Some(Vec::new()),
        }
    }

    /// A device that doesn't know the key: it sends a second token for the
    /// signature, and once its user allows the key it's sent, it answers
    /// as `respond` does.
    pub fn asking(mut respond: impl Respond) -> impl Respond {
        move |received| match (received.command, received.arg0) {
            (CNXN, _) | (AUTH, AUTH_SIGNATURE) => Some(vec![token(20)]),
            (AUTH, _) => respond(&message(CNXN, VERSION, MAX_PAYLOAD, b"host::\0")),
            _ => respond(received),
        }
    }

    /// A device on `listener`, which answers one connection as `respond`
    /// says. Returns the commands it received, by name, once either end
    /// hangs up.
    pub fn serve(listener: TcpListener, mut respond: impl Respond) -> JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut commands = Vec::new();
            while let Ok(received) = receive(&mut stream) {
                commands.push(name(received.command));
                let Some(responses) = respond(&received) else {
                    break;
                };
                for response in responses {
                    // OmniBuild may have hung up already.
                    let _ = write(&mut stream, &response);
                }
            }
            commands
        })
    }

    fn write(stream: &mut impl Write, message: &Message) -> anyhow::Result<()> {
        send(
            stream,
            message.command,
            message.arg0,
            message.arg1,
            &message.data,
        )
    }

    /// A device in memory: each message OmniBuild writes, which it writes
    /// whole, is answered with what `respond` returns for it, and logged.
    struct Device<F> {
        respond: F,
        log: Vec<Message>,
        outgoing: VecDeque<u8>,
        /// What a read gives when the device has nothing more to say.
        silence: ErrorKind,
        /// How many more writes succeed.
        writes_left: usize,
    }

    impl<F: Respond> Device<F> {
        fn new(respond: F) -> Self {
            Self {
                respond,
                log: Vec::new(),
                outgoing: VecDeque::new(),
                silence: ErrorKind::UnexpectedEof,
                writes_left: usize::MAX,
            }
        }

        /// Bytes the device sends before OmniBuild writes anything.
        fn saying(mut self, bytes: &[u8]) -> Self {
            self.outgoing.extend(bytes);
            self
        }

        /// The commands OmniBuild sent, by name.
        fn commands(&self) -> Vec<String> {
            self.log.iter().map(|sent| name(sent.command)).collect()
        }
    }

    impl<F: Respond> Read for Device<F> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.outgoing.is_empty() && self.silence != ErrorKind::UnexpectedEof {
                return Err(self.silence.into());
            }
            self.outgoing.read(buf)
        }
    }

    impl<F: Respond> Write for Device<F> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.writes_left == 0 {
                return Err(ErrorKind::BrokenPipe.into());
            }
            self.writes_left -= 1;
            let sent = receive(&mut Cursor::new(buf)).unwrap();
            // Hanging up leaves nothing more to read.
            for response in (self.respond)(&sent).into_iter().flatten() {
                write(&mut self.outgoing, &response).unwrap();
            }
            self.log.push(sent);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// For a device that never asks its user to allow the key, or whose
    /// asking doesn't matter.
    fn unasked() {}

    /// A device that answers every message with `messages`.
    fn always(messages: fn() -> Vec<Message>) -> impl Respond {
        move |_: &Message| Some(messages())
    }

    #[test]
    fn connects_to_a_device_that_doesnt_check_the_key() {
        let mut device = Device::new(always(|| vec![connected(4096)]));

        let connection = Connection::open(&mut device, &KEY, &unasked).unwrap();

        assert_eq!(connection.property("ro.product.model"), Some("Pixel 8"));
        assert_eq!(connection.property("ro.serialno"), None);
        assert!(connection.has_feature("cmd"));
        assert!(!connection.has_feature("abb_exec"));
        assert_eq!(connection.max_payload, 4096);
        drop(connection);
        // adb's version and payload limit, and the banner adb sent before
        // it had features to name, with the checksum earlier versions check.
        assert_eq!(
            device.log,
            [message(CNXN, VERSION, MAX_PAYLOAD, b"host::\0")]
        );
        let mut bytes = Vec::new();
        write(&mut bytes, &device.log[0]).unwrap();
        assert_eq!(
            bytes[16..20],
            (b"host::".iter().map(|&b| u32::from(b)).sum::<u32>()).to_le_bytes()
        );
        assert_eq!(bytes[20..24], (!CNXN).to_le_bytes());
    }

    #[test]
    fn signs_the_token_of_a_device_that_trusts_the_key() {
        let mut device = Device::new(|received: &Message| {
            Some(vec![match received.command {
                CNXN => message(AUTH, AUTH_TOKEN, 0, &[9; 20]),
                _ => connected(MAX_PAYLOAD),
            }])
        });

        Connection::open(&mut device, &KEY, &unasked).unwrap();

        // The device trusts the key, so it doesn't ask for it.
        assert_eq!(device.commands(), ["CNXN", "AUTH"]);
        let signature = &device.log[1];
        assert_eq!((signature.command, signature.arg0), (AUTH, AUTH_SIGNATURE));
        RsaPrivateKey::from_pkcs8_pem(&std::fs::read_to_string(FIXTURE).unwrap())
            .unwrap()
            .to_public_key()
            .verify(Pkcs1v15Sign::new::<Sha1>(), &[9; 20], &signature.data)
            .unwrap();
    }

    #[test]
    fn sends_the_key_to_a_device_that_doesnt_know_it() {
        // A device that sends a new token for the signature, and connects
        // once its user allows the key.
        let mut device = Device::new(|received: &Message| {
            Some(vec![match (received.command, received.arg0) {
                (AUTH, AUTH_PUBLIC_KEY) => connected(MAX_PAYLOAD),
                _ => token(20),
            }])
        });

        let asked = Cell::new(0);
        Connection::open(&mut device, &KEY, &|| asked.set(asked.get() + 1)).unwrap();

        // Once, as it sends the key, for the user to see the device asking.
        assert_eq!(asked.get(), 1);
        assert_eq!(device.commands(), ["CNXN", "AUTH", "AUTH"]);
        let public_key = &device.log[2];
        assert_eq!(public_key.arg0, AUTH_PUBLIC_KEY);
        assert_eq!(
            public_key.data,
            format!("{}\0", KEY.public_key()).as_bytes()
        );
    }

    #[test]
    fn says_why_it_couldnt_connect() {
        let refuses = |messages: fn() -> Vec<Message>| {
            let error = Connection::open(Device::new(always(messages)), &KEY, &unasked)
                .err()
                .unwrap();
            format!("{error:#}")
        };

        assert_eq!(
            refuses(|| vec![token(20)]),
            "the device didn't allow OmniBuild's key"
        );
        assert_eq!(
            refuses(|| vec![token(8)]),
            "the device sent a token of 8 bytes, and ADB's are 20"
        );
        assert!(refuses(|| vec![message(STLS, 0, 0, b"")]).contains("encrypted connection"));
        assert_eq!(
            refuses(|| vec![message(OKAY, 0, 0, b"")]),
            "the device sent OKAY in the handshake"
        );
        assert_eq!(
            refuses(|| vec![message(CNXN, VERSION, MAX_PAYLOAD, b"recovery::")]),
            "the device is in recovery mode, not running Android"
        );
        assert_eq!(
            refuses(|| vec![message(CNXN, VERSION, MAX_PAYLOAD, b"bootloader")]),
            "the device is in bootloader mode, not running Android"
        );
        assert_eq!(refuses(Vec::new), "the device closed the connection");
    }

    #[test]
    fn tells_adb_from_whatever_else_listens() {
        let refuses = |bytes: &[u8]| {
            let device = Device::new(always(Vec::new)).saying(bytes);
            format!(
                "{:#}",
                Connection::open(device, &KEY, &unasked).err().unwrap()
            )
        };
        let mut oversized = Vec::new();
        send(&mut oversized, CNXN, 0, 0, b"").unwrap();
        oversized[12..16].copy_from_slice(&(MAX_PAYLOAD + 1).to_le_bytes());
        let mut truncated = Vec::new();
        send(&mut truncated, CNXN, VERSION, 0, BANNER).unwrap();
        truncated.truncate(30);

        assert_eq!(
            refuses(b"HTTP/1.1 400 Bad Request\r\n\r\n"),
            "the device doesn't speak ADB"
        );
        assert_eq!(refuses(&oversized), "the device doesn't speak ADB");
        assert_eq!(refuses(&truncated), "the device closed the connection");
    }

    #[test]
    fn says_when_the_device_stops_answering_or_the_connection_resets() {
        let mut silent = Device::new(always(Vec::new));
        silent.silence = ErrorKind::WouldBlock;
        let mut reset = Device::new(always(Vec::new));
        reset.silence = ErrorKind::ConnectionReset;

        assert_eq!(
            format!(
                "{:#}",
                Connection::open(silent, &KEY, &unasked).err().unwrap()
            ),
            "the device stopped answering"
        );
        assert!(
            format!(
                "{:#}",
                Connection::open(reset, &KEY, &unasked).err().unwrap()
            )
            .starts_with("couldn't read from the device: ")
        );
    }

    #[test]
    fn writes_input_a_payload_at_a_time_and_returns_the_output() {
        let mut device = Device::new(exec_device(4096, BANNER, Some(b"Success\n")));
        let input: Vec<u8> = (0..10_000u32).map(|n| n.to_le_bytes()[0]).collect();
        let mut connection = Connection::open(&mut device, &KEY, &unasked).unwrap();

        let output = connection.run("exec:cat -S 10000", &input).unwrap();
        let second = connection.run("exec:true -S 1", b"!").unwrap();

        assert_eq!(output, b"Success\n");
        assert_eq!(second, b"Success\n");
        drop(connection);
        assert_eq!(
            device.commands(),
            [
                "CNXN", "OPEN", "WRTE", "WRTE", "WRTE", "OKAY", "OPEN", "WRTE", "OKAY"
            ]
        );
        let open = &device.log[1];
        assert_eq!((open.arg0, open.arg1), (1, 0));
        assert_eq!(open.data, b"exec:cat -S 10000\0");
        let writes = &device.log[2..5];
        assert!(
            writes
                .iter()
                .all(|write| (write.arg0, write.arg1) == (1, 100))
        );
        let written: Vec<u8> = writes.iter().flat_map(|write| write.data.clone()).collect();
        assert_eq!(written, input);
        // The output acknowledged, with both IDs for the stream; the second
        // stream has an ID of its own.
        assert_eq!((device.log[5].arg0, device.log[5].arg1), (1, 100));
        assert_eq!(device.log[6].arg0, 2);
    }

    /// A device that connects, then runs any service as one that fails as
    /// soon as it starts, as `cmd` does when told something it doesn't
    /// know: it writes "Failure [nope]", in two parts, and closes the
    /// stream, before it takes any input.
    fn failing_device() -> impl Respond {
        |received: &Message| {
            Some(match received.command {
                CNXN => vec![connected(4096)],
                OPEN => vec![
                    okay(received.arg0),
                    message(WRTE, 100, received.arg0, b"Failure "),
                    message(WRTE, 100, received.arg0, b"[nope]\n"),
                    message(CLSE, 100, received.arg0, b""),
                ],
                _ => Vec::new(),
            })
        }
    }

    #[test]
    fn keeps_output_the_device_writes_before_it_takes_all_the_input() {
        let mut device = Device::new(failing_device());
        let mut connection = Connection::open(&mut device, &KEY, &unasked).unwrap();

        let output = connection.run("exec:false", &[0; 10_000]).unwrap();

        assert_eq!(output, b"Failure [nope]\n");
        drop(connection);
        // One write, sent before the device's output arrived.
        assert_eq!(device.commands(), ["CNXN", "OPEN", "WRTE", "OKAY", "OKAY"]);
    }

    #[test]
    fn says_when_it_cant_send_to_the_device() {
        // Each kind of write fails in turn, after `writes` others.
        let handshake = |writes| {
            let mut device = Device::new(always(|| vec![token(20)]));
            device.writes_left = writes;
            Connection::open(device, &KEY, &unasked).err().unwrap()
        };
        let stream = |writes, input: &'static [u8]| {
            let mut device = Device::new(failing_device());
            device.writes_left = writes;
            let mut connection = Connection::open(device, &KEY, &unasked).unwrap();
            connection.run("exec:false", input).err().unwrap()
        };

        let errors = [
            // The CNXN and the signature.
            handshake(0),
            handshake(1),
            // The OPEN, the input and the OKAY for output.
            stream(1, b""),
            stream(2, b"input"),
            stream(2, b""),
        ];

        for error in errors {
            assert!(
                format!("{error:#}").starts_with("couldn't send to the device: "),
                "{error:#}"
            );
        }
    }

    #[test]
    fn returns_nothing_when_the_device_refuses_a_stream() {
        let mut device = Device::new(|received: &Message| {
            Some(vec![match received.command {
                CNXN => connected(4096),
                _ => message(CLSE, 0, received.arg0, b""),
            }])
        });
        let mut connection = Connection::open(&mut device, &KEY, &unasked).unwrap();

        assert_eq!(connection.run("nonsense:", b"input").unwrap(), b"");
    }

    #[test]
    fn refuses_messages_out_of_turn() {
        let fails = |stray: Message| {
            let mut stray = Some(stray);
            let device = Device::new(move |received: &Message| {
                Some(vec![match received.command {
                    CNXN => connected(4096),
                    _ => stray.take().unwrap(),
                }])
            });
            let mut connection = Connection::open(device, &KEY, &unasked).unwrap();
            format!("{:#}", connection.run("exec:true", b"").err().unwrap())
        };

        assert_eq!(
            fails(message(OKAY, 100, 7, b"")),
            "the device sent OKAY for stream 7, in the middle of stream 1"
        );
        assert_eq!(
            fails(message(CNXN, 100, 1, b"")),
            "the device sent CNXN for stream 1, in the middle of stream 1"
        );
    }

    #[test]
    fn writes_something_to_a_device_that_says_it_takes_nothing() {
        let mut device = Device::new(exec_device(0, BANNER, Some(b"Success\n")));
        let mut connection = Connection::open(&mut device, &KEY, &unasked).unwrap();

        assert_eq!(connection.max_payload, 1);
        assert_eq!(
            connection.run("exec:true -S 2", b"ok").unwrap(),
            b"Success\n"
        );
    }

    #[test]
    fn talks_to_a_device_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let device = serve(listener, exec_device(4096, BANNER, Some(b"Success\n")));

        let mut connection =
            Connection::open(TcpStream::connect(address).unwrap(), &KEY, &unasked).unwrap();

        assert_eq!(
            connection.run("exec:true -S 5", b"input").unwrap(),
            b"Success\n"
        );
        drop(connection);
        assert_eq!(device.join().unwrap(), ["CNXN", "OPEN", "WRTE", "OKAY"]);
    }

    #[test]
    fn says_when_a_device_hangs_up_on_a_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let device = serve(listener, exec_device(4096, BANNER, None));
        let mut connection =
            Connection::open(TcpStream::connect(address).unwrap(), &KEY, &unasked).unwrap();

        let error = connection.run("exec:true -S 5", b"input").err().unwrap();

        assert_eq!(format!("{error:#}"), "the device closed the connection");
        assert_eq!(device.join().unwrap(), ["CNXN", "OPEN"]);
    }
}
