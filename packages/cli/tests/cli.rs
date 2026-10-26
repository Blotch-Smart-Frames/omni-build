//! Runs the built `omni` binary as a user would. Besides checking the
//! command line end to end, this is what covers `main()`: cargo-llvm-cov
//! counts binaries that tests start, but no unit test can call `main()`.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::thread::JoinHandle;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use hkdf::Hkdf;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use sha2::{Digest, Sha256, Sha512};

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
    Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(args)
        .output()
        .expect("the omni binary should start")
}

/// Runs omni with `home` as the user's home folder, where it keeps
/// its debug key.
fn omni_build_at_home(home: &Path, args: &[&str]) -> Output {
    omni_build_with_password(home, None, args)
}

/// Runs omni at `home`, with a keystore password, or none.
fn omni_build_with_password(home: &Path, password: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_omni"));
    command
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("OMNI_BUILD_KEY_PASSWORD");
    if let Some(password) = password {
        command.env("OMNI_BUILD_KEY_PASSWORD", password);
    }
    command.output().expect("the omni binary should start")
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
        format!("omni {}\n", env!("CARGO_PKG_VERSION"))
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
            // One for the app and one for each extension, the watch app
            // and its widgets, as they'd be without a wildcard App ID, and
            // the App Clip's own.
            "--profile",
            PROFILE,
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
    assert!(contains(
        &ipa,
        b"Payload/OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/embedded.mobileprovision"
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

/// A certificate for the key in the request at `path`, as Apple issues
/// one, as DER: from an authority of the test's own, in a name of its own.
fn issue(path: &Path) -> Vec<u8> {
    use rsa::pkcs1v15::{Signature, SigningKey};
    use rsa::rand_core::OsRng;
    use rsa::sha2::Sha256;
    use x509_cert::builder::{Builder, CertificateBuilder, Profile};
    use x509_cert::der::{DecodePem, Encode};
    use x509_cert::request::CertReq;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::time::Validity;

    let request = CertReq::from_pem(fs::read(path).unwrap()).unwrap();
    let authority = SigningKey::<Sha256>::new(rsa::RsaPrivateKey::new(&mut OsRng, 2048).unwrap());
    CertificateBuilder::new(
        Profile::Leaf {
            issuer: "CN=Test Developer Relations,O=Apple Inc.".parse().unwrap(),
            enable_key_agreement: false,
            enable_key_encipherment: false,
        },
        SerialNumber::from(1u32),
        Validity::from_now(std::time::Duration::from_secs(3600)).unwrap(),
        "CN=Apple Distribution: Jane Doe (ABCDE12345),OU=ABCDE12345"
            .parse()
            .unwrap(),
        request.info.public_key,
        &authority,
    )
    .and_then(Builder::build::<Signature>)
    .unwrap()
    .to_der()
    .unwrap()
}

#[test]
fn requests_an_apple_certificate_and_signs_with_it_once_issued() {
    let home = tempfile::tempdir().unwrap();
    let keystore = home.path().join("keys/apple.p12");
    let keystore = keystore.to_str().unwrap();
    let request = home.path().join("keys/apple.certSigningRequest");
    let certificate = home.path().join("distribution.cer");
    let apk = home.path().join("app.apk");
    let run = |args: &[&str]| omni_build_with_password(home.path(), Some("a password"), args);
    let repackage = [
        "repackage",
        SHELL,
        "--signing-key",
        keystore,
        "--out",
        apk.to_str().unwrap(),
    ];

    let requested = run(&["key", "request", keystore, "--subject", "CN=Jane Doe key"]);
    let unissued = run(&repackage);
    fs::write(&certificate, issue(&request)).unwrap();
    let imported = run(&[
        "key",
        "import",
        keystore,
        certificate.to_str().unwrap(),
        "--json",
    ]);
    let signed = run(&repackage);

    assert!(requested.status.success(), "{}", stderr(&requested));
    assert!(
        String::from_utf8_lossy(&requested.stdout).starts_with(&format!(
            "Created a key for an Apple signing certificate in {keystore}, and its certificate signing request in {}.",
            request.display()
        ))
    );
    assert_eq!(unissued.status.code(), Some(1));
    assert!(stderr(&unissued).ends_with(
        "apple.p12 has a key but no certificate yet: add the one Apple issues for its request with `omni key import`\n"
    ));
    assert!(imported.status.success(), "{}", stderr(&imported));
    // Parsed, since JSON escapes the backslashes of a path on Windows.
    let event: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(event["event"], "certificate-imported");
    assert_eq!(event["path"], keystore);
    assert_eq!(
        event["certificate"],
        "Apple Distribution: Jane Doe (ABCDE12345)"
    );
    assert!(signed.status.success(), "{}", stderr(&signed));
    let apk = fs::read(apk).unwrap();
    assert!(
        apk.windows(18)
            .any(|window| window == b"Apple Distribution")
    );
}

#[test]
fn reports_an_apple_key_it_cant_request_or_import() {
    let home = tempfile::tempdir().unwrap();
    let cases = [
        (
            Some("a password"),
            vec!["key", "request", "apple.p12", "--subject", "not a name"],
            "error: \"not a name\" isn't a distinguished name, such as \"CN=Jane Doe key, emailAddress=jane@example.com\"\n",
        ),
        (
            None,
            vec!["key", "request", "apple.p12"],
            "error: the keystore's password (OMNI_BUILD_KEY_PASSWORD) has to have at least 6 characters",
        ),
        (
            Some("a password"),
            vec!["key", "import", "missing.p12", "distribution.cer"],
            "error: couldn't read the keystore missing.p12: ",
        ),
    ];

    for (password, args, message) in cases {
        let output = omni_build_with_password(home.path(), password, &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with(message), "{}", stderr(&output));
    }
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
                // Until omni hangs up, as a device does: hanging up
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
            "error: found no device named \"R5CT1234\": ".to_owned(),
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

/// The JSON lines omni wrote to stdout, for build tools.
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

/// An app config in `dir` naming the app's `package.json`, which depends
/// on a native module, `haptics`, for both platforms, as npm installs it.
fn app_with_a_native_module(dir: &Path) -> String {
    let module = dir.join("node_modules").join("haptics");
    fs::create_dir_all(&module).unwrap();
    fs::write(
        module.join("package.json"),
        r#"{ "name": "haptics", "version": "1.0.0", "omniBuild": {
            "runtime": "example@1",
            "ios": { "modules": ["Haptics"] },
            "android": { "modules": ["dev.example.Haptics"] }
        } }"#,
    )
    .unwrap();
    fs::write(
        dir.join("package.json"),
        r#"{ "dependencies": { "haptics": "^1.0.0" } }"#,
    )
    .unwrap();
    let config = dir.join("app.json");
    fs::write(&config, r#"{ "packageJson": "package.json" }"#).unwrap();
    config.display().to_string()
}

#[test]
fn repackage_finds_the_apps_native_modules() {
    let home = tempfile::tempdir().unwrap();
    let config = app_with_a_native_module(home.path());
    let out = home.path().join("app").display().to_string();

    // Neither shell takes native modules, as a shell that registers them
    // says.
    for (shell, place) in [
        (SHELL_APP, "its Info.plist, as OmniBuildRuntime"),
        (
            SHELL,
            "its manifest, as <meta-data android:name=\"dev.omnibuild.runtime\">",
        ),
    ] {
        let output = omni_build_at_home(
            home.path(),
            &["--json", "repackage", shell, "-c", &config, "-o", &out],
        );

        assert_eq!(output.status.code(), Some(1));
        let events = events(&output);
        let found = events
            .iter()
            .find(|event| event["event"] == "module-found")
            .unwrap();
        assert_eq!(
            found,
            &serde_json::json!({
                "event": "module-found",
                "name": "haptics",
                "version": "1.0.0",
                "message": "Found the native module haptics 1.0.0 among the app's dependencies",
            })
        );
        assert_eq!(
            events.last().unwrap()["message"],
            format!(
                "haptics 1.0.0 is a native module, and the shell takes none: a shell that registers native modules names its runtime in {place}"
            )
        );
    }
    fs::remove_dir_all(home.path().join("node_modules")).unwrap();
    let output = omni_build_at_home(
        home.path(),
        &["repackage", SHELL, "-c", &config, "-o", &out],
    );
    assert_eq!(
        stderr(&output),
        format!(
            "error: haptics, which {} depends on, isn't installed in node_modules: run npm install\n",
            home.path().join("package.json").display()
        )
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

/// What `ip -o -4 addr show` prints on a device on Wi-Fi, whose address is
/// this computer's.
const ON_WI_FI: &[u8] = b"30: wlan0    inet 127.0.0.1/8 scope global wlan0\n";

/// A stand-in for a device's adbd, on a port of this computer, which
/// connects without checking the key, answers the services it's asked for
/// with `outputs`, in turn, and waits for omni to hang up. Returns its
/// port, and the services it was asked for.
fn fake_services(outputs: Vec<Vec<u8>>) -> (u16, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let device = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert_eq!(&receive(&mut stream).0, b"CNXN");
        let banner = b"device::ro.product.model=Pixel 8;features=shell_v2,cmd\0";
        send(&mut stream, *b"CNXN", 0x0100_0001, 4096, banner);
        let services = outputs
            .into_iter()
            .map(|output| {
                let (open, local, _, service) = receive(&mut stream);
                assert_eq!(&open, b"OPEN");
                send(&mut stream, *b"OKAY", 100, local, b"");
                send(&mut stream, *b"WRTE", 100, local, &output);
                send(&mut stream, *b"CLSE", 100, local, b"");
                // omni's OKAY for the output.
                assert_eq!(&receive(&mut stream).0, b"OKAY");
                let service = String::from_utf8(service).unwrap();
                service.trim_end_matches('\0').to_owned()
            })
            .collect();
        stream.read_to_end(&mut Vec::new()).unwrap();
        services
    });
    (port, device)
}

/// A device on the network that restarts to listen on another port when
/// omni asks, with a stand-in listening there. Returns the device's port,
/// the one it listens on next, and the services omni asked it for.
fn restarting_device() -> (u16, u16, JoinHandle<Vec<String>>) {
    let (new_port, _) = fake_services(Vec::new());
    let answer = format!("restarting in TCP mode port: {new_port}\n");
    let (port, device) = fake_services(vec![ON_WI_FI.to_vec(), answer.into_bytes()]);
    (port, new_port, device)
}

#[test]
fn tcpip_has_a_device_listen_on_the_network() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".android")).unwrap();
    fs::copy(ADB_KEY, home.path().join(".android/adbkey")).unwrap();
    let (port, new_port, device) = restarting_device();

    let output = omni_build_at_home(
        home.path(),
        &[
            "tcpip",
            "--device",
            &format!("127.0.0.1:{port}"),
            "--port",
            &new_port.to_string(),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "Pixel 8 (127.0.0.1:{port}) listens for ADB at 127.0.0.1:{new_port} until it restarts. Unplug it, and install with --device 127.0.0.1:{new_port}\n"
        )
    );
    assert_eq!(
        device.join().unwrap(),
        [
            "shell:ip -o -4 addr show".to_owned(),
            format!("tcpip:{new_port}")
        ]
    );
}

