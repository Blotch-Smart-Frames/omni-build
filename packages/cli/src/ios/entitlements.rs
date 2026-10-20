//! An app's entitlements: what iOS lets it do beyond the sandbox, such as
//! being debugged. Its signature carries them twice, as codesign signs
//! them: as an XML property list, and in DER, the form iOS 15 and later
//! check.

use std::collections::BTreeMap;

use plist::{Dictionary, Value};

/// The entitlements OmniBuild signs with, which are strings, booleans and
/// arrays of strings. In order of their keys, which is the order DER needs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Entitlements(BTreeMap<String, Entitlement>);

#[derive(Debug, PartialEq, Eq)]
pub enum Entitlement {
    String(String),
    Boolean(bool),
    /// Such as the App IDs an App Clip and its app name each other by.
    Strings(Vec<String>),
}

impl Entitlements {
    pub fn insert(&mut self, key: &str, value: Entitlement) {
        self.0.insert(key.to_owned(), value);
    }

    pub fn get(&self, key: &str) -> Option<&Entitlement> {
        self.0.get(key)
    }

    /// As codesign writes them: Apple's XML property list, ending in a new
    /// line.
    pub fn xml(&self) -> Vec<u8> {
        let dictionary: Dictionary = self
            .0
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Entitlement::String(string) => Value::String(string.clone()),
                    Entitlement::Boolean(boolean) => Value::Boolean(*boolean),
                    Entitlement::Strings(strings) => {
                        Value::Array(strings.iter().cloned().map(Value::String).collect())
                    }
                };
                (key.clone(), value)
            })
            .collect();
        let mut xml = Vec::new();
        Value::Dictionary(dictionary)
            .to_writer_xml(&mut xml)
            .expect("writing into memory can't fail");
        xml.push(b'\n');
        xml
    }

    /// As Apple's `CoreEntitlements` reads them: version 1, then the keys
    /// and values, each pair a sequence, in a context-specific `[16]`, all
    /// in an application-specific `[16]`.
    pub fn der(&self) -> Vec<u8> {
        let pairs: Vec<u8> = self
            .0
            .iter()
            .flat_map(|(key, value)| {
                let value = match value {
                    Entitlement::String(string) => der(UTF8_STRING, string.as_bytes()),
                    Entitlement::Boolean(boolean) => {
                        der(BOOLEAN, &[if *boolean { 0xff } else { 0 }])
                    }
                    // A sequence of its strings, as codesign writes one.
                    Entitlement::Strings(strings) => der(
                        SEQUENCE,
                        &strings
                            .iter()
                            .flat_map(|string| der(UTF8_STRING, string.as_bytes()))
                            .collect::<Vec<u8>>(),
                    ),
                };
                der(
                    SEQUENCE,
                    &[der(UTF8_STRING, key.as_bytes()), value].concat(),
                )
            })
            .collect();
        der(
            APPLICATION_16,
            &[der(INTEGER, &[1]), der(CONTEXT_16, &pairs)].concat(),
        )
    }
}

const BOOLEAN: u8 = 0x01;
const INTEGER: u8 = 0x02;
const UTF8_STRING: u8 = 0x0c;
const SEQUENCE: u8 = 0x30;
/// Constructed, application class, tag 16.
const APPLICATION_16: u8 = 0x70;
/// Constructed, context-specific class, tag 16.
const CONTEXT_16: u8 = 0xb0;

/// A DER element: its tag, its length (in the short form under 128, else
/// the long form), and its contents.
fn der(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut element = vec![tag];
    match u8::try_from(contents.len()) {
        Ok(length) if length < 0x80 => element.push(length),
        _ => {
            let length = contents.len().to_be_bytes();
            let significant = &length[length.iter().take_while(|&&byte| byte == 0).count()..];
            element.push(0x80 | u8::try_from(significant.len()).expect("a usize has 8 bytes"));
            element.extend(significant);
        }
    }
    element.extend(contents);
    element
}

#[cfg(test)]
pub mod tests {
    use super::{Entitlement, Entitlements, der};

