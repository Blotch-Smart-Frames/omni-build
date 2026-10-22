//! ADB over USB, as adb speaks it to a phone plugged into this computer
//! with USB debugging on: through the interface adbd adds to the phone's
//! USB functions, with each system's own USB API, which nusb wraps.

use std::io::{self, Read, Write};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use nusb::descriptors::TransferType;
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, Direction, In, Out};
use nusb::{DeviceInfo, ErrorKind, Interface, InterfaceInfo, MaybeFuture};

use super::UsbDevices;
use super::connection::Stream;

/// The interface adbd offers, as adb looks for it: of a vendor's own
/// class, subclass 0x42 and protocol 1.
const ADB_INTERFACE: (u8, u8, u8) = (0xff, 0x42, 1);
/// How much each read asks the phone for. nusb joins what the reads bring
/// into one stream, so a payload larger than this comes in several.
const READ_SIZE: usize = 16 * 1024;
/// How much each write sends at most: the largest payload, so each of
/// OmniBuild's goes in one transfer, as adb sends them.
const WRITE_SIZE: usize = 1024 * 1024;

/// The phones plugged into this computer.
pub struct Usb;

impl UsbDevices for Usb {
    fn names(&self) -> Vec<String> {
        phones().iter().map(|(phone, _)| name(phone)).collect()
    }

    fn open(&self, name: &str, timeout: Duration) -> Result<Box<dyn Stream>> {
        let (phone, interface) = phones()
            .into_iter()
            .find(|(phone, _)| self::name(phone) == name)
            .context("it was unplugged")?;
        let interface = phone
            .open()
            .wait()
            .and_then(|device| device.claim_interface(interface).wait())
            .map_err(|error| {
                let why = why(error.kind());
                anyhow!(error).context(why)
            })?;
        let stream = bulk(&interface, Direction::In)
            .zip(bulk(&interface, Direction::Out))
            .context("its ADB interface has no bulk endpoints")
            .and_then(|(input, output)| {
                interface
                    .endpoint::<Bulk, In>(input)
                    .and_then(|input| {
                        interface
                            .endpoint::<Bulk, Out>(output)
                            .map(|output| (input, output))
                    })
                    .map_err(anyhow::Error::from)
            })?;
        Ok(Box::new(UsbStream {
            reader: stream.0.reader(READ_SIZE).with_read_timeout(timeout),
            writer: stream.1.writer(WRITE_SIZE).with_write_timeout(timeout),
        }))
    }
}

/// The devices plugged in that offer ADB's interface, each with that
/// interface's number. None when the system can't list its devices.
fn phones() -> Vec<(DeviceInfo, u8)> {
    let devices = nusb::list_devices().wait().into_iter().flatten();
    devices
        .filter_map(|device| {
            let adb = device
                .interfaces()
                .find(|interface| {
                    (
                        interface.class(),
                        interface.subclass(),
                        interface.protocol(),
                    ) == ADB_INTERFACE
                })
                .map(InterfaceInfo::interface_number);
            adb.map(|number| (device, number))
        })
        .collect()
}

/// A phone's name, as adb's: its serial number, or where it's plugged in
/// when it has none, its bus and its port on each hub, such as
/// `usb:001-3`.
fn name(phone: &DeviceInfo) -> String {
    let serial = phone.serial_number().filter(|serial| !serial.is_empty());
    serial.map_or_else(
        || {
            let ports: Vec<_> = phone.port_chain().iter().map(u8::to_string).collect();
            format!("usb:{}-{}", phone.bus_id(), ports.join("."))
        },
        str::to_owned,
    )
}

/// The address of `interface`'s bulk endpoint in `direction`.
fn bulk(interface: &Interface, direction: Direction) -> Option<u8> {
    interface
        .descriptor()
        .into_iter()
        .flat_map(|descriptor| descriptor.endpoints())
        .find(|endpoint| {
            endpoint.transfer_type() == TransferType::Bulk && endpoint.direction() == direction
        })
        .map(|endpoint| endpoint.address())
}

