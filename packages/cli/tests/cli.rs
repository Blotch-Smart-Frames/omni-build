//! Runs the built `omni-build` binary as a user would. Besides checking the
//! command line end to end, this is what covers `main()`: cargo-llvm-cov
//! counts binaries that tests start, but no unit test can call `main()`.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::thread::JoinHandle;

const SHELL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.apk");
const SHELL_AAB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.aab");
const SHELL_APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.app.zip");
const SHELL_IPA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.ipa");
/// A key and certificate, and a provisioning profile allowing it: see
/// tests/fixtures/README.md.
const IDENTITY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/identity.pem");
const PROFILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/profile.mobileprovision"
);
/// The test profile for the shell's App Clip, which needs one of its own.
const CLIP_PROFILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/clip.mobileprovision"
);

fn omni_build(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omni-build"))
        .args(args)
        .output()
        .expect("the omni-build binary should start")
}

/// Runs omni-build with `home` as the user's home folder, where it keeps
/// its debug key.
fn omni_build_at_home(home: &Path, args: &[&str]) -> Output {
    omni_build_with_password(home, None, args)
}

/// Runs omni-build at `home`, with a keystore password, or none.
fn omni_build_with_password(home: &Path, password: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_omni-build"));
    command
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("OMNI_BUILD_KEY_PASSWORD");
    if let Some(password) = password {
        command.env("OMNI_BUILD_KEY_PASSWORD", password);
    }
    command
        .output()
        .expect("the omni-build binary should start")
}

/// adb's key, as `adb keygen` makes one.
const ADB_KEY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb/adbkey");

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_flag_prints_the_crate_version() {
    let output = omni_build(&["--version"]);

    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("omni-build {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn no_arguments_prints_help_and_fails() {
    let output = omni_build(&[]);

    // clap prints help to stderr and exits with 2 when arguments are required.
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn repackage_signs_the_shell_with_a_debug_key_it_creates_once() {
    let home = tempfile::tempdir().unwrap();
    let app = home.path().join("app.apk");
    let app = app.to_str().unwrap();

    let first = omni_build_at_home(home.path(), &["repackage", SHELL, "--out", app]);
    let second = omni_build_at_home(home.path(), &["repackage", SHELL, "-o", app]);

    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).starts_with("Created a debug key for signing Android apps: "),
        "{}",
        stderr(&first)
    );
    assert!(
        home.path()
            .join(".omni-build/android-debug-key.pem")
            .is_file()
    );
    assert!(second.status.success(), "{}", stderr(&second));
    assert_eq!(stderr(&second), "");
    assert_eq!(
        String::from_utf8_lossy(&second.stdout),
        format!("Repackaged {SHELL} into {app}\n")
    );
    let signed = fs::read(app).unwrap();
    assert!(
        signed
            .windows(16)
            .any(|window| window == b"APK Sig Block 42")
    );
}

