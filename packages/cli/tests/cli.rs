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