#[test]
fn tells_build_tools_where_the_device_listens() {
    let home = tempfile::tempdir().unwrap();
    let (port, new_port, _) = restarting_device();
    let device = format!("127.0.0.1:{port}");
    let address = format!("127.0.0.1:{new_port}");

    let output = omni_build_at_home(
        home.path(),
        &[
            "tcpip",
            "-d",
            &device,
            "-p",
            &new_port.to_string(),
            "--json",
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let events = events(&output);
    assert_eq!(events[0]["event"], "key-created");
    assert_eq!(
        events[1..],
        [serde_json::json!({
            "event": "listening",
            "device": device,
            "model": "Pixel 8",
            "address": address,
            "message": format!("Pixel 8 ({device}) listens for ADB at {address} until it restarts. Unplug it, and install with --device {address}"),
        })]
    );
}

#[test]
fn tcpip_reports_what_went_wrong() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".android")).unwrap();
    fs::copy(ADB_KEY, home.path().join(".android/adbkey")).unwrap();
    let (port, _) = fake_services(vec![b"1: lo    inet 127.0.0.1/8 scope host lo\n".to_vec()]);
    let off_the_network = format!("127.0.0.1:{port}");
    let unreadable = tempfile::tempdir().unwrap();
    // A folder where the key should be.
    fs::create_dir_all(unreadable.path().join(".android/adbkey")).unwrap();
    let cases = [
        (
            home.path(),
            vec!["tcpip", "--device", "R5CT1234"],
            "error: found no device named \"R5CT1234\": ".to_owned(),
        ),
        (
            home.path(),
            vec!["tcpip", "--device", &off_the_network],
            format!(
                "error: Pixel 8 ({off_the_network}) isn't on Wi-Fi or Ethernet: connect it to the network this computer is on\n"
            ),
        ),
        (
            unreadable.path(),
            vec!["tcpip"],
            "error: couldn't read ".to_owned(),
        ),
    ];

    for (home, args, message) in cases {
        let output = omni_build_at_home(home, &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with(&message), "{}", stderr(&output));
    }
    // No device listens on port 0.
    let output = omni_build(&["tcpip", "--port", "0"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--port"), "{}", stderr(&output));
}

/// A device that pairs once for wireless debugging while it shows the code
/// 123456, as Android does, in its own words: over TLS, it runs SPAKE2's
/// server side, as BoringSSL has it, then sends its name,
/// adb-R5CT1234-a1B2c3, encrypted with the key that gives. It asks for no
/// certificate, which Android does but checks nothing of. Returns its
/// port, and the public key the computer sent.
fn pairing_device() -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let device = std::thread::spawn(move || {
        let config = ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_file(IDENTITY).unwrap()],
                PrivateKeyDer::from_pem_file(IDENTITY).unwrap(),
            )
            .unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut tls = StreamOwned::new(ServerConnection::new(Arc::new(config)).unwrap(), stream);
        tls.conn.complete_io(&mut tls.sock).unwrap();
        let exported = tls
            .conn
            .export_keying_material([0; 64], b"adb-label\0", None)
            .unwrap();

        // SPAKE2: each party's point masked with the password's hash times
        // a point of its own, M for the computer and N for the device.
        let point = |seed: &str| {
            let point = CompressedEdwardsY(Sha256::digest(seed).into())
                .decompress()
                .unwrap();
            point.mul_by_cofactor() * Scalar::from(8u8).invert()
        };
        let m = point("edwards25519 point generation seed (M)");
        let n = point("edwards25519 point generation seed (N)");
        let password_hash = Sha512::digest([&b"123456"[..], &exported].concat());
        let password = Scalar::from_bytes_mod_order_wide(&password_hash.into());
        let secret = Scalar::from_bytes_mod_order_wide(&[9; 64]);
        let ours = (EdwardsPoint::mul_base(&(secret * Scalar::from(8u8))) + n * password)
            .compress()
            .to_bytes();
        let theirs: [u8; 32] = read_pairing_packet(&mut tls).try_into().unwrap();
        write_pairing_packet(&mut tls, 0, &ours);
        let shared = ((CompressedEdwardsY(theirs).decompress().unwrap() - m * password)
            .mul_by_cofactor()
            * secret)
            .compress();
        let mut key = Sha512::new();
        for part in [
            &b"adb pair client\0"[..],
            b"adb pair server\0",
            &theirs,
            &ours,
            shared.as_bytes(),
            &password_hash,
        ] {
            key.update((part.len() as u64).to_le_bytes());
            key.update(part);
        }
        let mut aes_key = [0; 16];
        Hkdf::<Sha256>::new(None, &key.finalize())
            .expand(b"adb pairing_auth aes-128-gcm key", &mut aes_key)
            .unwrap();
        let cipher = Aes128Gcm::new(&aes_key.into());

        let mut info = vec![0; 8192];
        info[0] = 1;
        info[1..20].copy_from_slice(b"adb-R5CT1234-a1B2c3");
        let info = cipher.encrypt(&Nonce::default(), info.as_slice()).unwrap();
        let theirs = read_pairing_packet(&mut tls);
        write_pairing_packet(&mut tls, 1, &info);
        let info = cipher
            .decrypt(&Nonce::default(), theirs.as_slice())
            .unwrap();
        let key = info[1..].split(|&byte| byte == 0).next().unwrap();
        String::from_utf8(key.to_vec()).unwrap()
    });
    (port, device)
}

