//! What OmniBuild checks of a device's TLS certificate, over wireless
//! debugging to an Android device and in a lockdown session with an iPhone:
//! nothing that anyone vouches for, as adb and libimobiledevice check
//! nothing, since nobody vouches for a device's key. What proves a device is
//! the one whose user allowed this computer is pairing: an Android device's
//! key exchange works only with the code it shows, and an iPhone only takes
//! the certificate it was given when its user trusted this computer.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Takes a device's certificate, whoever issued it, but still checks the
/// device's signature of the handshake, which proves it holds the key it
/// sent, with these algorithms: the provider's.
#[derive(Debug)]
pub struct AnyDevice(pub WebPkiSupportedAlgorithms);

impl ServerCertVerifier for AnyDevice {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, certificate, signature, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, certificate, signature, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::signature::{RandomizedSigner, SignatureEncoding};
    use rsa::{RsaPrivateKey, pss};
    use rustls::DigitallySignedStruct;
    use rustls::client::danger::ServerCertVerifier;
    use rustls::internal::msgs::codec::Codec;
    use rustls::pki_types::CertificateDer;
    use sha2::Sha256;

    use super::AnyDevice;
    use crate::adb::tls::tests::DEVICE_KEY;

    #[test]
    fn checks_a_devices_signature_of_the_handshake() {
        let device = AnyDevice(rustls_rustcrypto::provider().signature_verification_algorithms);
        let key = RsaPrivateKey::from_pkcs8_der(&DEVICE_KEY.private_key()).unwrap();
        let signature = pss::SigningKey::<Sha256>::new(key)
            .sign_with_rng(&mut rsa::rand_core::OsRng, b"handshake")
            .to_vec();
        // RSA-PSS with SHA-256's number, the signature's length, and the
        // signature, as TLS sends them.
        let mut encoded = vec![0x08, 0x04];
        encoded.extend(u16::try_from(signature.len()).unwrap().to_be_bytes());
        encoded.extend(signature);
        let signed = DigitallySignedStruct::read_bytes(&encoded).unwrap();
        let certificate = CertificateDer::from(DEVICE_KEY.certificate());

        for verify in [
            AnyDevice::verify_tls12_signature,
            AnyDevice::verify_tls13_signature,
        ] {
            assert!(verify(&device, b"handshake", &certificate, &signed).is_ok());
            assert!(verify(&device, b"forged", &certificate, &signed).is_err());
        }
    }
}