    /// What OmniBuild signs an app with from the test provisioning profile
    /// (tests/fixtures/profile.mobileprovision), for the shell's bundle ID.
    pub fn shell_entitlements() -> Entitlements {
        let mut entitlements = Entitlements::default();
        entitlements.insert(
            "application-identifier",
            Entitlement::String("ABCDE12345.dev.omnibuild.shell".to_owned()),
        );
        entitlements.insert(
            "com.apple.developer.team-identifier",
            Entitlement::String("ABCDE12345".to_owned()),
        );
        entitlements.insert("get-task-allow", Entitlement::Boolean(true));
        entitlements
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn writes_xml_as_codesign_does() {
        assert_eq!(
            String::from_utf8(shell_entitlements().xml()).unwrap(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>application-identifier</key>
	<string>ABCDE12345.dev.omnibuild.shell</string>
	<key>com.apple.developer.team-identifier</key>
	<string>ABCDE12345</string>
	<key>get-task-allow</key>
	<true/>
</dict>
</plist>
"#
        );
    }

    #[test]
    fn writes_der_as_codesign_does() {
        // codesign's DER entitlements blob for the same entitlements, after
        // its magic and length.
        assert_eq!(
            shell_entitlements().der(),
            hex(concat!(
                "708188020101b0818230380c166170706c69636174696f6e2d6964656e746966",
                "6965720c1e414243444531323334352e6465762e6f6d6e696275696c642e7368",
                "656c6c30310c23636f6d2e6170706c652e646576656c6f7065722e7465616d2d",
                "6964656e7469666965720c0a4142434445313233343530130c0e6765742d7461",
                "736b2d616c6c6f770101ff"
            ))
        );
    }

    #[test]
    fn writes_arrays_as_codesign_does() {
        // An App Clip's, and arrays of two strings and of none.
        let mut entitlements = Entitlements::default();
        entitlements.insert(
            "application-identifier",
            Entitlement::String("ABCDE12345.dev.omnibuild.shell.Clip".to_owned()),
        );
        entitlements.insert(
            "com.apple.developer.on-demand-install-capable",
            Entitlement::Boolean(true),
        );
        entitlements.insert(
            "com.apple.developer.parent-application-identifiers",
            Entitlement::Strings(vec!["ABCDE12345.dev.omnibuild.shell".to_owned()]),
        );
        entitlements.insert(
            "z.two",
            Entitlement::Strings(vec!["a".to_owned(), "b".to_owned()]),
        );
        entitlements.insert("z.zero", Entitlement::Strings(Vec::new()));

        // What codesign wrote for the same entitlements, given as a file.
        assert_eq!(
            String::from_utf8(entitlements.xml()).unwrap(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>application-identifier</key>
	<string>ABCDE12345.dev.omnibuild.shell.Clip</string>
	<key>com.apple.developer.on-demand-install-capable</key>
	<true/>
	<key>com.apple.developer.parent-application-identifiers</key>
	<array>
		<string>ABCDE12345.dev.omnibuild.shell</string>
	</array>
	<key>z.two</key>
	<array>
		<string>a</string>
		<string>b</string>
	</array>
	<key>z.zero</key>
	<array/>
</dict>
</plist>
"#
        );
        assert_eq!(
            entitlements.der(),
            hex(concat!(
                "7081ee020101b081e8303d0c166170706c69636174696f6e2d6964656e746966",
                "6965720c23414243444531323334352e6465762e6f6d6e696275696c642e7368",
                "656c6c2e436c697030320c2d636f6d2e6170706c652e646576656c6f7065722e",
                "6f6e2d64656d616e642d696e7374616c6c2d63617061626c650101ff30560c32",
                "636f6d2e6170706c652e646576656c6f7065722e706172656e742d6170706c69",
                "636174696f6e2d6964656e7469666965727330200c1e41424344453132333435",
                "2e6465762e6f6d6e696275696c642e7368656c6c300f0c057a2e74776f30060c",
                "01610c0162300a0c067a2e7a65726f3000"
            ))
        );
    }

    #[test]
    fn writes_false_and_lengths_of_every_size() {
        let mut entitlements = Entitlements::default();
        entitlements.insert("get-task-allow", Entitlement::Boolean(false));

        assert_eq!(
            entitlements.der()[7..],
            hex("30130c0e6765742d7461736b2d616c6c6f77010100")
        );
        assert_eq!(
            entitlements.get("get-task-allow"),
            Some(&Entitlement::Boolean(false))
        );
        assert_eq!(der(0x04, &[0; 0x7f])[..2], [0x04, 0x7f]);
        assert_eq!(der(0x04, &[0; 0x80])[..3], [0x04, 0x81, 0x80]);
        assert_eq!(der(0x04, &[0; 0x1234])[..4], [0x04, 0x82, 0x12, 0x34]);
    }
}
