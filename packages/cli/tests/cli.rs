//! Runs the built `omni-build` binary as a user would. Besides checking the
//! command line end to end, this is what covers `main()`: cargo-llvm-cov
//! counts binaries that tests start, but no unit test can call `main()`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const SHELL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.apk");
const SHELL_APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.app.zip");
const SHELL_IPA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.ipa");
/// A key and certificate, and a provisioning profile allowing it: see
/// tests/fixtures/README.md.
const IDENTITY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/identity.pem");
const PROFILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/profile.mobileprovision"
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
            "--profile",
            PROFILE,
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
    assert!(contains(
        &fs::read(ipa).unwrap(),
        b"Payload/OmniBuildShell.app/embedded.mobileprovision"
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