/// The payload of the computer's next pairing packet.
fn read_pairing_packet(tls: &mut StreamOwned<ServerConnection, TcpStream>) -> Vec<u8> {
    let mut header = [0; 6];
    tls.read_exact(&mut header).unwrap();
    let mut payload = vec![0; u32::from_be_bytes(header[2..].try_into().unwrap()) as usize];
    tls.read_exact(&mut payload).unwrap();
    payload
}

/// Sends a pairing packet of `kind`: its version, 1, its kind, its
/// payload's length, and its payload.
fn write_pairing_packet(
    tls: &mut StreamOwned<ServerConnection, TcpStream>,
    kind: u8,
    payload: &[u8],
) {
    let len = u32::try_from(payload.len()).unwrap().to_be_bytes();
    tls.write_all(&[&[1, kind], &len[..], payload].concat())
        .unwrap();
}

#[test]
fn pairs_with_a_device_for_wireless_debugging() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".android")).unwrap();
    fs::copy(ADB_KEY, home.path().join(".android/adbkey")).unwrap();
    let (port, device) = pairing_device();
    let address = format!("127.0.0.1:{port}");

    let output = omni_build_at_home(home.path(), &["pair", &address, "123456", "--json"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        events(&output),
        [serde_json::json!({
            "event": "paired",
            "address": address,
            "guid": "adb-R5CT1234-a1B2c3",
            "message": format!("Paired with the device at {address} (adb-R5CT1234-a1B2c3). Install on it with --device and the IP address and port it shows under Wireless debugging"),
        })]
    );
    // adb's public key, and then who it belongs to, which the device lists
    // the computer by.
    let public_key = fs::read_to_string(format!("{ADB_KEY}.pub")).unwrap();
    let sent = device.join().unwrap();
    assert_eq!(sent.split(' ').next(), public_key.split(' ').next());
}

