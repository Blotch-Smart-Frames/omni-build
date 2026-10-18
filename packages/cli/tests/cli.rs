//! Runs the built `omni-build` binary as a user would. Besides checking the
//! command line end to end, this is what covers `main()`: cargo-llvm-cov
//! counts binaries that tests start, but no unit test can call `main()`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const SHELL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shell.apk");

fn omni_build(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omni-build"))
        .args(args)
        .output()
        .expect("the omni-build binary should start")
}

/// Runs omni-build with `home` as the user's home folder, where it keeps
/// its debug key.
fn omni_build_at_home(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omni-build"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
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
fn repackage_reports_what_went_wrong() {
    let home = tempfile::tempdir().unwrap();
    let not_apk = home.path().join("not.apk");
    fs::write(&not_apk, "not a zip").unwrap();
    let nowhere = home.path().join("missing/app.apk");
    let cases = [
        (
            vec!["repackage", "missing.apk", "--out", "app.apk"],
            "error: couldn't read the shell missing.apk: ",
        ),
        (
            vec!["repackage", not_apk.to_str().unwrap(), "--out", "app.apk"],
            "error: the shell isn't an APK: it isn't a zip archive\n",
        ),
        (
            vec!["repackage", SHELL, "--out", nowhere.to_str().unwrap()],
            "error: couldn't write the app to ",
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
