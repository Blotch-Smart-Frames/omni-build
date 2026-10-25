//! Pairing for wireless debugging, as `adb pair` does it
//! (`pairing_connection.cpp`): over TLS, the computer and the device run
//! SPAKE2 with the code the device shows, then tell each other who they
//! are, encrypted with the key that gives them. The computer sends its ADB
//! public key, which the device trusts from then on, and the device its
//! name for itself, its GUID.

use std::io::Write;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use anyhow::{Context, Result, anyhow, bail};
use hkdf::Hkdf;
use sha2::Sha256;

use super::connection::{Stream, read};
use super::key::AdbKey;
use super::spake2::{Role, Spake2};
use super::tls::{self, Tls};

/// The version of the protocol's packets, the only one there is.
const VERSION: u8 = 1;
/// The kinds of packet: a SPAKE2 message, and a party's encrypted peer
/// info, which says who it is.
const SPAKE2_MESSAGE: u8 = 0;
const PEER_INFO: u8 = 1;
/// A peer info's size: a byte for its kind, and its value, padded with
/// zeros.
const PEER_INFO_SIZE: usize = 8192;
/// The kinds of peer info: the computer's ADB public key, and the device's
/// GUID.
const PUBLIC_KEY: u8 = 0;
const GUID: u8 = 1;
/// The largest payload a packet has, as adb has it.
const MAX_PAYLOAD: usize = 2 * PEER_INFO_SIZE;
/// What the TLS connection exports keying material for the password under:
/// adb's label, with C's terminating NUL, which adb counts in.
const EXPORTER_LABEL: &[u8] = b"adb-label\0";
/// What HKDF derives the AES-128-GCM key from SPAKE2's under.
const KEY_INFO: &[u8] = b"adb pairing_auth aes-128-gcm key";

/// Pairs with the device at the other end of `stream`, given `code`, the
/// one it shows, so that it trusts `key`. Returns the device's GUID.
pub fn pair(stream: Box<dyn Stream + '_>, key: &AdbKey, code: &str) -> Result<String> {
    let mut tls = tls::connect(stream, key)?;
    // Keying material from this TLS connection goes into the password
    // after the code, so the exchange holds only over this connection, and
    // nobody in the middle can pass it on.
    let exported = tls
        .conn
        .export_keying_material([0; 64], EXPORTER_LABEL, None)
        .expect("the handshake is done");
    let spake2 = Spake2::new(Role::Client, &[code.as_bytes(), &exported].concat());
    let theirs = exchange(&mut tls, SPAKE2_MESSAGE, &spake2.message())?;
    let cipher = spake2
        .finish(&theirs)
        .map(|shared| cipher(&shared))
        .ok_or_else(|| anyhow!("the device's SPAKE2 message isn't a point on the curve"))?;

    // OmniBuild's public key, NUL-terminated as a C string, with as much
    // of its owner's name as fits.
    let mut info = vec![0; PEER_INFO_SIZE];
    info[0] = PUBLIC_KEY;
    let public_key = key.public_key().as_bytes();
    let len = public_key.len().min(PEER_INFO_SIZE - 2);
    info[1..=len].copy_from_slice(&public_key[..len]);
    let info = cipher
        .encrypt(&nonce(), info.as_slice())
        .expect("AES-GCM encrypts a peer info");
    let theirs = exchange(&mut tls, PEER_INFO, &info)?;
    let info = cipher.decrypt(&nonce(), theirs.as_slice()).map_err(|_| {
        anyhow!(
            "the code isn't the one the device shows. It shows a new one each time it starts pairing"
        )
    })?;
    match info.split_first() {
        Some((&GUID, guid)) if info.len() == PEER_INFO_SIZE => {
            let len = guid
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(guid.len());
            Ok(String::from_utf8_lossy(&guid[..len]).into_owned())
        }
        _ => bail!("the device didn't say who it is"),
    }
}

/// Sends a packet of `kind` with `payload`, and returns the payload of the
/// device's answer, which is of the same kind. A packet is its version,
/// its kind and its payload's length, 4 bytes big-endian, then its
/// payload.
fn exchange(tls: &mut Tls<'_>, kind: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let len = u32::try_from(payload.len()).expect("payloads are at most 16 KiB");
    let packet = [&[VERSION, kind], &len.to_be_bytes()[..], payload].concat();
    let mut header = [0; 6];
    tls.write_all(&packet)
        .and_then(|()| tls.flush())
        .context("couldn't send to the device")
        .and_then(|()| read(tls, &mut header))?;
    let len = u32::from_be_bytes(header[2..].try_into().expect("4 bytes")) as usize;
    if header[..2] != [VERSION, kind] || !(1..=MAX_PAYLOAD).contains(&len) {
        bail!("the device doesn't pair as adb does");
    }
    let mut payload = vec![0; len];
    read(tls, &mut payload)?;
    Ok(payload)
}