/// Why the system wouldn't let OmniBuild use a phone, as its user can do
/// something about it.
fn why(kind: ErrorKind) -> &'static str {
    match kind {
        // adb's server keeps the interface of every phone it finds.
        ErrorKind::Busy => {
            "another program is using it, such as adb's server: stop that with adb kill-server"
        }
        // Linux gives devices to root until a udev rule gives them to
        // their user, as Android's own rules do.
        ErrorKind::PermissionDenied => {
            "this user may not open it: on Linux, Android's udev rules allow that, from android-sdk-platform-tools-common on Debian and Ubuntu"
        }
        // Windows lets programs use a device only through its driver, and
        // nusb only through WinUSB.
        ErrorKind::Unsupported => {
            "its driver isn't WinUSB, which OmniBuild opens phones through on Windows: install one for its ADB interface, such as Google's USB driver"
        }
        ErrorKind::Disconnected => "it was unplugged",
        _ => "the system wouldn't open it",
    }
}

/// A phone's ADB interface, as a stream each way. Flushing ends a transfer
/// with a short packet, or a zero-length one when it has none: adbd reads
/// each message's header and payload as transfers of their own, and only a
/// short packet tells it where one ends.
struct UsbStream {
    reader: EndpointRead<Bulk>,
    writer: EndpointWrite<Bulk>,
}

impl Read for UsbStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Write for UsbStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush_end()
    }
}

#[cfg(test)]
mod tests {
    use nusb::ErrorKind;

    use super::{Usb, why};
    use crate::adb::{TIMEOUT, UsbDevices};

    #[test]
    fn says_why_the_system_wouldnt_open_a_phone() {
        let reasons = [
            ErrorKind::Busy,
            ErrorKind::PermissionDenied,
            ErrorKind::Unsupported,
            ErrorKind::Disconnected,
            ErrorKind::Other,
        ]
        .map(why);

        assert!(reasons[0].contains("adb kill-server"));
        assert!(reasons[1].contains("udev rule"));
        assert!(reasons[2].contains("WinUSB"));
        assert_eq!(reasons[3], "it was unplugged");
        assert_eq!(reasons[4], "the system wouldn't open it");
    }

    #[test]
    fn says_when_a_phone_was_unplugged() {
        let error = Usb.open("OMNIBUILD-NONE", TIMEOUT).err().unwrap();

        assert_eq!(error.to_string(), "it was unplugged");
    }