#[test]
fn repackage_gives_the_shell_the_apps_values() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("app.json");
    fs::write(
        &config,
        r#"{ "id": "dev.example.app", "version": "2.5.0", "build": 42 }"#,
    )
    .unwrap();
    let app = home.path().join("app.apk");

    let output = omni_build_at_home(
        home.path(),
        &[
            "repackage",
            SHELL,
            "--config",
            config.to_str().unwrap(),
            "--out",
            app.to_str().unwrap(),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    // The manifest is compressed, so this checks it changed at all; the unit
    // tests check what it says.
    assert_ne!(fs::read(app).unwrap().len(), fs::read(SHELL).unwrap().len());
}

#[test]
fn repackage_gives_an_ios_app_its_values_and_icon() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("app.json");
    fs::write(
        &config,
        r#"{ "id": "dev.example.app", "version": "2.5.0", "build": 42, "icon": "icon.png" }"#,
    )
    .unwrap();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([0xe8, 0x59, 0x0c, 0xff]))
        .save(home.path().join("icon.png"))
        .unwrap();
    let app = home.path().join("app.zip");

    let output = omni_build_at_home(
        home.path(),
        &[
            "repackage",
            SHELL_APP,
            "--config",
            config.to_str().unwrap(),
            "--out",
            app.to_str().unwrap(),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    // Info.plist and the icons are compressed, so this checks they changed
    // at all; the unit tests check what they say.
    assert_ne!(fs::read(app).unwrap(), fs::read(SHELL_APP).unwrap());
    // iOS apps aren't signed with the Android debug key.
    assert!(!home.path().join(".omni-build").exists());
}

#[test]
fn repackage_signs_with_the_key_it_is_given() {
    let home = tempfile::tempdir().unwrap();
    let (apk, ipa) = (home.path().join("app.apk"), home.path().join("app.ipa"));

    let android = omni_build_at_home(
        home.path(),
        &[
            "repackage",
            SHELL,
            "--signing-key",
            IDENTITY,
            "--out",
            apk.to_str().unwrap(),
        ],
    );
    let ios = omni_build_at_home(
        home.path(),
        &[
            "repackage",
            SHELL_IPA,
            "--signing-key",
            IDENTITY,
            // One for the app and one for each extension and the watch app,
            // as they'd be without a wildcard App ID, and the App Clip's own.
            "--profile",
            PROFILE,
            "--profile",
            PROFILE,
            "--profile",
            PROFILE,
            "--profile",
            PROFILE,
            "--profile",
            CLIP_PROFILE,
            "--out",
            ipa.to_str().unwrap(),
        ],
    );

    assert!(android.status.success(), "{}", stderr(&android));
    assert!(ios.status.success(), "{}", stderr(&ios));
    // No debug key: the APK's signature has the given key's certificate.
    assert!(!home.path().join(".omni-build").exists());
    let contains =
        |bytes: &[u8], text: &[u8]| bytes.windows(text.len()).any(|window| window == text);
    assert!(contains(&fs::read(apk).unwrap(), b"OmniBuild Test"));
    // Zip entry names aren't compressed.
    let ipa = fs::read(ipa).unwrap();
    assert!(contains(
        &ipa,
        b"Payload/OmniBuildShell.app/embedded.mobileprovision"
    ));
    assert!(contains(
        &ipa,
        b"Payload/OmniBuildShell.app/PlugIns/ShellNotifications.appex/embedded.mobileprovision"
    ));
    assert!(contains(
        &ipa,
        b"Payload/OmniBuildShell.app/Extensions/ShellIntents.appex/embedded.mobileprovision"
    ));
    assert!(contains(
        &ipa,
        b"Payload/OmniBuildShell.app/AppClips/ShellClip.app/embedded.mobileprovision"
    ));
    assert!(contains(
        &ipa,
        b"Payload/OmniBuildShell.app/Watch/ShellWatch.app/embedded.mobileprovision"
    ));
}

#[test]
fn creates_an_upload_key_and_signs_with_it() {
    let home = tempfile::tempdir().unwrap();
    let keystore = home.path().join("keys/upload.p12");
    let keystore = keystore.to_str().unwrap();
    let apk = home.path().join("app.apk");

    let created = omni_build_with_password(
        home.path(),
        Some("a password"),
        &["key", "create", keystore],
    );
    let signed = omni_build_with_password(
        home.path(),
        Some("a password"),
        &[
            "repackage",
            SHELL,
            "--signing-key",
            keystore,
            "--out",
            apk.to_str().unwrap(),
        ],
    );

    assert!(created.status.success(), "{}", stderr(&created));
    assert!(
        String::from_utf8_lossy(&created.stdout).starts_with(&format!(
            "Created an upload key in {keystore}, with the alias \"upload\"."
        ))
    );
    assert!(signed.status.success(), "{}", stderr(&signed));
    let apk = fs::read(apk).unwrap();
    assert!(apk.windows(10).any(|window| window == b"Upload key"));
}

#[test]
fn reports_an_upload_key_it_cant_create_or_read() {
    let home = tempfile::tempdir().unwrap();
    let keystore = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/keystores/openssl.p12"
    );
    let cases = [
        (
            Some("a password"),
            vec!["key", "create", "upload.p12", "--subject", "not a name"],
            "error: \"not a name\" isn't a distinguished name, such as \"CN=Upload key, O=Example\"\n",
        ),
        (
            None,
            vec!["key", "create", "upload.p12"],
            "error: the keystore's password (OMNI_BUILD_KEY_PASSWORD) has to have at least 6 characters",
        ),
        (
            Some("wrong"),
            vec![
                "repackage",
                SHELL,
                "--signing-key",
                keystore,
                "--out",
                "app.apk",
            ],
            "openssl.p12: the password isn't its password (OMNI_BUILD_KEY_PASSWORD)\n",
        ),
    ];

    for (password, args, message) in cases {
        let output = omni_build_with_password(home.path(), password, &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).starts_with(message) || stderr(&output).ends_with(message),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn repackage_reports_what_went_wrong() {
    let home = tempfile::tempdir().unwrap();
    let not_apk = home.path().join("not.apk");
    fs::write(&not_apk, "not a zip").unwrap();
    // An empty zip archive, only its end record: neither platform's app.
    let empty_zip = home.path().join("empty.zip");
    fs::write(
        &empty_zip,
        b"PK\x05\x06\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
    )
    .unwrap();
    let nowhere = home.path().join("missing/app.apk");
    // An ID neither platform takes: Android's need two parts, and iOS's
    // can't have underscores.
    let bad_id = home.path().join("bad-id.json");
    fs::write(&bad_id, r#"{ "id": "my_app" }"#).unwrap();
    let bad_id = bad_id.to_str().unwrap();
    let cases = [
        (
            vec!["repackage", "missing.apk", "--out", "app.apk"],
            "error: couldn't read the shell missing.apk: ",
        ),
        (
            vec!["repackage", not_apk.to_str().unwrap(), "--out", "app.apk"],
            "error: the shell isn't an APK, an AAB or an iOS app: it isn't a zip archive\n",
        ),
        (
            vec!["repackage", empty_zip.to_str().unwrap(), "--out", "app.apk"],
            "error: the shell isn't an APK, an AAB or an iOS app: it has no AndroidManifest.xml or BundleConfig.pb, and no .app with an Info.plist\n",
        ),
        (
            vec!["repackage", SHELL, "--out", nowhere.to_str().unwrap()],
            "error: couldn't write the app to ",
        ),
        (
            vec![
                "repackage",
                SHELL,
                "--config",
                "missing.json",
                "--out",
                "app.apk",
            ],
            "error: couldn't read the app config missing.json: ",
        ),
        (
            vec!["repackage", SHELL, "--config", bad_id, "--out", "app.apk"],
            "error: couldn't rewrite the shell's AndroidManifest.xml: \"my_app\" isn't a valid Android application ID: ",
        ),
        (
            vec![
                "repackage",
                SHELL_APP,
                "--config",
                bad_id,
                "--out",
                "app.zip",
            ],
            "error: couldn't rewrite the shell's OmniBuildShell.app/Info.plist: \"my_app\" isn't a valid iOS bundle ID: ",
        ),
        (
            vec![
                "repackage",
                SHELL,
                "--signing-key",
                "missing.pem",
                "--out",
                "app.apk",
            ],
            "error: couldn't read the signing key missing.pem: ",
        ),
        (
            vec![
                "repackage",
                SHELL,
                "--profile",
                "missing.mobileprovision",
                "--out",
                "app.apk",
            ],
            "error: couldn't read the provisioning profile missing.mobileprovision: ",
        ),
        (
            vec!["repackage", SHELL, "--profile", PROFILE, "--out", "app.apk"],
            "error: a provisioning profile is for iOS apps, and the shell is an Android app\n",
        ),
        (
            vec!["repackage", SHELL_IPA, "--out", "app.ipa"],
            "error: a build for devices has to be signed with a key Apple issued a certificate for, and a provisioning profile allowing it: give them with --signing-key and --profile\n",
        ),
    ];

    for (args, message) in cases {
        let output = omni_build_at_home(home.path(), &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).contains(message), "{}", stderr(&output));
    }
}

#[test]
fn repackage_reports_a_debug_key_it_cant_read() {
    let home = tempfile::tempdir().unwrap();
    // A folder where the key should be.
    fs::create_dir_all(home.path().join(".omni-build/android-debug-key.pem")).unwrap();

    let output = omni_build_at_home(home.path(), &["repackage", SHELL, "--out", "app.apk"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("error: couldn't read "),
        "{}",
        stderr(&output)
    );
}

/// What a device was asked to install: the command that installs it, and
/// the app.
type Install = (String, Vec<u8>);

/// A stand-in for a device's adbd, on a port of this computer, which takes
/// a connection for each of `answers`: it connects without checking the
/// key, or when it `asks`, once its user allows the key, and installs an
/// app as Android 14 does, with that answer. Returns its port, and what
/// each connection sent.
fn fake_device(asks: bool, answers: &[&'static [u8]]) -> (u16, JoinHandle<Vec<Install>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let answers = answers.to_vec();
    let device = std::thread::spawn(move || {
        answers
            .into_iter()
            .map(|answer| {
                let (mut stream, _) = listener.accept().unwrap();
                assert_eq!(&receive(&mut stream).0, b"CNXN");
                if asks {
                    // A token, a second for the signature, and then the
                    // key, which its user allows.
                    send(&mut stream, *b"AUTH", 1, 0, &[1; 20]);
                    assert_eq!(receive(&mut stream).1, 2);
                    send(&mut stream, *b"AUTH", 1, 0, &[2; 20]);
                    assert_eq!(receive(&mut stream).1, 3);
                }
                let banner = b"device::ro.product.model=Pixel 8;features=shell_v2,cmd\0";
                send(&mut stream, *b"CNXN", 0x0100_0001, 4096, banner);
                let (open, local, _, service) = receive(&mut stream);
                assert_eq!(&open, b"OPEN");
                let service = String::from_utf8(service).unwrap();
                let service = service.trim_end_matches('\0').to_owned();
                let size: usize = service.rsplit(' ').next().unwrap().parse().unwrap();
                send(&mut stream, *b"OKAY", 100, local, b"");
                let mut app = Vec::new();
                while app.len() < size {
                    let (write, _, _, data) = receive(&mut stream);
                    assert_eq!(&write, b"WRTE");
                    app.extend(data);
                    send(&mut stream, *b"OKAY", 100, local, b"");
                }
                send(&mut stream, *b"WRTE", 100, local, answer);
                send(&mut stream, *b"CLSE", 100, local, b"");
                // Until omni-build hangs up, as a device does: hanging up
                // first would make its OKAY for the answer reset the
                // connection.
                stream.read_to_end(&mut Vec::new()).unwrap();
                (service, app)
            })
            .collect()
    });
    (port, device)
}

/// An ADB message: its command, its two arguments and its payload.
fn receive(stream: &mut TcpStream) -> ([u8; 4], u32, u32, Vec<u8>) {
    let mut header = [0; 24];
    stream.read_exact(&mut header).unwrap();
    let word = |index: usize| u32::from_le_bytes(header[index * 4..][..4].try_into().unwrap());
    let mut data = vec![0; word(3) as usize];
    stream.read_exact(&mut data).unwrap();
    (header[..4].try_into().unwrap(), word(1), word(2), data)
}

fn send(stream: &mut TcpStream, command: [u8; 4], arg0: u32, arg1: u32, data: &[u8]) {
    let command = u32::from_le_bytes(command);
    let len = u32::try_from(data.len()).unwrap();
    let mut message = Vec::new();
    // No checksum: devices since Android 9 send none.
    for word in [command, arg0, arg1, len, 0, !command] {
        message.extend(word.to_le_bytes());
    }
    message.extend(data);
    stream.write_all(&message).unwrap();
}

#[test]
fn installs_an_apk_with_an_adb_key_it_creates_once() {
    let home = tempfile::tempdir().unwrap();
    let (port, device) = fake_device(false, &[b"Success\n", b"Success\n"]);
    let address = format!("127.0.0.1:{port}");

    let first = omni_build_at_home(home.path(), &["install", SHELL, "--device", &address]);
    let second = omni_build_at_home(home.path(), &["install", SHELL, "-d", &address]);

    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).starts_with("Created a key for ADB, which devices ask you to allow: "),
        "{}",
        stderr(&first)
    );
    assert!(home.path().join(".android/adbkey").is_file());
    assert!(home.path().join(".android/adbkey.pub").is_file());
    assert!(second.status.success(), "{}", stderr(&second));
    assert_eq!(stderr(&second), "");
    assert_eq!(
        String::from_utf8_lossy(&second.stdout),
        format!("Installed {SHELL} on Pixel 8 ({address})\n")
    );
    let apk = fs::read(SHELL).unwrap();
    for (service, app) in device.join().unwrap() {
        assert_eq!(
            service,
            format!("exec:cmd package install -r -S {}", apk.len())
        );
        assert_eq!(app, apk);
    }
}

#[test]
fn install_reports_what_went_wrong() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".android")).unwrap();
    fs::copy(ADB_KEY, home.path().join(".android/adbkey")).unwrap();
    let not_apk = home.path().join("not.apk");
    fs::write(&not_apk, "not a zip").unwrap();
    let not_apk = not_apk.to_str().unwrap();
    let cases = [
        (
            vec!["install", "missing.apk"],
            "error: couldn't read the app missing.apk: ".to_owned(),
        ),
        (
            vec!["install", not_apk],
            format!("error: {not_apk} isn't an APK\n"),
        ),
        (
            vec!["install", SHELL_AAB],
            format!("error: {SHELL_AAB} is an app bundle (AAB), which is for Google Play: "),
        ),
        (
            vec!["install", SHELL, "--device", "R5CT1234"],
            "error: \"R5CT1234\" isn't a device OmniBuild can reach: ".to_owned(),
        ),
    ];

    for (args, message) in cases {
        let output = omni_build_at_home(home.path(), &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with(&message), "{}", stderr(&output));
    }
}

