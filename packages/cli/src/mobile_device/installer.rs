//! The installation proxy, the service through which a computer installs
//! apps on an iPhone or iPad: given the path of an app staged on the device,
//! it has the device check and install it, and reports how that goes until
//! it's done, or why it failed.

use anyhow::{Result, bail};
use plist::{Dictionary, Value};

use super::{Stream, receive, send};

/// Installs the app staged at `path` on the device, through the
/// installation proxy at the other end of `stream`, replacing any with its
/// ID, and waits until it's installed.
pub fn install(mut stream: Box<dyn Stream>, path: &str) -> Result<()> {
    let mut request = Dictionary::new();
    request.insert("Command".to_owned(), "Install".into());
    request.insert("PackagePath".to_owned(), path.into());
    request.insert("ClientOptions".to_owned(), Dictionary::new().into());
    send(&mut stream, &request)?;
    // What it reports as it goes: a status and how far it is, for each step.
    loop {
        let answer = receive(&mut stream)?;
        let text = |key: &str| answer.get(key).and_then(Value::as_string);
        if let Some(error) = text("Error") {
            match text("ErrorDescription") {
                Some(description) => bail!("the device refused the app: {description} ({error})"),
                None => bail!("the device refused the app ({error})"),
            }
        }
        if text("Status") == Some("Complete") {
            return Ok(());
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use plist::Dictionary;

    use super::install;
    use crate::mobile_device::tests::{read_plist, write_plist};
    use crate::mobile_device::usbmux::tests::dictionary;

    /// Answers an install request on `stream` as a device does, with
    /// `answers`, and returns the request.
    pub fn serve_installer(mut stream: impl Read + Write, answers: &[Dictionary]) -> Dictionary {
        let request = read_plist(&mut stream);
        for answer in answers {
            write_plist(&mut stream, answer);
        }
        request
    }

    fn installing(answers: Vec<Dictionary>) -> (anyhow::Result<()>, Dictionary) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let device = thread::spawn(move || serve_installer(listener.accept().unwrap().0, &answers));
        let result = install(
            Box::new(TcpStream::connect(address).unwrap()),
            "PublicStaging/app.ipa",
        );
        (result, device.join().unwrap())
    }

    pub fn progress(status: &str, percent: u32) -> Dictionary {
        dictionary(&format!(
            "<dict><key>Status</key><string>{status}</string><key>PercentComplete</key><integer>{percent}</integer></dict>"
        ))
    }

    #[test]
    fn installs_the_staged_app_and_waits_until_its_done() {
        let (result, request) = installing(vec![
            progress("CreatingStagingDirectory", 5),
            progress("VerifyingApplication", 40),
            dictionary("<dict><key>Status</key><string>Complete</string></dict>"),
        ]);

        result.unwrap();
        assert_eq!(
            request,
            dictionary(
                "<dict><key>Command</key><string>Install</string><key>PackagePath</key><string>PublicStaging/app.ipa</string><key>ClientOptions</key><dict/></dict>"
            )
        );
    }

    #[test]
    fn says_why_the_device_refused_the_app() {
        let (described, _) = installing(vec![
            progress("VerifyingApplication", 40),
            dictionary(
                "<dict><key>Error</key><string>ApplicationVerificationFailed</string><key>ErrorDescription</key><string>Failed to verify code signature of /private/var/installd/Library/Caches/com.apple.mobile.installd.staging/temp.ipa/extracted/Payload/App.app : 0xe8008015 (A valid provisioning profile for this executable was not found.)</string></dict>",
            ),
        ]);
        let (bare, _) = installing(vec![dictionary(
            "<dict><key>Error</key><string>DeviceOSVersionTooLow</string></dict>",
        )]);
        let (cut_off, _) = installing(vec![progress("VerifyingApplication", 40)]);

        assert_eq!(
            format!("{:#}", described.unwrap_err()),
            "the device refused the app: Failed to verify code signature of /private/var/installd/Library/Caches/com.apple.mobile.installd.staging/temp.ipa/extracted/Payload/App.app : 0xe8008015 (A valid provisioning profile for this executable was not found.) (ApplicationVerificationFailed)"
        );
        assert_eq!(
            format!("{:#}", bare.unwrap_err()),
            "the device refused the app (DeviceOSVersionTooLow)"
        );
        assert!(cut_off.is_err());
        let mut broken = crate::mobile_device::tests::Broken;
        assert!(broken.flush().is_ok());
        assert!(install(Box::new(broken), "PublicStaging/app.ipa").is_err());
    }
}