    /// Tests with phones plugged in: made-up ones, which Linux's vhci-hcd
    /// attaches, the virtual USB host controller USB/IP brings remote
    /// devices in through. The kernel enumerates each as it would a real
    /// phone, and nusb reaches it through usbfs, while each phone speaks
    /// USB/IP to the kernel over a socket a thread here answers on. Only
    /// Linux has a virtual USB host controller, so only Linux runs them.
    /// CI loads vhci-hcd, and lets its user attach devices to it and open
    /// Google's; Docker Desktop's Linux has it built in, for a privileged
    /// container run by root.
    #[cfg(target_os = "linux")]
    mod plugged {
        use std::cell::Cell;
        use std::collections::VecDeque;
        use std::fs::{self, OpenOptions};
        use std::io::{self, Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::os::fd::AsRawFd;
        use std::path::Path;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::{LazyLock, Mutex};
        use std::thread::{self, JoinHandle};
        use std::time::Duration;

        use nusb::{DeviceInfo, MaybeFuture};

        use crate::adb::connection::tests::{
            BANNER, Respond, asking, exec_device, take_message, unasked, write,
        };
        use crate::adb::connection::{Connection, Message};
        use crate::adb::{AdbKey, Device, TIMEOUT, Usb, UsbDevices};

        const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb/adbkey");
        static KEY: LazyLock<AdbKey> =
            LazyLock::new(|| AdbKey::at(Path::new(FIXTURE), "omnibuild@test").unwrap().0);

        /// Where vhci-hcd takes the devices to attach, and lists its ports.
        const VHCI: &str = "/sys/devices/platform/vhci_hcd.0";
        /// USB/IP's commands: to the device, a transfer, and the
        /// cancelling of one; to the host, the answer to each.
        const CMD_UNLINK: u32 = 2;
        const RET_SUBMIT: u32 = 3;
        const RET_UNLINK: u32 = 4;
        /// A transfer's direction, to the host.
        const IN: u32 = 1;
        /// What the kernel takes as a transfer cancelled: `-ECONNRESET`.
        const CANCELLED: i32 = -104;
        /// The speed phones plug in at, USB 2.0's: `USB_SPEED_HIGH`.
        const HIGH_SPEED: u32 = 3;
        /// Google's vendor ID, whose devices CI's udev rule lets its user
        /// open, as Android's rules do.
        const GOOGLE: u16 = 0x18d1;

        /// Held while a test picks a free port and attaches a phone to it.
        static ATTACHING: Mutex<()> = Mutex::new(());
        /// How many phones have been plugged in, which tells each one's
        /// product name from the others'.
        static PHONES: AtomicU32 = AtomicU32::new(0);

        /// A phone plugged in: its name, and the socket the kernel speaks
        /// to it on, which a thread answers.
        struct Plugged {
            name: String,
            socket: TcpStream,
            answering: JoinHandle<Vec<usize>>,
        }

        impl Plugged {
            /// Unplugs it. Returns the length of each transfer OmniBuild
            /// sent to its ADB interface.
            fn unplug(self) -> Vec<usize> {
                self.socket.shutdown(Shutdown::Both).unwrap();
                self.answering.join().unwrap()
            }
        }

        /// Plugs in a phone with `serial`, or none, whose ADB interface has
        /// a bulk endpoint each way if `endpoints`, and whose adbd answers
        /// as `respond` says. Boxed, so this and `answer` are one function
        /// each, whose coverage is one count, not one per phone.
        fn plug(serial: Option<&str>, endpoints: bool, respond: Box<dyn Respond>) -> Plugged {
            let product = format!("OmniBuild {}", PHONES.fetch_add(1, Ordering::Relaxed));
            let descriptors = descriptors(serial, &product, endpoints);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let kernel = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (socket, _) = listener.accept().unwrap();
            let port = attach(&kernel);
            let answering = {
                let socket = socket.try_clone().unwrap();
                thread::spawn(move || answer(socket, &descriptors, respond))
            };
            // Once the kernel has enumerated it, and udev let this user
            // open it.
            let device = (0..1000)
                .find_map(|_| {
                    thread::sleep(Duration::from_millis(10));
                    usable(&product)
                })
                .expect("the phone shows up");
            // The bus, and the port on vhci-hcd's root hub, which counts
            // from 1.
            let name = serial.map_or_else(
                || format!("usb:{:03}-{}", device.busnum(), port + 1),
                str::to_owned,
            );
            Plugged {
                name,
                socket,
                answering,
            }
        }

        /// Attaches the device at the other end of `kernel` to a free
        /// high-speed port. Returns the port.
        fn attach(kernel: &TcpStream) -> u32 {
            let _attaching = ATTACHING.lock().unwrap();
            // A line per port: its hub, its number and its state, which is
            // 4, `VDEV_ST_NULL`, when it's free, then the rest.
            let status = fs::read_to_string(format!("{VHCI}/status")).unwrap();
            let port: u32 = status
                .lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>())
                .find(|fields| fields[0] == "hs" && fields[2] == "004")
                .expect("a free port")[1]
                .parse()
                .unwrap();
            // The port, the socket, an ID for the device, and its speed.
            let attach = format!("{port} {} {} {HIGH_SPEED}", kernel.as_raw_fd(), port + 1);
            fs::write(format!("{VHCI}/attach"), attach).unwrap();
            port
        }

        /// The phone named `product`, once it's listed and this user can
        /// open it.
        fn usable(product: &str) -> Option<DeviceInfo> {
            nusb::list_devices()
                .wait()
                .unwrap()
                .find(|device| device.product_string() == Some(product))
                .filter(|device| {
                    let node = format!(
                        "/dev/bus/usb/{:03}/{:03}",
                        device.busnum(),
                        device.device_address()
                    );
                    OpenOptions::new().read(true).write(true).open(node).is_ok()
                })
        }