#[test]
fn pair_reports_what_went_wrong() {
    let nothing_there = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let unreadable = tempfile::tempdir().unwrap();
    // A folder where the key should be.
    fs::create_dir_all(unreadable.path().join(".android/adbkey")).unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".android")).unwrap();
    fs::copy(ADB_KEY, home.path().join(".android/adbkey")).unwrap();
    let cases = [
        (
            home.path(),
            format!("error: couldn't pair with {nothing_there}: "),
        ),
        (unreadable.path(), "error: couldn't read ".to_owned()),
    ];

    for (home, message) in cases {
        let output = omni_build_at_home(home, &["pair", &nothing_there.to_string(), "123456"]);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with(&message), "{}", stderr(&output));
    }
}

/// The test App Store Connect API key.
const API_KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/AuthKey_TESTKEY123.p8"
);
/// The test identity in the keystore Keychain Access exports, with the
/// password `omnibuild`.
const KEYCHAIN_KEYSTORE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/keystores/keychain.p12"
);

/// A made-up App Store Connect, whose account has all the shell needs: the
/// test identity's certificate, an App ID for each bundle, and the test
/// profiles. It refuses to create anything more. Returns its URL.
fn app_store_connect() -> String {
    use base64::Engine;
    use std::io::{BufRead, BufReader};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let certificate = encode(&CertificateDer::from_pem_file(IDENTITY).unwrap());
    let profile = encode(&fs::read(PROFILE).unwrap());
    let clip_profile = encode(&fs::read(CLIP_PROFILE).unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut reader = BufReader::new(stream.unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request = line.clone();
            // The headers, up to the line that ends them, and the body.
            let mut length = 0;
            while line != "\r\n" {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let mut parts = request.split(' ');
            let (method, target) = (parts.next().unwrap(), parts.next().unwrap());
            let (path, query) = target.split_once('?').unwrap_or((target, ""));
            let (status, data) = if method == "POST" {
                let errors =
                    serde_json::json!([{ "title": "Refused.", "detail": "The test says so." }]);
                (409, serde_json::json!({ "errors": errors }))
            } else if path == "/v1/certificates" {
                (
                    200,
                    serde_json::json!({ "data": [{ "type": "certificates", "id": "C1", "attributes": { "certificateType": "DISTRIBUTION", "certificateContent": certificate } }] }),
                )
            } else if path == "/v1/bundleIds" {
                // The filter comes first: `filter[identifier]=<ID>`.
                let id = query.split('&').next().unwrap().split_once('=').unwrap().1;
                (
                    200,
                    serde_json::json!({ "data": [{ "type": "bundleIds", "id": id, "attributes": { "identifier": id } }] }),
                )
            } else {
                // A bundle's profiles, at /v1/bundleIds/<ID>/profiles.
                let content = if path.ends_with(".Clip/profiles") {
                    &clip_profile
                } else {
                    &profile
                };
                (
                    200,
                    serde_json::json!({ "data": [{ "type": "profiles", "id": "P1", "attributes": { "profileType": "IOS_APP_STORE", "profileState": "ACTIVE", "profileContent": content } }] }),
                )
            };
            let body = data.to_string();
            write!(
                reader.get_mut(),
                "HTTP/1.1 {status} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    url
}

/// Runs omni at `home` with the test keystore's password, against App
/// Store Connect at `url`.
fn omni_build_provisioning(home: &Path, url: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("OMNI_BUILD_KEY_PASSWORD", "omnibuild")
        .env("OMNI_BUILD_APP_STORE_CONNECT_URL", url)
        .output()
        .expect("the omni binary should start")
}

#[test]
fn provisions_an_ios_app_and_signs_it_with_what_it_got() {
    let home = tempfile::tempdir().unwrap();
    let keystore = home.path().join("apple.p12");
    fs::copy(KEYCHAIN_KEYSTORE, &keystore).unwrap();
    let profiles = home.path().join("profiles");
    let ipa = home.path().join("app.ipa");
    let url = app_store_connect();

    let provisioned = omni_build_provisioning(
        home.path(),
        &url,
        &[
            "provision",
            SHELL_IPA,
            "--signing-key",
            keystore.to_str().unwrap(),
            "--api-key",
            API_KEY,
            "--api-issuer",
            "issuer",
            "--out",
            profiles.to_str().unwrap(),
            "--json",
        ],
    );

    assert!(provisioned.status.success(), "{}", stderr(&provisioned));
    let events = events(&provisioned);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "provisioned");
    assert_eq!(events[0]["certificate"], "OmniBuild Test");
    assert_eq!(events[0]["distribution"], "app-store");
    assert_eq!(events[0]["out"], profiles.to_str().unwrap());
    let mut written: Vec<_> = fs::read_dir(&profiles)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    written.sort();
    assert_eq!(
        written,
        [
            "dev.omnibuild.shell.Clip.mobileprovision",
            "dev.omnibuild.shell.ShellIntents.mobileprovision",
            "dev.omnibuild.shell.ShellNotifications.mobileprovision",
            "dev.omnibuild.shell.mobileprovision",
            "dev.omnibuild.shell.watchkitapp.ShellWatchWidgets.mobileprovision",
            "dev.omnibuild.shell.watchkitapp.mobileprovision",
        ]
    );
    assert_eq!(
        fs::read(profiles.join("dev.omnibuild.shell.Clip.mobileprovision")).unwrap(),
        fs::read(CLIP_PROFILE).unwrap()
    );
    // What isn't a profile, beside them, is left out.
    fs::write(profiles.join("README.txt"), "Profiles").unwrap();

    let repackaged = omni_build_provisioning(
        home.path(),
        &url,
        &[
            "repackage",
            SHELL_IPA,
            "--signing-key",
            keystore.to_str().unwrap(),
            "--profile",
            profiles.to_str().unwrap(),
            "--out",
            ipa.to_str().unwrap(),
        ],
    );

    assert!(repackaged.status.success(), "{}", stderr(&repackaged));
}

#[test]
fn provision_reports_what_went_wrong() {
    let home = tempfile::tempdir().unwrap();
    let keystore = home.path().join("apple.p12");
    fs::copy(KEYCHAIN_KEYSTORE, &keystore).unwrap();
    let keystore = keystore.to_str().unwrap();
    let not_ipa = home.path().join("not.ipa");
    fs::write(&not_ipa, "not a zip").unwrap();
    let file = home.path().join("file");
    fs::write(&file, "a file").unwrap();
    // A folder where the app's profile should be written.
    let taken = home.path().join("taken");
    fs::create_dir_all(taken.join("dev.omnibuild.shell.mobileprovision")).unwrap();
    let url = app_store_connect();
    let nothing_there = format!(
        "http://{}",
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    );
    let new_keystore = home.path().join("new.p12");
    let provision_with = |shell: &str, out: &Path, api_key: &str, keystore: &str| {
        [
            "provision",
            shell,
            "--signing-key",
            keystore,
            "--api-key",
            api_key,
            "--out",
            out.to_str().unwrap(),
        ]
        .map(str::to_owned)
        .to_vec()
    };
    let provision = |shell: &str, out: &Path| provision_with(shell, out, API_KEY, keystore);
    let out = home.path().join("profiles");
    let cases = [
        (
            &url,
            provision("missing.ipa", &out),
            "error: couldn't read the shell missing.ipa: ".to_owned(),
        ),
        (
            &url,
            [provision(SHELL_IPA, &out), vec!["--config".to_owned(), "missing.json".to_owned()]].concat(),
            "error: couldn't read the app config missing.json: ".to_owned(),
        ),
        (
            &url,
            provision(not_ipa.to_str().unwrap(), &out),
            "error: the shell isn't an APK, an AAB or an iOS app: it isn't a zip archive\n"
                .to_owned(),
        ),
        (
            &url,
            provision(SHELL, &out),
            format!("error: {SHELL} isn't an iOS app, which is what provisioning profiles sign\n"),
        ),
        (
            &url,
            provision(SHELL_APP, &out),
            "error: only a build for iOS devices, an .ipa, is signed with provisioning profiles, and the shell's CFBundleSupportedPlatforms is [iPhoneSimulator]\n".to_owned(),
        ),
        (
            &url,
            provision_with(SHELL_IPA, &out, "missing.p8", keystore),
            "error: couldn't read the API key missing.p8: ".to_owned(),
        ),
        (
            &nothing_there,
            provision(SHELL_IPA, &out),
            "error: couldn't list the account's certificates: couldn't reach App Store Connect: "
                .to_owned(),
        ),
        (
            &url,
            provision(SHELL_IPA, &file),
            "error: couldn't create the folder ".to_owned(),
        ),
        (
            &url,
            provision(SHELL_IPA, &taken),
            "error: couldn't write the provisioning profile to ".to_owned(),
        ),
        // It says it created the key, then why Apple didn't issue its
        // certificate.
        (
            &url,
            provision_with(SHELL_IPA, &out, API_KEY, new_keystore.to_str().unwrap()),
            format!(
                "Created a key for an Apple signing certificate in {}. Keep it, and its password, safe: the certificate Apple issues for it signs nothing without it.\nerror: couldn't create a certificate for the key in {}: App Store Connect refused it (409). It said: Refused. The test says so.\n",
                new_keystore.display(),
                new_keystore.display()
            ),
        ),
    ];

    for (url, args, message) in cases {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = omni_build_provisioning(home.path(), url, &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with(&message), "{}", stderr(&output));
    }
}

#[cfg(unix)]
#[test]
fn repackage_reports_a_folder_of_profiles_it_cant_read() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let profiles = home.path().join("profiles");
    fs::create_dir(&profiles).unwrap();
    fs::set_permissions(&profiles, fs::Permissions::from_mode(0o000)).unwrap();

    let output = omni_build_at_home(
        home.path(),
        &[
            "repackage",
            SHELL_IPA,
            "--signing-key",
            IDENTITY,
            "--profile",
            profiles.to_str().unwrap(),
            "--out",
            "app.ipa",
        ],
    );

    fs::set_permissions(&profiles, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("error: couldn't read the folder "),
        "{}",
        stderr(&output)
    );
}

/// A made-up Google, which issues the test service account an access
/// token, and whose Google Play takes any app's bundle on any track but
/// `gone`.
/// Returns its URL, and each request it received, as its method and path.
fn google() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut reader = BufReader::new(stream.unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request = line.trim_end().rsplit_once(' ').unwrap().0.to_owned();
            let mut length = 0;
            while line != "\r\n" {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let path = request.split(['?', ' ']).nth(1).unwrap();
            let (status, body) = if path.ends_with("/tracks/gone") {
                let error = serde_json::json!({ "code": 404, "message": "Track not found: gone." });
                (404, serde_json::json!({ "error": error }))
            } else if path == "/token" {
                let token =
                    serde_json::json!({ "access_token": "ya29.access", "token_type": "Bearer" });
                (200, token)
            } else if path.ends_with("/bundles") {
                (200, serde_json::json!({ "versionCode": 1 }))
            } else {
                (200, serde_json::json!({ "id": "E1" }))
            };
            let body = body.to_string();
            received.lock().unwrap().push(request);
            write!(
                reader.get_mut(),
                "HTTP/1.1 {status} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (url, requests)
}

/// A service account's key, as Google Cloud gives one, with the test
/// identity's private key, written to `dir`, asking `url` for its access
/// tokens. It's made as tests run, rather than kept as a fixture, which
/// GitHub's push protection would take for a real one.
fn service_account(dir: &Path, url: &str) -> String {
    let pem = fs::read_to_string(IDENTITY).unwrap();
    let end = "-----END PRIVATE KEY-----";
    let key = serde_json::json!({
        "type": "service_account",
        "private_key_id": "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c",
        "private_key": &pem[..pem.find(end).unwrap() + end.len()],
        "client_email": "upload@omnibuild-test.iam.gserviceaccount.com",
        "token_uri": format!("{url}/token"),
    });
    let path = dir.join("service-account.json");
    fs::write(&path, key.to_string()).unwrap();
    path.to_str().unwrap().to_owned()
}

/// Runs omni against Google Play's API at `url`, and App Store Connect's
/// nowhere it can reach.
fn omni_build_uploading(url: &str, args: &[&str]) -> Output {
    let nothing_there = format!(
        "http://{}",
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    );
    Command::new(env!("CARGO_BIN_EXE_omni"))
        .args(args)
        .env("OMNI_BUILD_GOOGLE_PLAY_URL", url)
        .env("OMNI_BUILD_APP_STORE_CONNECT_URL", nothing_there)
        .output()
        .expect("the omni binary should start")
}

#[test]
fn uploads_an_app_bundle_to_google_play() {
    let dir = tempfile::tempdir().unwrap();
    let (url, requests) = google();
    let key = service_account(dir.path(), &url);

    let output = omni_build_uploading(
        &url,
        &[
            "upload",
            SHELL_AAB,
            "--service-account",
            &key,
            "--track",
            "beta",
            "--json",
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        events(&output),
        [serde_json::json!({
            "event": "uploaded",
            "app": SHELL_AAB,
            "id": "dev.omnibuild.shell",
            "version": "1.0.0",
            "build": "1",
            "store": "google-play",
            "track": "beta",
            "draft": false,
            "message": format!("Released {SHELL_AAB}, dev.omnibuild.shell 1.0.0 (1), to Google Play's beta track"),
        })]
    );
    let app = "/androidpublisher/v3/applications/dev.omnibuild.shell";
    assert_eq!(
        *requests.lock().unwrap(),
        [
            "POST /token".to_owned(),
            format!("POST {app}/edits"),
            format!("POST /upload{app}/edits/E1/bundles?uploadType=media"),
            format!("PUT {app}/edits/E1/tracks/beta"),
            format!("POST {app}/edits/E1:commit"),
        ]
    );
}

#[test]
fn upload_reports_what_went_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let not_zip = dir.path().join("not.aab");
    fs::write(&not_zip, "not a zip").unwrap();
    let not_zip = not_zip.to_str().unwrap();
    let (url, _) = google();
    let nothing_there = format!(
        "http://{}",
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    );
    let unreachable = service_account(dir.path(), &nothing_there);
    let reachable = dir.path().join("reachable");
    fs::create_dir(&reachable).unwrap();
    let key = service_account(&reachable, &url);
    // An app bundle without a manifest, under another name.
    let no_manifest = dir.path().join("no-manifest.aab");
    let mut bundle = fs::read(SHELL_AAB).unwrap();
    let name = b"base/manifest/AndroidManifest.xml";
    while let Some(at) = bundle.windows(name.len()).position(|window| window == name) {
        bundle[at + name.len() - 1] = b'_';
    }
    fs::write(&no_manifest, bundle).unwrap();
    let no_manifest = no_manifest.to_str().unwrap();
    let cases = [
        (
            vec!["upload", "missing.aab", "--service-account", &key],
            "error: couldn't read the app missing.aab: ".to_owned(),
        ),
        (
            vec!["upload", SHELL, "--service-account", &key],
            format!(
                "error: {SHELL} is an APK, and Google Play takes app bundles (AAB), from which it makes the APKs each device installs: repackage an AAB shell to upload the app\n"
            ),
        ),
        (
            vec!["upload", not_zip, "--service-account", &key],
            format!(
                "error: {not_zip} isn't an app bundle (AAB), or an iOS app built for devices (an .ipa)\n"
            ),
        ),
        (
            vec!["upload", SHELL_APP, "--api-key", API_KEY],
            "error: App Store Connect takes builds for iOS devices, and the app's CFBundleSupportedPlatforms is [iPhoneSimulator]\n".to_owned(),
        ),
        (
            vec!["upload", SHELL_IPA, "--service-account", &key],
            "error: an upload to App Store Connect needs an API key: give it with --api-key, and a team key's issuer ID with --api-issuer\n".to_owned(),
        ),
        (
            vec!["upload", SHELL_IPA, "--api-key", "missing.p8"],
            "error: couldn't read the API key missing.p8: ".to_owned(),
        ),
        (
            vec!["upload", SHELL_IPA, "--api-key", API_KEY],
            "error: couldn't list the account's apps: couldn't reach App Store Connect: ".to_owned(),
        ),
        (
            vec!["upload", SHELL_AAB],
            "error: give the key of the Google Cloud service account to upload as with --service-account\n".to_owned(),
        ),
        (
            vec!["upload", SHELL_AAB, "--service-account", "missing.json"],
            "error: couldn't read the service account's key missing.json: ".to_owned(),
        ),
        (
            vec!["upload", no_manifest, "--service-account", &key],
            "error: the app has no base/manifest/AndroidManifest.xml\n".to_owned(),
        ),
        (
            vec!["upload", SHELL_AAB, "--service-account", &unreachable],
            "error: couldn't reach Google: ".to_owned(),
        ),
        (
            vec!["upload", SHELL_AAB, "--service-account", &key, "--track", "gone"],
            "error: couldn't release it to the gone track: Google Play refused it (404). It said: Track not found: gone.\n".to_owned(),
        ),
    ];

    for (args, message) in cases {
        let output = omni_build_uploading(&url, &args);

        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with(&message), "{}", stderr(&output));
    }
}

/// A made-up App Store Connect, whose account has an app,
/// `dev.omnibuild.shell`, and takes an upload of it in one part, which it
/// takes at once, with a warning. Returns its URL, and each request it
/// received, as its method and path.
fn app_store_connect_for_uploads() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = Arc::clone(&requests);
    let own = url.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut reader = BufReader::new(stream.unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request = line.trim_end().rsplit_once(' ').unwrap().0.to_owned();
            let mut length = 0;
            while line != "\r\n" {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let path = request.split(['?', ' ']).nth(1).unwrap();
            let data = match path {
                "/v1/apps" => serde_json::json!([
                    { "type": "apps", "id": "A1", "attributes": { "bundleId": "dev.omnibuild.shell" } },
                ]),
                "/v1/buildUploads" => {
                    serde_json::json!({ "type": "buildUploads", "id": "U1", "attributes": {} })
                }
                "/v1/buildUploadFiles" => {
                    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let operation = serde_json::json!({
                        "method": "PUT",
                        "url": format!("{own}/storage"),
                        "offset": 0,
                        "length": body["data"]["attributes"]["fileSize"],
                    });
                    let attributes = serde_json::json!({ "uploadOperations": [operation] });
                    serde_json::json!({ "type": "buildUploadFiles", "id": "F1", "attributes": attributes })
                }
                "/v1/buildUploads/U1" => {
                    let warning = serde_json::json!({ "code": "90078", "description": "Missing Push Notification Entitlement." });
                    let state = serde_json::json!({ "state": "COMPLETE", "warnings": [warning] });
                    serde_json::json!({ "type": "buildUploads", "id": "U1", "attributes": { "state": state } })
                }
                // The upload's file, committed, and its part, stored.
                _ => {
                    serde_json::json!({ "type": "buildUploadFiles", "id": "F1", "attributes": {} })
                }
            };
            let body = serde_json::json!({ "data": data }).to_string();
            received.lock().unwrap().push(request);
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (url, requests)
}

#[test]
fn uploads_an_ios_app_to_app_store_connect() {
    let (url, requests) = app_store_connect_for_uploads();

    let output = Command::new(env!("CARGO_BIN_EXE_omni"))
        .args([
            "upload",
            SHELL_IPA,
            "--api-key",
            API_KEY,
            "--api-issuer",
            "issuer",
            "--json",
        ])
        .env("OMNI_BUILD_APP_STORE_CONNECT_URL", &url)
        .output()
        .expect("the omni binary should start");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        events(&output),
        [
            serde_json::json!({
                "event": "upload-warning",
                "code": "90078",
                "description": "Missing Push Notification Entitlement.",
                "message": "App Store Connect warns: Missing Push Notification Entitlement. (90078)",
            }),
            serde_json::json!({
                "event": "uploaded",
                "app": SHELL_IPA,
                "id": "dev.omnibuild.shell",
                "version": "1.0.0",
                "build": "1",
                "store": "app-store-connect",
                "message": format!("Uploaded {SHELL_IPA}, dev.omnibuild.shell 1.0.0 (1), to App Store Connect, which took it: TestFlight has it for testers once Apple has processed it"),
            }),
        ]
    );
    assert_eq!(
        *requests.lock().unwrap(),
        [
            "GET /v1/apps?filter%5BbundleId%5D=dev.omnibuild.shell&limit=200",
            "POST /v1/buildUploads",
            "POST /v1/buildUploadFiles",
            "PUT /storage",
            "PATCH /v1/buildUploadFiles/F1",
            "GET /v1/buildUploads/U1",
        ]
    );
}
