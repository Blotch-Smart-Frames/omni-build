//! Installing apps on Android devices with no Android SDK, over ADB, the
//! protocol the SDK's adb speaks to them. Over TCP for now: to the
//! emulators running on this computer, and to devices on the network.

mod connection;
mod key;

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use connection::Connection;
pub use key::{AdbKey, key_path, owner};

/// How long OmniBuild waits for a device: long enough for its user to
/// allow OmniBuild's key, and for it to check and compile a large app.
const TIMEOUT: Duration = Duration::from_secs(120);
/// How long OmniBuild waits to connect to a port on this computer, where
/// nothing listening refuses at once.
const LOCAL_TIMEOUT: Duration = Duration::from_millis(500);

/// The ports where emulators on this computer listen for ADB, and where
/// adb looks for them: each emulator takes two ports from 5554 on, its
/// console's and then ADB's, so the odd ones from 5555 to 5585.
pub fn emulator_ports() -> Vec<u16> {
    (5555..=5585).step_by(2).collect()
}

/// A device OmniBuild has connected to.
pub struct Device {
    /// Its name, as adb's: `emulator-5554`, or its address.
    serial: String,
    connection: Connection<'static>,
}

impl Device {
    /// Connects to `device`, an emulator's name or a device's address and
    /// port, or without one to the only emulator listening on one of
    /// `emulator_ports`, and proves OmniBuild's identity with `key`.
    pub fn connect(device: Option<&str>, emulator_ports: &[u16], key: &AdbKey) -> Result<Self> {
        let (serial, stream) = match device {
            Some(device) => (device.to_owned(), reach(device)?),
            None => only_emulator(emulator_ports)?,
        };
        // Without Nagle's algorithm, as adb: each small message would
        // otherwise wait for the device to acknowledge the last.
        let connection = stream
            .set_read_timeout(Some(TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
            .and_then(|()| stream.set_nodelay(true))
            .map_err(anyhow::Error::from)
            .and_then(|()| Connection::open(stream, key))
            .with_context(|| format!("couldn't connect to {serial}"))?;
        Ok(Self { serial, connection })
    }

    /// Installs an APK, or updates the app with its ID, as `adb install`
    /// does from Android 7.0 on: streamed to the package manager's install
    /// command, which the device's `cmd` runs. Earlier versions have no
    /// `cmd`, and need a v1 signature, which OmniBuild doesn't sign apps
    /// with.
    pub fn install(&mut self, apk: &[u8]) -> Result<()> {
        if !self.connection.has_feature("cmd") {
            bail!("{self} runs Android before 7.0, which OmniBuild can't install apps on");
        }
        let output = self
            .connection
            .run(
                &format!("exec:cmd package install -r -S {}", apk.len()),
                apk,
            )
            .with_context(|| format!("couldn't install the app on {self}"))?;
        // "Success", or "Failure [INSTALL_FAILED_...: why]".
        let output = String::from_utf8_lossy(&output);
        if output.lines().any(|line| line.starts_with("Success")) {
            return Ok(());
        }
        let reason = output.trim();
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
}

impl fmt::Display for Device {
    /// Its model, as its user knows it, then its name: "Pixel 8
    /// (emulator-5554)".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.connection.property("ro.product.model") {
            Some(model) => write!(f, "{model} ({})", self.serial),
            None => f.write_str(&self.serial),
        }
    }
}

/// Connects to a device by its name: an emulator's, `emulator-<console
/// port>`, which listens for ADB on the next port, or a device's address
/// and port.
fn reach(device: &str) -> Result<TcpStream> {
    let emulator = device
        .strip_prefix("emulator-")
        .and_then(|port| port.parse::<u16>().ok())
        .and_then(|port| port.checked_add(1));
    let stream = match emulator {
        Some(port) => TcpStream::connect((Ipv4Addr::LOCALHOST, port)),
        None if device.contains(':') => TcpStream::connect(device),
        None => bail!(
            "\"{device}\" isn't a device OmniBuild can reach: name an emulator, such as emulator-5554, or a device on the network by its address and port, such as 192.168.1.20:5555. OmniBuild doesn't install over USB yet"
        ),
    };
    stream.with_context(|| format!("couldn't reach {device}"))
}

/// The only emulator listening on one of `ports` on this computer, with
/// the name adb gives it.
fn only_emulator(ports: &[u16]) -> Result<(String, TcpStream)> {
    // All at once: Windows takes a second or two to refuse a connection
    // to a port nothing listens on, where other systems refuse at once.
    let mut emulators: Vec<_> = std::thread::scope(|scope| {
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
    });
    match emulators.len() {
        0 => bail!(
            "found no Android device: start an emulator, or name a device on the network with --device <address>:<port>. OmniBuild doesn't install over USB yet"
        ),
        1 => Ok(emulators.remove(0)),
        count => {
            let names: Vec<_> = emulators.iter().map(|(name, _)| name.as_str()).collect();
            bail!(
                "{count} emulators are running ({}): name the one to install on with --device",
                names.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::LazyLock;

    use super::connection::tests::{BANNER, exec_device, serve};
    use super::{AdbKey, Device, emulator_ports};

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb/adbkey");
    static KEY: LazyLock<AdbKey> =
        LazyLock::new(|| AdbKey::at(Path::new(FIXTURE), "omnibuild@test").unwrap().0);

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

        let mut connected = Device::connect(Some(&format!("127.0.0.1:{port}")), &[], &KEY).unwrap();
        connected.install(&apk).unwrap();

        assert_eq!(connected.to_string(), format!("Pixel 8 (127.0.0.1:{port})"));
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

        let connected = Device::connect(Some(&name), &[], &KEY).unwrap();

        // A banner without the model, as early versions of Android send.
        assert_eq!(connected.to_string(), name);
    }

    #[test]
    fn finds_the_only_emulator_running() {
        let (listener, port) = listening();
        serve(listener, exec_device(4096, BANNER, Some(b"Success\n")));

        let connected = Device::connect(None, &[closed(), port], &KEY).unwrap();

        assert_eq!(
            connected.to_string(),
            format!("Pixel 8 (emulator-{})", port - 1)
        );
    }

    #[test]
    fn asks_which_emulator_when_several_are_running() {
        let ((_first, one), (_second, two)) = (listening(), listening());

        let error = Device::connect(None, &[one, two], &KEY).err().unwrap();

        assert_eq!(
            error.to_string(),
            format!(
                "2 emulators are running (emulator-{}, emulator-{}): name the one to install on with --device",
                one - 1,
                two - 1
            )
        );
    }

    #[test]
    fn says_when_it_finds_no_device() {
        let error = Device::connect(None, &[closed()], &KEY).err().unwrap();

        assert!(error.to_string().starts_with("found no Android device"));
    }

    #[test]
    fn says_why_it_couldnt_reach_or_connect_to_a_device() {
        let port = closed();
        let (listener, open) = listening();
        // A device that hangs up on OmniBuild's first message, once it has
        // read it: hanging up on unread bytes would reset the connection.
        serve(listener, |_| None);

        let refused = Device::connect(Some(&format!("emulator-{}", port - 1)), &[], &KEY);
        let unknown = Device::connect(Some("R5CT1234"), &[], &KEY);
        let out_of_range = Device::connect(Some("emulator-65535"), &[], &KEY);
        let hung_up = Device::connect(Some(&format!("127.0.0.1:{open}")), &[], &KEY);

        let message = |result: anyhow::Result<Device>| format!("{:#}", result.err().unwrap());
        assert!(message(refused).starts_with(&format!("couldn't reach emulator-{}: ", port - 1)));
        assert!(message(unknown).starts_with("\"R5CT1234\" isn't a device OmniBuild can reach"));
        assert!(message(out_of_range).starts_with("\"emulator-65535\" isn't a device"));
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
            let mut device =
                Device::connect(Some(&format!("127.0.0.1:{port}")), &[], &KEY).unwrap();
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
        assert_eq!(
            install(b"device::ro.product.model=Nexus 5;features=shell_v2\0", b""),
            "Nexus 5 (127.0.0.1:PORT) runs Android before 7.0, which OmniBuild can't install apps on"
        );
    }

    #[test]
    fn says_when_the_device_goes_away_during_an_install() {
        let (listener, port) = listening();
        // A device that connects, then hangs up when the app comes.
        serve(listener, exec_device(4096, BANNER, None));
        let mut device = Device::connect(Some(&format!("127.0.0.1:{port}")), &[], &KEY).unwrap();

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
}
