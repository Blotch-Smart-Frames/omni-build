//! AFC, Apple File Conduit: the service through which a computer reads and
//! writes the files an iPhone or iPad shares with it, the folder installers
//! stage apps in among them. A request and its answer are a packet each: a
//! header, the operation's own fields, and the data it carries.

use std::io::{Read, Write};

use anyhow::{Context, Result, bail};

use super::Stream;

/// What every packet starts with.
const MAGIC: &[u8; 8] = b"CFA6LPAA";
/// A packet's header: the magic, then the packet's length, the length of
/// its header and fields, its number and its operation, eight bytes each,
/// little-endian.
const HEADER: usize = 40;
/// The operations OmniBuild sends, and those the device answers with, by
/// their numbers.
const STATUS: u64 = 0x01;
const REMOVE_PATH: u64 = 0x08;
const MAKE_DIR: u64 = 0x09;
const FILE_OPEN: u64 = 0x0d;
const FILE_OPEN_RESULT: u64 = 0x0e;
const FILE_WRITE: u64 = 0x10;
const FILE_CLOSE: u64 = 0x14;
/// How a file is opened to be written: created, or emptied if it's there.
const WRITE_ONLY: u64 = 3;
/// The most a write carries, as libimobiledevice writes.
const CHUNK: usize = 1 << 20;
/// The most OmniBuild reads of an answer, which only says how it went.
const MAX_ANSWER: u64 = 1 << 16;

/// A connection to AFC.
pub struct Afc {
    stream: Box<dyn Stream>,
    /// The last packet's number, which the device's answer repeats.
    packet: u64,
}

impl Afc {
    pub fn new(stream: Box<dyn Stream>) -> Self {
        Self { stream, packet: 0 }
    }

    /// Makes the folder `path`, and those it's in, if they're not there.
    pub fn make_directory(&mut self, path: &str) -> Result<()> {
        self.operation(MAKE_DIR, &c_string(path), &[])
            .with_context(|| format!("couldn't make the folder {path} on the device"))?;
        Ok(())
    }

    /// Writes `contents` to the file at `path`, replacing any there.
    pub fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<()> {
        let context = || format!("couldn't write {path} on the device");
        let mut fields = WRITE_ONLY.to_le_bytes().to_vec();
        fields.extend(c_string(path));
        let (operation, answer) = self
            .operation(FILE_OPEN, &fields, &[])
            .with_context(context)?;
        let handle = match (operation, answer.get(..8)) {
            (FILE_OPEN_RESULT, Some(handle)) => handle.to_vec(),
            _ => bail!("{}: the device didn't open it", context()),
        };
        for chunk in contents.chunks(CHUNK) {
            self.operation(FILE_WRITE, &handle, chunk)
                .with_context(context)?;
        }
        self.operation(FILE_CLOSE, &handle, &[])
            .with_context(context)?;
        Ok(())
    }

    /// Removes the file at `path`.
    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.operation(REMOVE_PATH, &c_string(path), &[])
            .with_context(|| format!("couldn't remove {path} from the device"))?;
        Ok(())
    }

    /// Sends a packet of `operation`, with its `fields` and the `data` it
    /// carries, and returns the device's answer: its operation, and what
    /// came after its header. A status other than success is an error.
    fn operation(&mut self, operation: u64, fields: &[u8], data: &[u8]) -> Result<(u64, Vec<u8>)> {
        self.packet += 1;
        let fields_end = (HEADER + fields.len()) as u64;
        let mut packet = MAGIC.to_vec();
        for field in [
            fields_end + data.len() as u64,
            fields_end,
            self.packet,
            operation,
        ] {
            packet.extend(field.to_le_bytes());
        }
        packet.extend(fields);
        packet.extend(data);
        self.stream.write_all(&packet)?;
        let mut header = [0; HEADER];
        self.stream.read_exact(&mut header)?;
        let field = |at: usize| {
            u64::from_le_bytes(
                header[at..at + 8]
                    .try_into()
                    .expect("a field is eight bytes"),
            )
        };
        let length = field(8);
        if &header[..8] != MAGIC
            || field(24) != self.packet
            || !(HEADER as u64..=MAX_ANSWER).contains(&length)
        {
            bail!("the device answered with what isn't an AFC packet");
        }
        let mut answer = vec![0; usize::try_from(length).expect("an answer fits memory") - HEADER];
        self.stream.read_exact(&mut answer)?;
        let operation = field(32);
        if operation == STATUS {
            let status = answer.get(..8).map_or(0, |status| {
                u64::from_le_bytes(status.try_into().expect("eight bytes"))
            });
            if status != 0 {
                bail!(status_error(status));
            }
        }
        Ok((operation, answer))
    }
}