        /// A phone's descriptors, by type and index: one configuration
        /// with two interfaces, another function such as MTP, then ADB's.
        fn descriptors(
            serial: Option<&str>,
            product: &str,
            endpoints: bool,
        ) -> Vec<((u8, u8), Vec<u8>)> {
            let [vendor_low, vendor_high] = GOOGLE.to_le_bytes();
            // USB 2.0, each interface of its own class, 64-byte packets on
            // endpoint 0, a Pixel's product ID with ADB, version 1.0, its
            // strings, and one configuration.
            let device = vec![
                18,
                1,
                0x00,
                0x02,
                0,
                0,
                0,
                64,
                vendor_low,
                vendor_high,
                0xe7,
                0x4e,
                0x00,
                0x01,
                1,
                2,
                u8::from(serial.is_some()) * 3,
                1,
            ];
            // Bulk, out and in, of 512-byte packets, high speed's.
            let bulk: &[u8] = &[7, 5, 0x01, 2, 0, 2, 0, 7, 5, 0x81, 2, 0, 2, 0];
            let interfaces = [
                &[9, 4, 0, 0, 0, 0xff, 0xff, 0, 0][..],
                &[9, 4, 1, 0, u8::from(endpoints) * 2, 0xff, 0x42, 1, 0],
                if endpoints { bulk } else { &[] },
            ]
            .concat();
            let len = u8::try_from(9 + interfaces.len()).unwrap();
            // Two interfaces, bus-powered, drawing 500 mA.
            let configuration = [&[9, 2, len, 0, 2, 1, 0, 0x80, 250][..], &interfaces].concat();
            let mut descriptors = vec![
                ((1, 0), device),
                ((2, 0), configuration),
                // The languages of its strings: American English.
                ((3, 0), vec![4, 3, 0x09, 0x04]),
                ((3, 1), string("Google")),
                ((3, 2), string(product)),
            ];
            descriptors.extend(serial.map(|serial| ((3, 3), string(serial))));
            descriptors
        }

        /// A string descriptor: its length, its type, and the string in
        /// UTF-16.
        fn string(string: &str) -> Vec<u8> {
            let units: Vec<u8> = string.encode_utf16().flat_map(u16::to_le_bytes).collect();
            [&[u8::try_from(2 + units.len()).unwrap(), 3], &units[..]].concat()
        }

        /// Answers the kernel on `socket` as a phone with `descriptors`
        /// whose adbd answers as `respond` says, until the socket closes.
        /// Returns the length of each transfer to its ADB interface.
        fn answer(
            mut socket: TcpStream,
            descriptors: &[((u8, u8), Vec<u8>)],
            mut respond: Box<dyn Respond>,
        ) -> Vec<usize> {
            let mut sent = Vec::new();
            let mut incoming = Vec::new();
            let mut outgoing = Transfers::default();
            // The kernel's reads from the phone, which wait for something
            // to read.
            let mut reads = VecDeque::new();
            let mut header = [0; 48];
            while socket.read_exact(&mut header).is_ok() {
                let word = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap());
                let (seqnum, endpoint, len) = (word(4), word(16), word(24) as usize);
                let response = match (word(0), endpoint, word(12)) {
                    // A read OmniBuild gave up on.
                    (CMD_UNLINK, ..) => {
                        reads.retain(|&read| read != word(20));
                        reply(RET_UNLINK, seqnum, CANCELLED, &[])
                    }
                    // The kernel enumerating the phone, which asks for a
                    // descriptor by its type and index, and how much of it.
                    (_, 0, IN) => {
                        let setup = &header[40..48];
                        let descriptor = descriptors
                            .iter()
                            .find(|(key, _)| *key == (setup[3], setup[2]))
                            .map(|(_, descriptor)| descriptor.clone())
                            .unwrap_or_default();
                        let len = descriptor.len().min(usize::from(setup[6]));
                        reply(RET_SUBMIT, seqnum, 0, &descriptor[..len])
                    }
                    (_, _, IN) => {
                        reads.push_back(seqnum);
                        Vec::new()
                    }
                    // SET_CONFIGURATION.
                    (_, 0, _) => reply(RET_SUBMIT, seqnum, 0, &[]),
                    // A transfer to adbd, which answers each message once
                    // it's whole.
                    _ => {
                        let mut data = vec![0; len];
                        socket.read_exact(&mut data).unwrap();
                        sent.push(len);
                        incoming.extend(data);
                        while let Some(message) = take_message(&mut incoming) {
                            for response in respond(&message).into_iter().flatten() {
                                write(&mut outgoing, &response).unwrap();
                            }
                        }
                        // How much it took, which an OUT transfer's answer
                        // says where an IN one's has the data.
                        let mut taken = reply(RET_SUBMIT, seqnum, 0, &[]);
                        taken[24..28].copy_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
                        taken
                    }
                };
                socket.write_all(&response).unwrap();
                // Each read gets a transfer of what adbd has to say.
                let answerable = reads.len().min(outgoing.done.len());
                let transfers = outgoing.done.drain(..answerable);
                for (read, transfer) in reads.drain(..answerable).zip(transfers) {
                    socket
                        .write_all(&reply(RET_SUBMIT, read, 0, &transfer))
                        .unwrap();
                }
            }
            sent
        }

