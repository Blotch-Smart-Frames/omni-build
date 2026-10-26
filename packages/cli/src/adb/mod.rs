//! Installing apps on Android devices with no Android SDK, over ADB, the
//! protocol the SDK's adb speaks to them: to phones plugged in by USB, and
//! over TCP to the emulators running on this computer and to devices on
//! the network, which a phone plugged in becomes when it's asked to listen
//! there.

mod connection;
mod key;
mod pairing;
mod spake2;
pub mod tls;
mod usb;

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use connection::{Connection, Stream};
pub use key::{AdbKey, key_path, owner};
pub use usb::Usb;

/// How long OmniBuild waits for a device: long enough for its user to
/// allow OmniBuild's key, and for it to check and compile a large app.
const TIMEOUT: Duration = Duration::from_secs(120);
/// How long OmniBuild waits to connect to a port on this computer, where
/// nothing listening refuses at once.
const LOCAL_TIMEOUT: Duration = Duration::from_millis(500);
/// Where an APK goes on a device before Android 7.0, for the package
/// manager to install it from: a folder adb's shell can write to and the
/// package manager read, where adb puts it too.
const PUSHED_APK: &str = "/data/local/tmp/omni-build.apk";
/// The pushed APK's mode, as `adb push` sends it: a regular file anyone may
/// read, so the package manager can.
const PUSHED_MODE: u32 = 0o100_644;
/// The most a sync DATA request carries.
const SYNC_DATA_MAX: usize = 64 * 1024;
/// How long OmniBuild waits for a device it has asked to listen on the
/// network to be listening there: its adbd restarts to listen, which takes
/// a second or two, and an old phone longer.
pub const LISTEN_TIMEOUT: Duration = Duration::from_secs(15);
/// How long each attempt to connect to a device on the network waits. A
/// network that keeps its devices apart drops the attempt rather than
/// refusing it, which the system would otherwise wait on for a minute or
/// more.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long OmniBuild waits between attempts, while the device's adbd
/// restarts and refuses connections.
const RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// The ports where emulators on this computer listen for ADB, and where
/// adb looks for them: each emulator takes two ports from 5554 on, its
/// console's and then ADB's, so the odd ones from 5555 to 5585.
pub fn emulator_ports() -> Vec<u16> {
    (5555..=5585).step_by(2).collect()
}

/// The devices plugged into this computer's USB ports with USB debugging
/// on: `Usb`, or phones a test makes up.
pub trait UsbDevices {
    /// Their names, as adb's: each one's serial number, or where it's
    /// plugged in when it has none.
    fn names(&self) -> Vec<String>;
    /// Opens ADB's interface on the one named `name`, waiting up to
    /// `timeout` for each read and write.
    fn open(&self, name: &str, timeout: Duration) -> Result<Box<dyn Stream>>;
}

/// How OmniBuild reaches a device.
enum Route {
    /// Over TCP, connected: a device on the network.
    Tcp(TcpStream),
    /// Over TCP, connected: an emulator on this computer.
    Emulator(TcpStream),
    /// Plugged in by USB, not yet opened.
    Usb,
}

/// A device OmniBuild has connected to.
pub struct Device {
    /// Its name, as adb's: a phone's serial number, `emulator-5554`, or
    /// its address.
    serial: String,
    /// Whether it's an emulator on this computer.
    emulator: bool,
    connection: Connection<'static>,
}

impl Device {
    /// Connects to `device`, the name of a device in `usb` or of an
    /// emulator, or a device's address and port, or without one to the
    /// only device there is, in `usb` or listening on one of
    /// `emulator_ports`, and proves OmniBuild's identity with `key`.
    /// Calls `authorizing` with the device's name when the device asks its
    /// user whether to allow the key, and waits for the answer.
    pub fn connect(
        device: Option<&str>,
        emulator_ports: &[u16],
        usb: &dyn UsbDevices,
        key: &AdbKey,
        authorizing: &dyn Fn(&str),
    ) -> Result<Self> {
        let (serial, route) = match device {
            Some(device) => (device.to_owned(), reach(device, usb)?),
            None => only_device(emulator_ports, usb)?,
        };
        let emulator = matches!(route, Route::Emulator(_));
        let stream = match route {
            Route::Tcp(stream) | Route::Emulator(stream) => tcp(stream),
            Route::Usb => usb.open(&serial, TIMEOUT),
        };
        let connection = stream
            .and_then(|stream| Connection::open(stream, key, &|| authorizing(&serial)))
            .with_context(|| format!("couldn't connect to {serial}"))?;
        Ok(Self {
            serial,
            emulator,
            connection,
        })
    }

    /// Its name, as adb's: a phone's serial number, `emulator-5554`, or
    /// its address.
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Its model, as its user knows it, if it says.
    pub fn model(&self) -> Option<&str> {
        self.connection.property("ro.product.model")
    }