/// `text` as AFC takes a path: ending in a zero byte.
fn c_string(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// What AFC's status `status` means, as libimobiledevice names them.
fn status_error(status: u64) -> String {
    let meaning = match status {
        7 => "an argument was invalid",
        8 => "it isn't there",
        9 => "it's a folder",
        10 => "permission was denied",
        16 => "it's there already",
        18 => "the device has no space left",
        _ => return format!("AFC failed with the status {status}"),
    };
    format!("{meaning} (AFC status {status})")
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::{Afc, CHUNK, status_error};

    /// The files a made-up AFC holds, by their paths, and its folders,
    /// with an empty file.
    pub type Files = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

    /// A request a made-up AFC read: its number, operation, fields and
    /// data.
    fn read_packet(stream: &mut impl Read) -> Option<(u64, u64, Vec<u8>, Vec<u8>)> {
        let mut header = [0; 40];
        stream.read_exact(&mut header).ok()?;
        assert_eq!(&header[..8], b"CFA6LPAA");
        let field = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
        let mut fields = vec![0; usize::try_from(field(16)).unwrap() - 40];
        stream.read_exact(&mut fields).unwrap();
        let mut data = vec![0; usize::try_from(field(8) - field(16)).unwrap()];
        stream.read_exact(&mut data).unwrap();
        Some((field(24), field(32), fields, data))
    }

    fn write_packet(stream: &mut impl Write, number: u64, operation: u64, body: &[u8]) {
        let mut packet = b"CFA6LPAA".to_vec();
        let length = 40 + body.len() as u64;
        for field in [length, length, number, operation] {
            packet.extend(field.to_le_bytes());
        }
        packet.extend(body);
        stream.write_all(&packet).unwrap();
    }

    fn path(bytes: &[u8]) -> String {
        String::from_utf8(bytes.strip_suffix(&[0]).unwrap().to_vec()).unwrap()
    }

    /// Answers the AFC requests on `stream` as a device does, keeping the
    /// files they write in `files`, until the stream ends. Refuses the
    /// operation `refused` with the status 10, permission denied.
    pub fn serve_afc(mut stream: impl Read + Write, files: &Files, refused: Option<u64>) {
        let status = |stream: &mut _, number, code: u64| {
            write_packet(stream, number, 1, &code.to_le_bytes());
        };
        let mut open = None;
        while let Some((number, operation, fields, data)) = read_packet(&mut stream) {
            if refused == Some(operation) {
                status(&mut stream, number, 10);
                continue;
            }
            match operation {
                0x09 => {
                    files.lock().unwrap().insert(path(&fields), Vec::new());
                    status(&mut stream, number, 0);
                }
                0x0d => {
                    assert_eq!(&fields[..8], &3u64.to_le_bytes());
                    let path = path(&fields[8..]);
                    files.lock().unwrap().insert(path.clone(), Vec::new());
                    open = Some(path);
                    write_packet(&mut stream, number, 0x0e, &42u64.to_le_bytes());
                }
                0x10 => {
                    assert_eq!(fields, 42u64.to_le_bytes());
                    let path = open.clone().unwrap();
                    files.lock().unwrap().get_mut(&path).unwrap().extend(data);
                    status(&mut stream, number, 0);
                }
                0x14 => {
                    open = None;
                    status(&mut stream, number, 0);
                }
                _ => {
                    let gone = files.lock().unwrap().remove(&path(&fields));
                    status(&mut stream, number, if gone.is_some() { 0 } else { 8 });
                }
            }
        }
    }

    /// A made-up AFC on a connection of its own, and a client of it.
    fn afc(refused: Option<u64>) -> (Afc, Files) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let files = Files::default();
        let kept = Arc::clone(&files);
        thread::spawn(move || serve_afc(listener.accept().unwrap().0, &kept, refused));
        (
            Afc::new(Box::new(TcpStream::connect(address).unwrap())),
            files,
        )
    }

    #[test]
    fn writes_a_file_in_a_folder_and_removes_it() {
        let (mut afc, files) = afc(None);
        // Larger than a write carries.
        let contents: Vec<u8> = (0..CHUNK + 10)
            .map(|at| u8::try_from(at % 251).unwrap())
            .collect();

        afc.make_directory("PublicStaging").unwrap();
        afc.write_file("PublicStaging/app.ipa", &contents).unwrap();

        assert_eq!(files.lock().unwrap()["PublicStaging/app.ipa"], contents);
        assert!(files.lock().unwrap().contains_key("PublicStaging"));
        afc.remove("PublicStaging/app.ipa").unwrap();
        assert!(!files.lock().unwrap().contains_key("PublicStaging/app.ipa"));
        assert_eq!(
            format!("{:#}", afc.remove("PublicStaging/app.ipa").unwrap_err()),
            "couldn't remove PublicStaging/app.ipa from the device: it isn't there (AFC status 8)"
        );
    }

    #[test]
    fn says_what_the_device_refused() {
        let refused = |operation: u64| {
            let (mut afc, _) = afc(Some(operation));
            let error = if operation == 0x09 {
                afc.make_directory("PublicStaging").unwrap_err()
            } else {
                afc.write_file("PublicStaging/app.ipa", b"app").unwrap_err()
            };
            format!("{error:#}")
        };

        assert_eq!(
            refused(0x09),
            "couldn't make the folder PublicStaging on the device: permission was denied (AFC status 10)"
        );
        for operation in [0x0d, 0x10, 0x14] {
            assert_eq!(
                refused(operation),
                "couldn't write PublicStaging/app.ipa on the device: permission was denied (AFC status 10)"
            );
        }
    }

    /// An AFC client of a device that answers a request with `answer`, and
    /// hangs up.
    fn answering(answer: Vec<u8>) -> Afc {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            read_packet(&mut stream);
            stream.write_all(&answer).unwrap();
        });
        Afc::new(Box::new(TcpStream::connect(address).unwrap()))
    }

    #[test]
    fn says_when_the_device_answers_with_what_isnt_afc() {
        let packet = |number: u64, operation: u64, body: &[u8]| {
            let mut bytes = Vec::new();
            write_packet(&mut bytes, number, operation, body);
            bytes
        };
        let mut bad_magic = packet(1, 1, &[0; 8]);
        bad_magic[0] = b'X';
        let mut too_long = packet(1, 1, &[0; 8]);
        too_long[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        let error = |answer: Vec<u8>| format!("{:#}", answering(answer).remove("a").unwrap_err());

        for answer in [bad_magic, too_long, packet(2, 1, &[0; 8])] {
            assert_eq!(
                error(answer),
                "couldn't remove a from the device: the device answered with what isn't an AFC packet"
            );
        }
        // A status with no code is success, as libimobiledevice reads one.
        assert!(answering(packet(1, 1, &[])).remove("a").is_ok());
        // An open that hands back no file.
        assert_eq!(
            format!(
                "{:#}",
                answering(packet(1, 2, &[]))
                    .write_file("a", b"")
                    .unwrap_err()
            ),
            "couldn't write a on the device: the device didn't open it"
        );
        assert!(error(b"CFA6".to_vec()).starts_with("couldn't remove a from the device: "));
        // A header that promises more than comes.
        let mut cut_short = packet(1, 1, &[0; 8]);
        cut_short.truncate(44);
        assert!(error(cut_short).starts_with("couldn't remove a from the device: "));
    }

    #[test]
    fn names_the_statuses_it_knows() {
        assert_eq!(
            status_error(18),
            "the device has no space left (AFC status 18)"
        );
        assert_eq!(status_error(7), "an argument was invalid (AFC status 7)");
        assert_eq!(status_error(9), "it's a folder (AFC status 9)");
        assert_eq!(status_error(16), "it's there already (AFC status 16)");
        assert_eq!(status_error(33), "AFC failed with the status 33");
    }

    #[test]
    fn says_when_the_connection_breaks() {
        let mut afc = Afc::new(Box::new(crate::mobile_device::tests::Broken));

        let error = format!("{:#}", afc.remove("a").unwrap_err());

        assert!(
            error.starts_with("couldn't remove a from the device: "),
            "{error}"
        );
    }

    #[test]
    fn says_when_the_device_hangs_up() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || drop(listener.accept().unwrap()));
        let mut afc = Afc::new(Box::new(TcpStream::connect(address).unwrap()));

        let error = format!("{:#}", afc.make_directory("PublicStaging").unwrap_err());

        assert!(error.starts_with("couldn't make the folder PublicStaging on the device: "));
    }
}