#[test]
fn install_reports_an_adb_key_it_cant_read() {
    let home = tempfile::tempdir().unwrap();
    // A folder where the key should be.
    fs::create_dir_all(home.path().join(".android/adbkey")).unwrap();

    let output = omni_build_at_home(home.path(), &["install", SHELL]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("error: couldn't read "),
        "{}",
        stderr(&output)
    );
}

#[test]
fn install_reports_why_the_device_refused_the_app() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".android")).unwrap();
    fs::copy(ADB_KEY, home.path().join(".android/adbkey")).unwrap();
    let (port, _) = fake_device(
        false,
        &[b"Failure [INSTALL_FAILED_OLDER_SDK: Requires newer sdk version #37]\n"],
    );
    let address = format!("127.0.0.1:{port}");

    let output = omni_build_at_home(home.path(), &["install", SHELL, "--device", &address]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        format!(
            "error: Pixel 8 ({address}) didn't install the app: Failure [INSTALL_FAILED_OLDER_SDK: Requires newer sdk version #37]\n"
        )
    );
}

/// The JSON lines omni-build wrote to stdout, for build tools.
fn events(output: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn reports_to_build_tools_as_json_lines() {
    let home = tempfile::tempdir().unwrap();
    let app = home.path().join("app.aab");
    let app = app.to_str().unwrap();
    // Joined a part at a time, as each system writes its paths.
    let key = home
        .path()
        .join(".omni-build")
        .join("android-debug-key.pem");

    // --json goes after the command, or before it.
    let output = omni_build_at_home(home.path(), &["repackage", SHELL_AAB, "-o", app, "--json"]);
    let failed = omni_build_at_home(
        home.path(),
        &["--json", "repackage", "missing.apk", "-o", app],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    // Everything on stdout, and nothing for people on stderr.
    assert_eq!(stderr(&output), "");
    assert_eq!(
        events(&output),
        [
            serde_json::json!({
                "event": "key-created",
                "key": "debug",
                "path": key.display().to_string(),
                "message": format!("Created a debug key for signing Android apps: {}", key.display()),
            }),
            serde_json::json!({
                "event": "repackaged",
                "shell": SHELL_AAB,
                "out": app,
                "platform": "android",
                "format": "aab",
                "message": format!("Repackaged {SHELL_AAB} into {app}"),
            }),
        ]
    );
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(stderr(&failed), "");
    let [error] = &events(&failed)[..] else {
        panic!("{}", String::from_utf8_lossy(&failed.stdout));
    };
    assert_eq!(error["event"], "error");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("couldn't read the shell missing.apk: "),
        "{error}"
    );
}

#[test]
fn tells_build_tools_when_the_device_asks_its_user() {
    let home = tempfile::tempdir().unwrap();
    let (port, _) = fake_device(true, &[b"Success\n"]);
    let address = format!("127.0.0.1:{port}");

    let output = omni_build_at_home(
        home.path(),
        &["install", SHELL, "--device", &address, "--json"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let events = events(&output);
    let names: Vec<_> = events.iter().map(|event| &event["event"]).collect();
    assert_eq!(names, ["key-created", "authorizing", "installed"]);
    assert_eq!(events[0]["key"], "adb");
    assert_eq!(events[1]["device"], address.as_str());
    assert_eq!(
        events[2],
        serde_json::json!({
            "event": "installed",
            "app": SHELL,
            "device": address,
            "model": "Pixel 8",
            "message": format!("Installed {SHELL} on Pixel 8 ({address})"),
        })
    );
}