    /// Installs an APK, or updates the app with its ID, as `adb install`
    /// does: from Android 7.0 on, streamed to the package manager's install
    /// command, which the device's `cmd` runs, and before, pushed to the
    /// device for `pm` to install.
    pub fn install(&mut self, apk: &[u8]) -> Result<()> {
        let output = if self.connection.has_feature("cmd") {
            self.connection.run(
                &format!("exec:cmd package install -r -S {}", apk.len()),
                apk,
            )
        } else {
            self.push_and_install(apk)
        }
        .with_context(|| format!("couldn't install the app on {self}"))?;
        // "Success", or "Failure [INSTALL_FAILED_...: why]".
        let output = String::from_utf8_lossy(&output);
        if output.lines().any(|line| line.starts_with("Success")) {
            return Ok(());
        }
        // pm, before Android 7.0, names the file it installs first.
        let reason = output
            .lines()
            .find(|line| line.starts_with("Failure"))
            .unwrap_or(output.trim());
        // What a developer meets first: the device has the app as Gradle
        // built it, signed with the Android SDK's debug key, not
        // OmniBuild's.
        let hint = if reason.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
            ". The app on the device is signed with another key, and Android only updates an app signed with the same one: uninstall it from the device first"
        } else {
            ""
        };
        bail!(
            "{self} didn't install the app: {}{hint}",
            if reason.is_empty() {
                "it gave no reason"
            } else {
                reason
            }
        );
    }

    /// Installs the APK as adb does before Android 7.0, where `cmd` isn't:
    /// pushes it to a file on the device, which `pm` installs, then deletes
    /// the file. Returns what `pm` said.
    fn push_and_install(&mut self, apk: &[u8]) -> Result<Vec<u8>> {
        self.push(PUSHED_APK, apk)?;
        let output = self
            .connection
            .run(&format!("shell:pm install -r {PUSHED_APK}"), &[])?;
        self.connection
            .run(&format!("shell:rm -f {PUSHED_APK}"), &[])?;
        Ok(output)
    }

    /// Has the device listen for ADB on the network, on `port`, as `adb
    /// tcpip` does, so OmniBuild reaches it there without its cable until it
    /// restarts. Reads the device's address, asks its adbd to restart
    /// listening on the port, which ends this connection, then connects to
    /// it there within `patience`, proving OmniBuild's identity with `key`
    /// again and calling `authorizing` with the address if the device asks
    /// its user. Returns the address.
    pub fn listen(
        &mut self,
        port: u16,
        key: &AdbKey,
        authorizing: &dyn Fn(&str),
        patience: Duration,
    ) -> Result<SocketAddr> {
        // An emulator listens on a network of its own, inside the emulator,
        // which this computer reaches only through the ports the emulator
        // forwards to it: as `emulator-5554`, already.
        if self.emulator {
            bail!(
                "{self} is an emulator, which this computer reaches over the network already: name it with --device {}",
                self.serial
            );
        }
        // Before the switch, which ends the connection to ask on.
        let addresses = self
            .connection
            .run("shell:ip -o -4 addr show", &[])
            .with_context(|| format!("couldn't read the address of {self}"))?;
        let Some(ip) = network_address(&String::from_utf8_lossy(&addresses)) else {
            bail!(
                "{self} isn't on Wi-Fi or Ethernet: connect it to the network this computer is on"
            );
        };
        // "restarting in TCP mode port: 5555", and then adbd restarts.
        let answer = self
            .connection
            .run(&format!("tcpip:{port}"), &[])
            .with_context(|| format!("couldn't ask {self} to listen on the network"))?;
        let answer = String::from_utf8_lossy(&answer);
        if !answer.starts_with("restarting in TCP mode") {
            bail!(
                "{self} didn't listen on the network: {}",
                match answer.trim() {
                    "" => "it gave no reason",
                    reason => reason,
                }
            );
        }
        let address = SocketAddr::from((ip, port));
        let stream = wait_for(address, patience).map_err(|error| {
            anyhow!(
                "{self} listens for ADB at {address} now, but this computer couldn't reach it there ({error}): it may be on another network, or on one that keeps its devices apart, as many guest networks do"
            )
        })?;
        // The device trusts the key over USB, but asks its user again if
        // they allowed it only that once.
        let name = address.to_string();
        tcp(stream)
            .and_then(|stream| Connection::open(stream, key, &|| authorizing(&name)))
            .with_context(|| format!("couldn't connect to {name}"))?;
        Ok(address)
    }

    /// Writes `contents` to the file at `path` on the device, as `adb push`
    /// does, through the device's sync service: requests of an ID and a
    /// length, the file's path and mode first, then its contents in pieces,
    /// then its time, to which the device answers OKAY or FAIL and why, and
    /// then one that ends the session.
    fn push(&mut self, path: &str, contents: &[u8]) -> Result<()> {
        let request = |id: &[u8; 4], data: &[u8]| {
            let len = u32::try_from(data.len()).expect("a request is under 4 GiB");
            [&id[..], &len.to_le_bytes(), data].concat()
        };
        let mut requests = request(b"SEND", format!("{path},{PUSHED_MODE}").as_bytes());
        for piece in contents.chunks(SYNC_DATA_MAX) {
            requests.extend(request(b"DATA", piece));
        }
        // DONE's length is the file's time, which doesn't matter for a file
        // deleted once it's installed.
        requests.extend(request(b"DONE", &[]));
        requests.extend(request(b"QUIT", &[]));
        let reply = self.connection.run("sync:", &requests)?;
        match reply.get(..4) {
            Some(b"OKAY") => Ok(()),
            Some(b"FAIL") => bail!(
                "the device couldn't save the app to {path}: {}",
                String::from_utf8_lossy(reply.get(8..).unwrap_or_default())
            ),
            _ => bail!("the device didn't take the app"),
        }
    }
}

