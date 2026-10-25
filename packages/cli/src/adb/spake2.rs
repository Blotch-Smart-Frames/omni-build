//! SPAKE2 as BoringSSL has it (`crypto/curve25519/spake25519.cc`), which
//! Android pairs a computer with for wireless debugging: the two parties,
//! who share a password, the code the device shows, each send a point on
//! Ed25519's curve masked with it, and derive the same key from the two
//! only if their passwords match. Neither an eavesdropper nor a party
//! guessing learns anything to test guesses against offline.

use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use rsa::rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256, Sha512};

/// Which end of the pairing a party is: the client, which BoringSSL calls
/// Alice, is the computer, and the server, Bob, the device.
#[derive(Clone, Copy)]
pub enum Role {
    Client,
    Server,
}

impl Role {
    /// The party's name, which the key depends on: adb's
    /// (`pairing_auth.cpp`), with the NUL that ends a C string, which adb
    /// counts in.
    fn name(self) -> &'static [u8] {
        match self {
            Role::Client => b"adb pair client\0",
            Role::Server => b"adb pair server\0",
        }
    }

    /// The point the party masks its own with, M for the client and N for
    /// the server. BoringSSL's generator hashes a phrase with SHA-256, and
    /// hashes the hash until it decodes as a point, so that nobody knows
    /// the point's discrete log; both phrases' first hashes do.
    fn mask(self) -> EdwardsPoint {
        let seed: &[u8] = match self {
            Role::Client => b"edwards25519 point generation seed (M)",
            Role::Server => b"edwards25519 point generation seed (N)",
        };
        let point = CompressedEdwardsY(Sha256::digest(seed).into())
            .decompress()
            .expect("the seeds' hashes are points");
        // Only its part in the curve's prime-order subgroup, as BoringSSL
        // masks with since it fixed a slip that let the rest give away 3
        // bits of the password's hash. 8 times the point is 8 times that
        // part, so dividing by 8 modulo the subgroup's order leaves it.
        point.mul_by_cofactor() * Scalar::from(8u8).invert()
    }
}

/// One party's side of the exchange.
pub struct Spake2 {
    role: Role,
    /// The party's secret. Its point is 8 times this times the curve's base
    /// point, and it multiplies the other party's by 8 times this too, so
    /// both products end up in the prime-order subgroup.
    secret: Scalar,
    password_hash: [u8; 64],
    /// The password's hash as a scalar, which the masks are multiplied by.
    password: Scalar,
    /// The masked point it sends the other party.
    message: [u8; 32],
}

impl Spake2 {
    /// Starts the exchange for the party in `role`, with a new secret.
    pub fn new(role: Role, password: &[u8]) -> Self {
        let mut random = [0; 64];
        OsRng.fill_bytes(&mut random);
        Self::with_random(role, password, &random)
    }

    /// Starts the exchange with the 64 random bytes that BoringSSL reduces
    /// to the secret.
    pub(super) fn with_random(role: Role, password: &[u8], random: &[u8; 64]) -> Self {
        let secret = Scalar::from_bytes_mod_order_wide(random);
        let password_hash: [u8; 64] = Sha512::digest(password).into();
        let password = Scalar::from_bytes_mod_order_wide(&password_hash);
        let point = EdwardsPoint::mul_base(&(secret * Scalar::from(8u8))) + role.mask() * password;
        Self {
            role,
            secret,
            password_hash,
            password,
            message: point.compress().to_bytes(),
        }
    }

    /// What the party sends the other.
    pub fn message(&self) -> [u8; 32] {
        self.message
    }

    /// The key, given the other party's message: the same as the other
    /// party's only if their passwords were the same. Nothing when the
    /// message isn't a point on the curve.
    pub fn finish(self, theirs: &[u8]) -> Option<[u8; 64]> {
        let theirs: [u8; 32] = theirs.try_into().ok()?;
        let (other, client, server) = match self.role {
            Role::Client => (Role::Server, self.message, theirs),
            Role::Server => (Role::Client, theirs, self.message),
        };
        let unmasked = CompressedEdwardsY(theirs).decompress()? - other.mask() * self.password;
        let shared = (unmasked.mul_by_cofactor() * self.secret).compress();
        // The client's name and message first, whichever party this is,
        // each part after its length as 8 bytes, little-endian.
        let mut hash = Sha512::new();
        for part in [
            Role::Client.name(),
            Role::Server.name(),
            &client,
            &server,
            shared.as_bytes(),
            &self.password_hash,
        ] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part);
        }
        Some(hash.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    use curve25519_dalek::traits::IsIdentity;

    use super::{Role, Spake2};

    #[test]
    fn agrees_on_a_key_when_the_passwords_match() {
        let client = Spake2::new(Role::Client, b"123456");
        let server = Spake2::new(Role::Server, b"123456");
        let (to_server, to_client) = (client.message(), server.message());

        let key = client.finish(&to_client).unwrap();

        assert_eq!(server.finish(&to_server).unwrap(), key);
        // A secret of its own each time.
        assert_ne!(Spake2::new(Role::Client, b"123456").message(), to_server);
    }

    #[test]
    fn disagrees_when_they_dont() {
        let client = Spake2::new(Role::Client, b"123456");
        let server = Spake2::new(Role::Server, b"654321");
        let (to_server, to_client) = (client.message(), server.message());

        assert_ne!(
            client.finish(&to_client).unwrap(),
            server.finish(&to_server).unwrap()
        );
    }

    #[test]
    fn takes_only_a_point_on_the_curve() {
        let client = || Spake2::new(Role::Client, b"123456");
        // y = 2 has no x on the curve.
        let mut off_the_curve = [0; 32];
        off_the_curve[0] = 2;

        assert_eq!(client().finish(&off_the_curve), None);
        assert_eq!(client().finish(&[0; 31]), None);
    }

    #[test]
    fn masks_with_the_points_android_does() {
        // As spake25519.cc gives them, encoded.
        let m = "5ada7e4bf6ddd9adb6626d32131c6b5c51a1e347a3478f53cfcf441b88eed12e";
        let n = "10e3df0ae37d8e7a99b5fe74b44672103dbddcbd06af680d71329a11693bc778";
        let point = |hex: &str| {
            let bytes: Vec<u8> = (0..32)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            CompressedEdwardsY::from_slice(&bytes)
                .unwrap()
                .decompress()
                .unwrap()
        };

        // The parts of the points in the prime-order subgroup: the same
        // when multiplied by 8, and of the subgroup's order.
        for (role, point) in [(Role::Client, point(m)), (Role::Server, point(n))] {
            let mask = role.mask();
            assert_eq!(mask.mul_by_cofactor(), point.mul_by_cofactor());
            assert!(mask.is_torsion_free());
            assert!(!mask.is_identity());
        }
    }
}
