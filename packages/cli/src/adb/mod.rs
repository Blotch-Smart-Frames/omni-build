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
/// Where an APK goes on a device before Android 7.0, for the package
/// manager to install it from: a folder adb's shell can write to and the
/// package manager read, where adb puts it too.
const PUSHED_APK: &str = "/data/local/tmp/omni-build.apk";
/// The pushed APK's mode, as `adb push` sends it: a regular file anyone may
/// read, so the package manager can.
const PUSHED_MODE: u32 = 0o100_644;
/// The most a sync DATA request carries.
const SYNC_DATA_MAX: usize = 64 * 1024;

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
    /// Calls `authorizing` with the device's name when the device asks its
    /// user whether to allow the key, and waits for the answer.
    pub fn connect(
        device: Option<&str>,
        emulator_ports: &[u16],
        key: &AdbKey,
        authorizing: &dyn Fn(&str),
    ) -> Result<Self> {
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
            .and_then(|()| Connection::open(stream, key, &|| authorizing(&serial)))
            .with_context(|| format!("couldn't connect to {serial}"))?;
        Ok(Self { serial, connection })
    }

    /// Its name, as adb's: `emulator-5554`, or its address.
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
    /// (emulator-5554)".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.model() {
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
    use std::cell::RefCell;
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::{Arc, LazyLock, Mutex};

    use super::connection::tests::{
        Answer, BANNER, Respond, asking, exec_device, serve, service_device,
    };
    use super::{AdbKey, Device, emulator_ports};

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb/adbkey");
    static KEY: LazyLock<AdbKey> =
        LazyLock::new(|| AdbKey::at(Path::new(FIXTURE), "omnibuild@test").unwrap().0);

    thread_local! {
        /// The devices that asked their user to allow the key, by name, in
        /// this thread's test.
        static ASKED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// Connects as `install` does, with the fixture key, noting each device
    /// that asks its user to allow the key in `ASKED`.
    fn connect(device: Option<&str>, emulator_ports: &[u16]) -> anyhow::Result<Device> {
        Device::connect(device, emulator_ports, &KEY, &|name| {
            ASKED.with_borrow_mut(|asked| asked.push(name.to_owned()));
        })
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
    fn asks_which_emulator_when_several_are_running() {
        let ((_first, one), (_second, two)) = (listening(), listening());

        let error = connect(None, &[one, two]).err().unwrap();

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
}