impl fmt::Display for Device {
    /// Its model, as its user knows it, then its name: "Pixel 8
    /// (emulator-5554)", or "Pixel 8 (R5CT1234)" for a phone plugged
    /// in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.model() {
            Some(model) => write!(f, "{model} ({})", self.serial),
            None => f.write_str(&self.serial),
        }
    }
}

/// Pairs this computer with a device for wireless debugging, as `adb pair`
/// does, so that the device trusts `key` from then on: at `address`, where
/// the device listens for pairing while it shows `code`. Returns the
/// device's GUID, its name for itself.
pub fn pair(address: &str, code: &str, key: &AdbKey) -> Result<String> {
    TcpStream::connect(address)
        .map_err(anyhow::Error::from)
        .and_then(tcp)
        .and_then(|stream| pairing::pair(stream, key, code))
        .with_context(|| format!("couldn't pair with {address}"))
}

/// A TCP connection to a device, set up as adb sets one up.
fn tcp(stream: TcpStream) -> Result<Box<dyn Stream>> {
    // Without Nagle's algorithm, as adb: each small message would
    // otherwise wait for the device to acknowledge the last.
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .and_then(|()| stream.set_nodelay(true))
        .map(|()| Box::new(stream) as Box<dyn Stream>)
        .map_err(anyhow::Error::from)
}

/// Connects to `address` once something listens there, trying until
/// `patience` runs out: a device's adbd refuses connections while it
/// restarts to listen on the network.
fn wait_for(address: SocketAddr, patience: Duration) -> io::Result<TcpStream> {
    let deadline = Instant::now() + patience;
    loop {
        match TcpStream::connect_timeout(&address, ATTEMPT_TIMEOUT) {
            Err(_) if Instant::now() < deadline => std::thread::sleep(RETRY_INTERVAL),
            result => return result,
        }
    }
}

/// The device's address on a network this computer may share with it, from
/// what `ip -o -4 addr show` prints, a line for each address, such as `30:
/// wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global wlan0`: its
/// address on Wi-Fi, or else on Ethernet. Not one on mobile data
/// (`rmnet_data0`, `ccmni0`), a VPN's tunnel or loopback, where no computer
/// nearby reaches it.
fn network_address(ip: &str) -> Option<Ipv4Addr> {
    let addresses: Vec<(&str, Ipv4Addr)> = ip
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let interface = words.nth(1)?;
            let address = words.skip_while(|word| *word != "inet").nth(1)?;
            // With its prefix's length, the size of its network.
            let address = address
                .split_once('/')
                .map_or(address, |(address, _)| address);
            Some((interface, address.parse().ok()?))
        })
        .collect();
    ["wlan", "eth"].iter().find_map(|kind| {
        addresses
            .iter()
            .find(|(interface, _)| interface.starts_with(kind))
            .map(|&(_, address)| address)
    })
}

/// Reaches a device by its name: an emulator's, `emulator-<console
/// port>`, which listens for ADB on the next port, a device's address and
/// port, or a phone's serial number, the name adb gives it.
fn reach(device: &str, usb: &dyn UsbDevices) -> Result<Route> {
    let emulator = device
        .strip_prefix("emulator-")
        .and_then(|port| port.parse::<u16>().ok())
        .and_then(|port| port.checked_add(1));
    let route = match emulator {
        Some(port) => TcpStream::connect((Ipv4Addr::LOCALHOST, port)).map(Route::Emulator),
        // Before addresses: a phone without a serial number is named by
        // where it's plugged in, such as `usb:001-3`.
        None if usb.names().iter().any(|name| name == device) => return Ok(Route::Usb),
        None if device.contains(':') => TcpStream::connect(device).map(Route::Tcp),
        None => bail!(
            "found no device named \"{device}\": name a phone plugged in by USB, with USB debugging on, by its serial number, as adb devices shows it; an emulator, such as emulator-5554; or a device on the network by its address and port, such as 192.168.1.20:5555"
        ),
    };
    route.with_context(|| format!("couldn't reach {device}"))
}

/// The only device there is: a phone in `usb`, or an emulator listening on
/// one of `ports` on this computer, with the name adb gives it.
fn only_device(ports: &[u16], usb: &dyn UsbDevices) -> Result<(String, Route)> {
    let phones = usb.names().into_iter().map(|name| (name, Route::Usb));
    let mut devices: Vec<_> = phones
        .chain(
            running_emulators(ports)
                .into_iter()
                .map(|(name, stream)| (name, Route::Emulator(stream))),
        )
        .collect();
    match devices.len() {
        0 => bail!(
            "found no Android device: plug in a phone with USB debugging on, start an emulator, or name a device on the network with --device <address>:<port>"
        ),
        1 => Ok(devices.remove(0)),
        count => {
            let names: Vec<_> = devices.iter().map(|(name, _)| name.as_str()).collect();
            bail!(
                "found {count} devices ({}): name one with --device",
                names.join(", ")
            )
        }
    }
}