/// The cipher both parties encrypt their peer info with: AES-128-GCM, with
/// a key HKDF derives from SPAKE2's.
fn cipher(key: &[u8]) -> Aes128Gcm {
    let mut aes_key = [0; 16];
    Hkdf::<Sha256>::new(None, key)
        .expand(KEY_INFO, &mut aes_key)
        .expect("HKDF derives 16 bytes");
    Aes128Gcm::new(&aes_key.into())
}

/// The nonce of a party's first message, and only one here: adb counts
/// each party's messages from 0, little-endian, in the nonce's first 8
/// bytes.
fn nonce() -> Nonce<aes_gcm::aead::consts::U12> {
    Nonce::default()
}

#[cfg(test)]
pub mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::LazyLock;
    use std::thread::JoinHandle;

    use aes_gcm::aead::Aead;

    use super::{
        EXPORTER_LABEL, GUID, PEER_INFO, PEER_INFO_SIZE, PUBLIC_KEY, SPAKE2_MESSAGE, VERSION,
        cipher, nonce, pair,
    };
    use crate::adb::key::AdbKey;
    use crate::adb::spake2::{Role, Spake2};
    use crate::adb::tls::tests::{DeviceTls, accept};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/adb");
    static KEY: LazyLock<AdbKey> = LazyLock::new(|| {
        AdbKey::at(&Path::new(FIXTURES).join("adbkey"), "omnibuild@test")
            .unwrap()
            .0
    });

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// How a made-up device strays from pairing as Android does.
    #[derive(Clone, Copy, PartialEq)]
    pub enum Stray {
        Not,
        /// It hangs up before TLS.
        NoTls,
        /// Its SPAKE2 packet is of another version.
        Version,
        /// It sends its SPAKE2 packet's header, and hangs up.
        Truncated,
        /// Its SPAKE2 message isn't a point on the curve.
        NotAPoint,
        /// It reads the computer's SPAKE2 message, and hangs up.
        HangsUp,
        /// It reads the computer's peer info, and hangs up.
        Quits,
        /// Its peer info is a public key, as a computer's is.
        Info,
    }

    /// A packet of `kind` with `payload`.
    fn packet(kind: u8, payload: &[u8]) -> Vec<u8> {
        let len = u32::try_from(payload.len()).unwrap().to_be_bytes();
        [&[VERSION, kind], &len[..], payload].concat()
    }

    /// The payload of the computer's next packet.
    fn payload(tls: &mut DeviceTls) -> Vec<u8> {
        let mut header = [0; 6];
        tls.read_exact(&mut header).unwrap();
        let mut payload = vec![0; u32::from_be_bytes(header[2..].try_into().unwrap()) as usize];
        tls.read_exact(&mut payload).unwrap();
        payload
    }

    /// A device on `listener` that pairs with one computer while it shows
    /// `code`, as Android does, except as `stray` says, with the name
    /// adb-R5CT1234-a1B2c3. Returns the public key the computer sent,
    /// unless it hung up first or the code was wrong.
    pub fn device(
        listener: TcpListener,
        code: &'static str,
        stray: Stray,
    ) -> JoinHandle<Option<String>> {
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            if stray == Stray::NoTls {
                return None;
            }
            let mut tls = accept(stream, &KEY);
            while tls.conn.is_handshaking() {
                tls.conn.complete_io(&mut tls.sock).unwrap();
            }
            let exported = tls
                .conn
                .export_keying_material([0; 64], EXPORTER_LABEL, None)
                .unwrap();
            let spake2 = Spake2::new(Role::Server, &[code.as_bytes(), &exported].concat());
            // Before it answers, unlike Android, so that it never hangs up
            // with the computer's message unread, which would have the
            // system reset the connection.
            let theirs = payload(&mut tls);
            if stray == Stray::HangsUp {
                return None;
            }
            // y = 2 has no x on the curve.
            let mut message = spake2.message();
            if stray == Stray::NotAPoint {
                message = [0; 32];
                message[0] = 2;
            }
            let mut first = packet(SPAKE2_MESSAGE, &message);
            match stray {
                Stray::Version => first[0] = 2,
                Stray::Truncated => first.truncate(6),
                _ => {}
            }
            tls.write_all(&first).unwrap();
            if matches!(stray, Stray::Version | Stray::Truncated | Stray::NotAPoint) {
                return None;
            }
            if stray == Stray::Quits {
                payload(&mut tls);
                return None;
            }
            let cipher = cipher(&spake2.finish(&theirs).unwrap());
            let mut info = vec![0; PEER_INFO_SIZE];
            info[0] = if stray == Stray::Info {
                PUBLIC_KEY
            } else {
                GUID
            };
            info[1..20].copy_from_slice(b"adb-R5CT1234-a1B2c3");
            let info = cipher.encrypt(&nonce(), info.as_slice()).unwrap();
            tls.write_all(&packet(PEER_INFO, &info)).unwrap();
            let info = cipher
                .decrypt(&nonce(), payload(&mut tls).as_slice())
                .ok()?;
            assert_eq!((info.len(), info[0]), (PEER_INFO_SIZE, PUBLIC_KEY));
            let key = info[1..].split(|&byte| byte == 0).next().unwrap();
            Some(String::from_utf8(key.to_vec()).unwrap())
        })
    }

    /// Pairs with a device that shows `code` and strays from pairing as
    /// `stray` says, given `entered`, the code the user entered. Returns
    /// the device's GUID, or why pairing failed, and what the device got.
    fn pairing(
        code: &'static str,
        entered: &str,
        stray: Stray,
    ) -> (Result<String, String>, Option<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let device = device(listener, code, stray);

        let paired = pair(
            Box::new(TcpStream::connect(address).unwrap()),
            &KEY,
            entered,
        )
        .map_err(|error| format!("{error:#}"));
        (paired, device.join().unwrap())
    }

    #[test]
    fn pairs_with_the_code_the_device_shows() {
        let (paired, sent) = pairing("123456", "123456", Stray::Not);

        assert_eq!(paired.unwrap(), "adb-R5CT1234-a1B2c3");
        // OmniBuild's public key, as adb writes adbkey.pub.
        assert_eq!(sent.unwrap(), KEY.public_key());
    }

    #[test]
    fn says_when_the_code_is_wrong() {
        let (paired, sent) = pairing("123456", "654321", Stray::Not);

        assert_eq!(
            paired.unwrap_err(),
            "the code isn't the one the device shows. It shows a new one each time it starts pairing"
        );
        assert_eq!(sent, None);
    }

    #[test]
    fn says_when_the_device_doesnt_pair_as_adb_does() {
        let fails = |stray| pairing("123456", "123456", stray).0.unwrap_err();

        assert!(
            fails(Stray::NoTls)
                .starts_with("couldn't make an encrypted connection to the device: ")
        );
        assert_eq!(fails(Stray::Version), "the device doesn't pair as adb does");
        assert_eq!(fails(Stray::Truncated), "the device closed the connection");
        assert_eq!(
            fails(Stray::NotAPoint),
            "the device's SPAKE2 message isn't a point on the curve"
        );
        assert_eq!(fails(Stray::HangsUp), "the device closed the connection");
        assert_eq!(fails(Stray::Quits), "the device closed the connection");
        assert_eq!(fails(Stray::Info), "the device didn't say who it is");
    }

    #[test]
    fn understands_a_real_device() {
        // A pairing with an Android 14 emulator, with OmniBuild's random
        // bytes fixed: the password, which is the code the emulator showed
        // and the TLS connection's keying material, the message the
        // emulator took, its own, and its encrypted peer info.
        let password = hex(concat!(
            "383138343436",
            "06ea5efeeb3b37035a1265fb61c7fa65b47a0ea26efc6c0c9fdcb02eea4c3517",
            "f93c360f37617370f51c947d396fca37a7fb3bb0d21dd7a1c4b761af4a0ce0c9",
        ));
        let spake2 = Spake2::with_random(Role::Client, &password, &[7; 64]);
        assert_eq!(
            spake2.message().to_vec(),
            hex("987be16822d6097ef9699edfd30ceeed4da71be542bd12357cd7f26dde01529e")
        );
        let key = spake2
            .finish(&hex(
                "63e4e5863cdc4d5f0eaf17c862a37ba264d7661e0c35850f05469400b26f8934",
            ))
            .unwrap();
        let sent = std::fs::read(Path::new(FIXTURES).join("peer-info")).unwrap();

        let info = cipher(&key).decrypt(&nonce(), sent.as_slice()).unwrap();

        assert_eq!(info.len(), PEER_INFO_SIZE);
        assert_eq!(info[0], GUID);
        assert!(info[1..].starts_with(b"adb-EMULATOR37X2X12X0-H1pHlz\0"));
    }
}