        /// An answer to the kernel: the command, the request's sequence
        /// number, a status, the length of `data` and `data`, which
        /// `RET_SUBMIT` and `RET_UNLINK` both lay out so.
        fn reply(command: u32, seqnum: u32, status: i32, data: &[u8]) -> Vec<u8> {
            let len = u32::try_from(data.len()).unwrap();
            let words = [
                command,
                seqnum,
                0,
                0,
                0,
                status.cast_unsigned(),
                len,
                0,
                0,
                0,
                0,
                0,
            ];
            let header: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
            [&header[..], data].concat()
        }

        /// What adbd sends, as transfers: what it writes between flushes,
        /// as OmniBuild's are.
        #[derive(Default)]
        struct Transfers {
            writing: Vec<u8>,
            done: VecDeque<Vec<u8>>,
        }

        impl Write for Transfers {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.writing.extend(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.done.push_back(std::mem::take(&mut self.writing));
                Ok(())
            }
        }

        /// A phone that answers nothing.
        fn silent() -> Box<dyn Respond> {
            Box::new(|_: &Message| Some(Vec::new()))
        }

        /// Connects to the phone named `name` as `install` does. Says
        /// whether it asked its user to allow the key.
        fn connect(name: &str) -> (Device, bool) {
            let asked = Cell::new(false);
            let device =
                Device::connect(Some(name), &[], &Usb, &KEY, &|_| asked.set(true)).unwrap();
            (device, asked.get())
        }

        #[test]
        fn installs_on_a_phone_plugged_in() {
            let phone = plug(
                Some("OMNIBUILD0001"),
                true,
                Box::new(asking(exec_device(4096, BANNER, Some(b"Success\n")))),
            );

            let names = Usb.names();
            let (mut device, asked) = connect(&phone.name);
            device.install(&[7; 5000]).unwrap();
            drop(device);
            let sent = phone.unplug();

            assert!(names.contains(&"OMNIBUILD0001".to_owned()), "{names:?}");
            assert!(asked);
            // Each header, then its payload, each a transfer of its own: a
            // payload of whole packets ends with a zero-length one, for
            // adbd to see where it ends. The CNXN, the signature and the
            // key, the OPEN, two WRTEs, and the OKAY for the output.
            let key = KEY.public_key().len() + 1;
            let open = "exec:cmd package install -r -S 5000\0".len();
            assert_eq!(
                sent,
                [24, 7, 24, 256, 24, key, 24, open, 24, 4096, 0, 24, 904, 24]
            );
        }

        #[test]
        fn names_a_phone_without_a_serial_number_by_where_its_plugged_in() {
            let phone = plug(
                None,
                true,
                Box::new(asking(exec_device(4096, BANNER, Some(b"Success\n")))),
            );

            let names = Usb.names();
            let (device, _) = connect(&phone.name);

            assert!(names.contains(&phone.name), "{} in {names:?}", phone.name);
            assert_eq!(device.serial(), phone.name);
            drop(device);
            phone.unplug();
        }

        #[test]
        fn says_when_another_program_has_the_phone() {
            let phone = plug(Some("OMNIBUILD0002"), true, silent());

            let first = Usb.open(&phone.name, TIMEOUT).unwrap();
            let second = Usb.open(&phone.name, TIMEOUT).err().unwrap();

            assert!(
                format!("{second:#}").starts_with(
                    "another program is using it, such as adb's server: stop that with adb kill-server: "
                ),
                "{second:#}"
            );
            drop(first);
            phone.unplug();
        }

        #[test]
        fn says_when_a_phones_adb_interface_has_no_endpoints() {
            let phone = plug(Some("OMNIBUILD0003"), false, silent());

            let error = Usb.open(&phone.name, TIMEOUT).err().unwrap();

            assert_eq!(error.to_string(), "its ADB interface has no bulk endpoints");
            phone.unplug();
        }

        #[test]
        fn gives_up_on_a_phone_that_stops_answering() {
            let phone = plug(Some("OMNIBUILD0004"), true, silent());

            let stream = Usb.open(&phone.name, Duration::from_millis(200)).unwrap();
            // Closing the phone cancels the read it never answered, and the
            // kernel waits for the phone to say it's cancelled.
            let error = Connection::open(stream, &KEY, &unasked).err().unwrap();

            assert_eq!(error.to_string(), "the device stopped answering");
            assert_eq!(phone.unplug(), [24, 7]);
        }
    }
}