/// The emulators listening on one of `ports` on this computer, with the
/// names adb gives them.
fn running_emulators(ports: &[u16]) -> Vec<(String, TcpStream)> {
    // All at once: Windows takes a second or two to refuse a connection
    // to a port nothing listens on, where other systems refuse at once.
    std::thread::scope(|scope| {
        let attempts: Vec<_> = ports
            .iter()
            .map(|&port| {
                scope.spawn(move || {
                    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                    let stream = TcpStream::connect_timeout(&address, LOCAL_TIMEOUT).ok()?;
                    Some((format!("emulator-{}", port - 1), stream))
                })
            })
            .collect();
        attempts
            .into_iter()
            .filter_map(|attempt| attempt.join().expect("connecting doesn't panic"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::Duration;

    use anyhow::Context;

    use super::connection::Stream;
    use super::connection::tests::{
        Answer, BANNER, Respond, asking, exec_device, serve, service_device,
    };
    use super::pairing::tests::{Stray, device};
    use super::{AdbKey, Device, TIMEOUT, UsbDevices, emulator_ports, network_address, pair};

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb/adbkey");
    static KEY: LazyLock<AdbKey> =
        LazyLock::new(|| AdbKey::at(Path::new(FIXTURE), "omnibuild@test").unwrap().0);

    thread_local! {
        /// The devices that asked their user to allow the key, by name, in
        /// this thread's test.
        static ASKED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// Phones plugged in by USB, made up: each a name, and the port of a
    /// device on this computer that stands in for it, or none for a phone
    /// the system won't open.
    struct Phones(Vec<(&'static str, Option<u16>)>);

    impl UsbDevices for Phones {
        fn names(&self) -> Vec<String> {
            self.0.iter().map(|(name, _)| (*name).to_owned()).collect()
        }

        fn open(&self, name: &str, timeout: Duration) -> anyhow::Result<Box<dyn Stream>> {
            assert_eq!(timeout, TIMEOUT);
            let (_, port) = self.0.iter().find(|(phone, _)| *phone == name).unwrap();
            let port = port.context("another program is using it")?;
            Ok(Box::new(TcpStream::connect(("127.0.0.1", port)).unwrap()))
        }
    }

    /// Connects as `install` does, with no phone plugged in.
    fn connect(device: Option<&str>, emulator_ports: &[u16]) -> anyhow::Result<Device> {
        connect_with(device, emulator_ports, &Phones(Vec::new()))
    }

    /// Connects as `install` does, with the fixture key and `phones`
    /// plugged in, noting each device that asks its user to allow the key
    /// in `ASKED`.
    fn connect_with(
        device: Option<&str>,
        emulator_ports: &[u16],
        phones: &Phones,
    ) -> anyhow::Result<Device> {
        Device::connect(device, emulator_ports, phones, &KEY, &asking_user)
    }

    /// Notes in `ASKED` that the device named `name` asks its user to allow
    /// the key.
    fn asking_user(name: &str) {
        ASKED.with_borrow_mut(|asked| asked.push(name.to_owned()));
    }

    /// A port on this computer with something listening, and its number.
    fn listening() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    /// A port on this computer with nothing listening.
    fn closed() -> u16 {
        listening().1
    }

    #[test]
    fn installs_on_a_device_at_an_address() {
        let (listener, port) = listening();
        let device = serve(listener, exec_device(4096, BANNER, Some(b"Success\n")));
        let apk = vec![7; 10_000];

        let mut connected = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();
        connected.install(&apk).unwrap();

        assert_eq!(connected.to_string(), format!("Pixel 8 (127.0.0.1:{port})"));
        assert_eq!(connected.serial(), format!("127.0.0.1:{port}"));
        assert_eq!(connected.model(), Some("Pixel 8"));
        // The device didn't check the key, let alone ask its user.
        assert_eq!(ASKED.take(), Vec::<String>::new());
        drop(connected);
        assert_eq!(
            device.join().unwrap(),
            ["CNXN", "OPEN", "WRTE", "WRTE", "WRTE", "OKAY"]
        );
    }

    #[test]
    fn finds_an_emulator_by_its_name() {
        let (listener, port) = listening();
        serve(
            listener,
            exec_device(4096, b"device::\0", Some(b"Success\n")),
        );
        let name = format!("emulator-{}", port - 1);

        let connected = connect(Some(&name), &[]).unwrap();

        // A banner without the model, as early versions of Android send.
        assert_eq!(connected.to_string(), name);
    }

    #[test]
    fn says_when_the_device_asks_its_user_to_allow_the_key() {
        let (listener, port) = listening();
        serve(
            listener,
            asking(exec_device(4096, BANNER, Some(b"Success\n"))),
        );
        let name = format!("emulator-{}", port - 1);

        let mut connected = connect(Some(&name), &[]).unwrap();
        connected.install(&[7; 100]).unwrap();

        assert_eq!(ASKED.take(), [name]);
    }

    #[test]
    fn finds_the_only_emulator_running() {
        let (listener, port) = listening();
        serve(listener, exec_device(4096, BANNER, Some(b"Success\n")));

        let connected = connect(None, &[closed(), port]).unwrap();

        assert_eq!(
            connected.to_string(),
            format!("Pixel 8 (emulator-{})", port - 1)
        );
    }

    #[test]
    fn asks_which_device_when_there_are_several() {
        let ((_first, one), (_second, two)) = (listening(), listening());
        let phone = Phones(vec![("R5CT1234", None)]);

        let emulators = connect(None, &[one, two]).err().unwrap();
        let phone_and_emulator = connect_with(None, &[one], &phone).err().unwrap();

        assert_eq!(
            emulators.to_string(),
            format!(
                "found 2 devices (emulator-{}, emulator-{}): name one with --device",
                one - 1,
                two - 1
            )
        );
        assert_eq!(
            phone_and_emulator.to_string(),
            format!(
                "found 2 devices (R5CT1234, emulator-{}): name one with --device",
                one - 1
            )
        );
    }

    #[test]
    fn installs_on_the_only_phone_plugged_in() {
        let (listener, port) = listening();
        let device = serve(listener, exec_device(4096, BANNER, Some(b"Success\n")));
        let phones = Phones(vec![("R5CT1234", Some(port))]);

        let mut connected = connect_with(None, &[closed()], &phones).unwrap();
        connected.install(&[7; 100]).unwrap();

        assert_eq!(connected.to_string(), "Pixel 8 (R5CT1234)");
        drop(connected);
        assert_eq!(device.join().unwrap(), ["CNXN", "OPEN", "WRTE", "OKAY"]);
    }

    #[test]
    fn finds_a_phone_by_its_name() {
        let (first, one) = listening();
        let (second, two) = listening();
        serve(first, exec_device(4096, BANNER, Some(b"Success\n")));
        serve(second, exec_device(4096, BANNER, Some(b"Success\n")));
        // A phone without a serial number, which is named by where it's
        // plugged in, and looks like an address.
        let phones = Phones(vec![("R5CT1234", Some(one)), ("usb:1-3", Some(two))]);

        let serial = connect_with(Some("R5CT1234"), &[], &phones).unwrap();
        let port = connect_with(Some("usb:1-3"), &[], &phones).unwrap();

        assert_eq!(serial.serial(), "R5CT1234");
        assert_eq!(port.serial(), "usb:1-3");
    }

    #[test]
    fn says_why_it_couldnt_open_a_phone() {
        let phones = Phones(vec![("R5CT1234", None)]);

        let error = connect_with(Some("R5CT1234"), &[], &phones).err().unwrap();

        assert_eq!(
            format!("{error:#}"),
            "couldn't connect to R5CT1234: another program is using it"
        );
    }

    #[test]
    fn says_when_it_finds_no_device() {
        let error = connect(None, &[closed()]).err().unwrap();

        assert!(error.to_string().starts_with("found no Android device"));
    }

    #[test]
    fn says_why_it_couldnt_reach_or_connect_to_a_device() {
        let port = closed();
        let (listener, open) = listening();
        // A device that hangs up on OmniBuild's first message, once it has
        // read it: hanging up on unread bytes would reset the connection.
        serve(listener, |_| None);

        let refused = connect(Some(&format!("emulator-{}", port - 1)), &[]);
        let unknown = connect(Some("R5CT1234"), &[]);
        let out_of_range = connect(Some("emulator-65535"), &[]);
        let hung_up = connect(Some(&format!("127.0.0.1:{open}")), &[]);

        let message = |result: anyhow::Result<Device>| format!("{:#}", result.err().unwrap());
        assert!(message(refused).starts_with(&format!("couldn't reach emulator-{}: ", port - 1)));
        assert!(message(unknown).starts_with("found no device named \"R5CT1234\": name a phone"));
        assert!(message(out_of_range).starts_with("found no device named \"emulator-65535\""));
        assert_eq!(
            message(hung_up),
            format!("couldn't connect to 127.0.0.1:{open}: the device closed the connection")
        );
    }

    #[test]
    fn says_why_the_device_didnt_install_the_app() {
        let install = |banner: &'static [u8], output: &'static [u8]| {
            let (listener, port) = listening();
            serve(listener, exec_device(4096, banner, Some(output)));
            let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();
            let error = device.install(b"apk").err().unwrap();
            format!("{error:#}").replace(&port.to_string(), "PORT")
        };

        assert_eq!(
            install(
                BANNER,
                b"Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match]\n"
            ),
            "Pixel 8 (127.0.0.1:PORT) didn't install the app: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match]. The app on the device is signed with another key, and Android only updates an app signed with the same one: uninstall it from the device first"
        );
        assert_eq!(
            install(
                BANNER,
                b"Failure [INSTALL_FAILED_OLDER_SDK: Requires newer sdk version #37]\n"
            ),
            "Pixel 8 (127.0.0.1:PORT) didn't install the app: Failure [INSTALL_FAILED_OLDER_SDK: Requires newer sdk version #37]"
        );
        assert_eq!(
            install(BANNER, b""),
            "Pixel 8 (127.0.0.1:PORT) didn't install the app: it gave no reason"
        );
    }

    /// A banner as a device before Android 7.0 sends it: with no features,
    /// so no `cmd`.
    const LEGACY_BANNER: &[u8] =
        b"device::ro.product.name=hammerhead;ro.product.model=Nexus 5;ro.product.device=hammerhead;\0";
    const PM_INSTALL: &str = "shell:pm install -r /data/local/tmp/omni-build.apk";
    const RM: &str = "shell:rm -f /data/local/tmp/omni-build.apk";
    const SYNC_OKAY: &[u8] = b"OKAY\0\0\0\0";

    /// The services a device was asked for, each with what was written to
    /// it.
    type Log = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    /// What a device before Android 7.0 answers a service with: its
    /// output, or with none, it hangs up.
    type Answers = &'static [(&'static str, Option<&'static [u8]>)];

    /// A device before Android 7.0, which takes writes of 4 KiB at most, as
    /// those versions do, and answers each service as `answers` says, or
    /// with nothing: a sync session once it's been ended. Logs each service
    /// it answers in `log`, with what was written to it.
    fn legacy_device(answers: Answers, log: &Log) -> impl Respond {
        let log = Arc::clone(log);
        service_device(4096, LEGACY_BANNER, move |service, input| {
            let answer = answers
                .iter()
                .find(|(name, _)| *name == service)
                .map_or(Some(&b""[..]), |(_, answer)| *answer);
            let Some(output) = answer else {
                return Answer::HangUp;
            };
            if service == "sync:" && !input.ends_with(b"QUIT\0\0\0\0") {
                return Answer::Waiting;
            }
            log.lock()
                .unwrap()
                .push((service.to_owned(), input.to_vec()));
            Answer::Done(output.to_vec())
        })
    }

    /// The services the device was asked for.
    fn services(log: &Log) -> Vec<String> {
        log.lock()
            .unwrap()
            .iter()
            .map(|(service, _)| service.clone())
            .collect()
    }

    /// A sync request: its ID, the length of its data, and the data.
    fn request(id: &[u8], data: &[u8]) -> Vec<u8> {
        let len = u32::try_from(data.len()).unwrap();
        [id, &len.to_le_bytes(), data].concat()
    }

    #[test]
    fn installs_on_a_device_before_android_7() {
        let (listener, port) = listening();
        let log = Log::default();
        serve(
            listener,
            legacy_device(
                &[
                    ("sync:", Some(SYNC_OKAY)),
                    (
                        PM_INSTALL,
                        Some(b"\tpkg: /data/local/tmp/omni-build.apk\r\nSuccess\r\n"),
                    ),
                ],
                &log,
            ),
        );
        let apk: Vec<u8> = (0..100_000u32).map(|n| n.to_le_bytes()[0]).collect();

        let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();
        device.install(&apk).unwrap();

        assert_eq!(services(&log), ["sync:", PM_INSTALL, RM]);
        // The file's path and mode, its contents in pieces of 64 KiB, its
        // time, and the end of the session.
        let expected = [
            request(b"SEND", b"/data/local/tmp/omni-build.apk,33188"),
            request(b"DATA", &apk[..65_536]),
            request(b"DATA", &apk[65_536..]),
            request(b"DONE", b""),
            request(b"QUIT", b""),
        ]
        .concat();
        assert_eq!(log.lock().unwrap()[0].1, expected);
    }

    #[test]
    fn says_why_a_device_before_android_7_didnt_install_the_app() {
        let install = |answers: Answers| {
            let (listener, port) = listening();
            let log = Log::default();
            serve(listener, legacy_device(answers, &log));
            let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();
            let error = device.install(b"apk").err().unwrap();
            let error = format!("{error:#}").replace(&port.to_string(), "PORT");
            (error, services(&log))
        };

        assert_eq!(
            install(&[("sync:", Some(b"FAIL\x17\0\0\0No space left on device"))]),
            (
                "couldn't install the app on Nexus 5 (127.0.0.1:PORT): the device couldn't save the app to /data/local/tmp/omni-build.apk: No space left on device".to_owned(),
                vec!["sync:".to_owned()]
            )
        );
        assert_eq!(
            install(&[]).0,
            "couldn't install the app on Nexus 5 (127.0.0.1:PORT): the device didn't take the app"
        );
        // The file goes, installed or not.
        assert_eq!(
            install(&[
                ("sync:", Some(SYNC_OKAY)),
                (
                    PM_INSTALL,
                    Some(b"\tpkg: /data/local/tmp/omni-build.apk\r\nFailure [INSTALL_FAILED_OLDER_SDK]\r\n")
                ),
            ]),
            (
                "Nexus 5 (127.0.0.1:PORT) didn't install the app: Failure [INSTALL_FAILED_OLDER_SDK]".to_owned(),
                vec!["sync:".to_owned(), PM_INSTALL.to_owned(), RM.to_owned()]
            )
        );
    }

    #[test]
    fn says_when_a_device_before_android_7_goes_away_during_an_install() {
        // Hanging up on each service in turn.
        let answers: [Answers; 3] = [
            &[("sync:", None)],
            &[("sync:", Some(SYNC_OKAY)), (PM_INSTALL, None)],
            &[
                ("sync:", Some(SYNC_OKAY)),
                (PM_INSTALL, Some(b"Success\r\n")),
                (RM, None),
            ],
        ];
        for answers in answers {
            let (listener, port) = listening();
            serve(listener, legacy_device(answers, &Log::default()));
            let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();

            let error = device.install(b"apk").err().unwrap();

            assert_eq!(
                format!("{error:#}"),
                format!(
                    "couldn't install the app on Nexus 5 (127.0.0.1:{port}): the device closed the connection"
                )
            );
        }
    }

    #[test]
    fn says_when_the_device_goes_away_during_an_install() {
        let (listener, port) = listening();
        // A device that connects, then hangs up when the app comes.
        serve(listener, exec_device(4096, BANNER, None));
        let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();

        let error = device.install(b"apk").err().unwrap();

        assert_eq!(
            format!("{error:#}"),
            format!(
                "couldn't install the app on Pixel 8 (127.0.0.1:{port}): the device closed the connection"
            )
        );
    }

    #[test]
    fn looks_for_emulators_where_adb_does() {
        let ports = emulator_ports();

        assert_eq!(ports.len(), 16);
        assert_eq!((ports[0], ports[15]), (5555, 5585));
    }

    /// What `ip -o -4 addr show` prints on a phone on Wi-Fi and mobile data,
    /// where this computer's address is `127.0.0.1`, as a pseudo-terminal
    /// gives it before Android 7.0, with each line ending in `\r\n`.
    const ON_WI_FI: &[u8] = b"1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever preferred_lft forever\r\n\
        12: rmnet_data0    inet 10.141.7.9/30 scope global rmnet_data0\\       valid_lft forever preferred_lft forever\r\n\
        30: wlan0    inet 127.0.0.1/24 brd 127.0.0.255 scope global wlan0\\       valid_lft forever preferred_lft forever\r\n";

    /// What a phone answers each service with, by the start of its name:
    /// its output, or with none, it hangs up.
    type Services = Vec<(&'static str, Option<Vec<u8>>)>;

    /// A phone that answers services as `answers` says, and others with
    /// nothing.
    fn phone(answers: Services) -> impl Respond {
        service_device(4096, BANNER, move |service, _| {
            match answers.iter().find(|(name, _)| service.starts_with(name)) {
                Some((_, None)) => Answer::HangUp,
                Some((_, Some(output))) => Answer::Done(output.clone()),
                None => Answer::Done(Vec::new()),
            }
        })
    }

    /// A phone on Wi-Fi that restarts to listen on `port` when asked.
    fn restarting(port: u16) -> Services {
        vec![
            ("shell:ip", Some(ON_WI_FI.to_vec())),
            (
                "tcpip:",
                Some(format!("restarting in TCP mode port: {port}\n").into_bytes()),
            ),
        ]
    }

    /// Has the device at `port` listen on `new_port`, waiting a second for
    /// it, as `tcpip` does.
    fn listen(port: u16, new_port: u16) -> anyhow::Result<std::net::SocketAddr> {
        let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();
        device.listen(new_port, &KEY, &asking_user, Duration::from_secs(1))
    }

    #[test]
    fn has_a_phone_plugged_in_listen_on_the_network() {
        let (listener, port) = listening();
        let new_port = closed();
        let log = Log::default();
        let asked = Arc::clone(&log);
        // A phone whose adbd restarts when asked to listen on the network,
        // and listens there a moment later, where it asks its user to
        // allow the key again.
        serve(
            listener,
            service_device(4096, BANNER, move |service, input| {
                asked
                    .lock()
                    .unwrap()
                    .push((service.to_owned(), input.to_vec()));
                if service.starts_with("shell:ip") {
                    return Answer::Done(ON_WI_FI.to_vec());
                }
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(300));
                    let listener = TcpListener::bind(("127.0.0.1", new_port)).unwrap();
                    serve(
                        listener,
                        asking(service_device(4096, BANNER, |_, _| Answer::HangUp)),
                    );
                });
                Answer::Done(format!("restarting in TCP mode port: {new_port}\n").into_bytes())
            }),
        );
        let phones = Phones(vec![("R5CT1234", Some(port))]);
        let mut phone = connect_with(None, &[closed()], &phones).unwrap();

        let address = phone
            .listen(new_port, &KEY, &asking_user, Duration::from_secs(5))
            .unwrap();

        assert_eq!(address.to_string(), format!("127.0.0.1:{new_port}"));
        assert_eq!(ASKED.take(), [address.to_string()]);
        assert_eq!(
            services(&log),
            [
                "shell:ip -o -4 addr show".to_owned(),
                format!("tcpip:{new_port}")
            ]
        );
    }

    #[test]
    fn says_why_a_device_didnt_listen_on_the_network() {
        let fails = |answers: Services| {
            let (listener, port) = listening();
            serve(listener, phone(answers));
            let error = listen(port, 5555).err().unwrap();
            format!("{error:#}").replace(&port.to_string(), "PORT")
        };
        let on_wi_fi = || ("shell:ip", Some(ON_WI_FI.to_vec()));

        assert_eq!(
            fails(vec![(
                "shell:ip",
                Some(b"1: lo    inet 127.0.0.1/8 scope host lo\n".to_vec())
            )]),
            "Pixel 8 (127.0.0.1:PORT) isn't on Wi-Fi or Ethernet: connect it to the network this computer is on"
        );
        assert_eq!(
            fails(vec![("shell:ip", None)]),
            "couldn't read the address of Pixel 8 (127.0.0.1:PORT): the device closed the connection"
        );
        assert_eq!(
            fails(vec![on_wi_fi(), ("tcpip:", None)]),
            "couldn't ask Pixel 8 (127.0.0.1:PORT) to listen on the network: the device closed the connection"
        );
        assert_eq!(
            fails(vec![
                on_wi_fi(),
                ("tcpip:", Some(b"invalid port 0\n".to_vec()))
            ]),
            "Pixel 8 (127.0.0.1:PORT) didn't listen on the network: invalid port 0"
        );
        assert_eq!(
            fails(vec![on_wi_fi()]),
            "Pixel 8 (127.0.0.1:PORT) didn't listen on the network: it gave no reason"
        );
    }

    #[test]
    fn says_when_it_cant_reach_the_device_on_the_network() {
        // Nothing listens on the new port.
        let (listener, port) = listening();
        let unreachable = closed();
        serve(listener, phone(restarting(unreachable)));
        // Something on the new port that hangs up.
        let (second, second_port) = listening();
        let (hanging_up, hanging_up_port) = listening();
        serve(second, phone(restarting(hanging_up_port)));
        serve(hanging_up, |_| None);

        let mut device = connect(Some(&format!("127.0.0.1:{port}")), &[]).unwrap();
        let refused = device
            .listen(unreachable, &KEY, &asking_user, Duration::ZERO)
            .err()
            .unwrap();
        let hung_up = listen(second_port, hanging_up_port).err().unwrap();

        let refused = format!("{refused:#}");
        assert!(
            refused.starts_with(&format!(
                "Pixel 8 (127.0.0.1:{port}) listens for ADB at 127.0.0.1:{unreachable} now, but this computer couldn't reach it there ("
            )),
            "{refused}"
        );
        assert!(
            refused.ends_with("): it may be on another network, or on one that keeps its devices apart, as many guest networks do"),
            "{refused}"
        );
        assert_eq!(
            format!("{hung_up:#}"),
            format!(
                "couldn't connect to 127.0.0.1:{hanging_up_port}: the device closed the connection"
            )
        );
    }

    #[test]
    fn leaves_emulators_on_their_own_network() {
        let (first, one) = listening();
        let (second, two) = listening();
        serve(first, phone(Vec::new()));
        serve(second, phone(Vec::new()));

        let named = connect(Some(&format!("emulator-{}", one - 1)), &[]).unwrap();
        let only = connect(None, &[two]).unwrap();

        for (mut emulator, port) in [(named, one), (only, two)] {
            let error = emulator
                .listen(5555, &KEY, &asking_user, Duration::ZERO)
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                format!(
                    "Pixel 8 (emulator-{0}) is an emulator, which this computer reaches over the network already: name it with --device emulator-{0}",
                    port - 1
                )
            );
        }
    }

    #[test]
    fn finds_the_address_on_wi_fi_or_ethernet() {
        let address = |ip: &str| network_address(ip).map(|address| address.to_string());

        assert_eq!(
            address(&String::from_utf8_lossy(ON_WI_FI)),
            Some("127.0.0.1".to_owned())
        );
        // Wi-Fi before Ethernet, whichever comes first.
        assert_eq!(
            address(
                "2: eth0    inet 10.0.2.15/24 brd 10.0.2.255 scope global eth0\n\
                 3: wlan0    inet 192.168.1.20/24 scope global wlan0\n"
            ),
            Some("192.168.1.20".to_owned())
        );
        assert_eq!(
            address("2: eth0    inet 10.0.2.15/24 scope global eth0\n"),
            Some("10.0.2.15".to_owned())
        );
        // An address without its prefix's length.
        assert_eq!(
            address("3: wlan0    inet 192.168.1.20 scope global wlan0\n"),
            Some("192.168.1.20".to_owned())
        );
        // Nothing on Wi-Fi or Ethernet: mobile data, a VPN, loopback, and
        // lines that say nothing.
        assert_eq!(
            address(
                "1: lo    inet 127.0.0.1/8 scope host lo\n\
                 12: rmnet_data0    inet 10.141.7.9/30 scope global rmnet_data0\n\
                 40: tun0    inet 10.8.0.2/24 scope global tun0\n\
                 \n\
                 3: wlan0\n\
                 3: wlan0    inet\n\
                 3: wlan0    inet not-an-address/24\n"
            ),
            None
        );
    }

    #[test]
    fn pairs_with_a_device_at_an_address() {
        let (listener, port) = listening();
        let device = device(listener, "123456", Stray::Not);

        let guid = pair(&format!("127.0.0.1:{port}"), "123456", &KEY).unwrap();

        assert_eq!(guid, "adb-R5CT1234-a1B2c3");
        assert_eq!(device.join().unwrap().unwrap(), KEY.public_key());
    }

    #[test]
    fn says_why_it_couldnt_pair() {
        let port = closed();
        let wrong_code = {
            let (listener, port) = listening();
            device(listener, "123456", Stray::Not);
            pair(&format!("127.0.0.1:{port}"), "000000", &KEY)
                .err()
                .unwrap()
        };
        let refused = pair(&format!("127.0.0.1:{port}"), "123456", &KEY)
            .err()
            .unwrap();

        assert!(
            format!("{wrong_code:#}").contains(": the code isn't the one the device shows"),
            "{wrong_code:#}"
        );
        assert!(
            format!("{refused:#}").starts_with(&format!("couldn't pair with 127.0.0.1:{port}: ")),
            "{refused:#}"
        );
    }
}
