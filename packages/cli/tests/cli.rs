//! Runs the built `omni-build` binary as a user would. Besides checking the
//! command line end to end, this is what covers `main()`: cargo-llvm-cov
//! counts binaries that tests start, but no unit test can call `main()`.

use std::process::{Command, Output};

fn omni_build(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omni-build"))
        .args(args)
        .output()
        .expect("the omni-build binary should start")
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
