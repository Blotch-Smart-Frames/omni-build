//! Provisioning profiles: Apple's permission for apps with a given App ID,
//! signed with given certificates, to run on devices (or be uploaded to the
//! App Store) with given entitlements. A device installs an app only with a
//! profile in its bundle (`embedded.mobileprovision`) that allows its
//! signature and entitlements.
//!
//! A profile is a property list in CMS signed data, signed by Apple.
//! OmniBuild reads it without checking Apple's signature: the device does.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use cms::content_info::ContentInfo;
use cms::signed_data::SignedData;
use plist::{Dictionary, Value};
use x509_cert::der::Decode;
use x509_cert::der::asn1::OctetString;

use super::entitlements::{Entitlement, Entitlements};

/// What makes an app an App Clip, which only the profile of an App ID
/// registered as an App Clip's allows.
const ON_DEMAND_INSTALL: &str = "com.apple.developer.on-demand-install-capable";
/// The App ID of the app an App Clip belongs to, as its signature names it.
const PARENT_APPLICATIONS: &str = "com.apple.developer.parent-application-identifiers";

pub struct Profile {
    /// The profile as Apple made it, which goes in the app as it is.
    pub bytes: Vec<u8>,
    /// The App ID's prefix and bundle ID, which may end in a wildcard:
    /// `ABCDE12345.*` allows any bundle ID.
    application_identifier: String,
    entitlements: Dictionary,
    /// The certificates the app may be signed with, as DER.
    certificates: Vec<Vec<u8>>,
}

impl Profile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let profile = ContentInfo::from_der(bytes)
            .ok()
            .and_then(|content| content.content.decode_as::<SignedData>().ok())
            .and_then(|data| data.encap_content_info.econtent)
            .and_then(|content| content.decode_as::<OctetString>().ok())
            .and_then(|plist| Value::from_reader(Cursor::new(plist.as_bytes())).ok())
            .and_then(Value::into_dictionary)
            .context("it isn't a provisioning profile, a property list in CMS signed data")?;
        let entitlements = profile
            .get("Entitlements")
            .and_then(Value::as_dictionary)
            .cloned()
            .unwrap_or_default();
        let application_identifier = entitlements
            .get("application-identifier")
            .and_then(Value::as_string)
            .context("it has no App ID (application-identifier)")?
            .to_owned();
        let certificates = profile
            .get("DeveloperCertificates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_data)
            .map(<[u8]>::to_vec)
            .collect();
        Ok(Self {
            bytes: bytes.to_vec(),
            application_identifier,
            entitlements,
            certificates,
        })
    }

    /// The App ID, as `TEAM.dev.example.app` or `TEAM.dev.example.*`.
    pub fn app_id(&self) -> &str {
        &self.application_identifier
    }

    /// The App ID's prefix, the team's ID: `TEAM` in `TEAM.dev.example.app`.
    pub fn prefix(&self) -> &str {
        self.application_identifier
            .split_once('.')
            .map_or("", |(prefix, _)| prefix)
    }

    /// How closely the App ID fits the bundle `bundle_id`, if it includes
    /// it: an exact App ID most, and then a wildcard the longer what it
    /// matches before its `*`, so that `TEAM.dev.example.*` fits closer
    /// than `TEAM.*`. Xcode picks the profile that fits a bundle closest.
    pub fn fit(&self, bundle_id: &str) -> Option<usize> {
        let (_, allowed) = self.application_identifier.split_once('.')?;
        match allowed.strip_suffix('*') {
            Some(start) => bundle_id.starts_with(start).then_some(start.len()),
            None => (allowed == bundle_id).then_some(usize::MAX),
        }
    }

    /// The entitlements to sign the bundle `bundle_id` with, whose ID the
    /// profile includes, signed with `certificate` (DER), if the profile
    /// allows it.
    ///
    /// Only the entitlements every app has: its App ID and team, and
    /// whether it may be debugged (a development profile) or report to
    /// `TestFlight` (a distribution profile). A profile also lists every
    /// capability its App ID may use, such as push notifications, but an
    /// app claims one only if it uses it.
    pub fn entitlements(&self, bundle_id: &str, certificate: &[u8]) -> Result<Entitlements> {
        let prefix = self.prefix();
        if !self
            .certificates
            .iter()
            .any(|allowed| allowed == certificate)
        {
            bail!("it doesn't allow the signing key's certificate (DeveloperCertificates)");
        }
        let mut entitlements = Entitlements::default();
        entitlements.insert(
            "application-identifier",
            Entitlement::String(format!("{prefix}.{bundle_id}")),
        );
        for (key, value) in &self.entitlements {
            match (key.as_str(), value) {
                ("com.apple.developer.team-identifier", Value::String(team)) => {
                    entitlements.insert(key, Entitlement::String(team.clone()));
                }
                ("get-task-allow" | "beta-reports-active", Value::Boolean(allowed)) => {
                    entitlements.insert(key, Entitlement::Boolean(*allowed));
                }
                _ => {}
            }
        }
        Ok(entitlements)
    }

    /// The entitlements to sign the App Clip `bundle_id`, of the app
    /// `app_id`, with: an app's, and those that make it an App Clip of that
    /// app, as Xcode signs one. iOS installs an App Clip only with them,
    /// and only with a profile that allows them: the profile of an App ID
    /// registered as an App Clip's, which this has to be.
    pub fn app_clip_entitlements(
        &self,
        bundle_id: &str,
        app_id: &str,
        certificate: &[u8],
    ) -> Result<Entitlements> {
        let mut entitlements = self.entitlements(bundle_id, certificate)?;
        if self
            .entitlements
            .get(ON_DEMAND_INSTALL)
            .and_then(Value::as_boolean)
            != Some(true)
        {
            bail!(
                "it isn't an App Clip's: it doesn't allow {ON_DEMAND_INSTALL}, as the profile of an App ID registered as an App Clip's does"
            );
        }
        entitlements.insert(ON_DEMAND_INSTALL, Entitlement::Boolean(true));
        entitlements.insert(
            PARENT_APPLICATIONS,
            Entitlement::Strings(vec![format!("{}.{app_id}", self.prefix())]),
        );
        Ok(entitlements)
    }
}

#[cfg(test)]
pub mod tests {
    use cms::content_info::{CmsVersion, ContentInfo};
    use cms::signed_data::{EncapsulatedContentInfo, SignedData, SignerInfos};
    use const_oid::db::rfc5911::{ID_DATA, ID_SIGNED_DATA};
    use x509_cert::der::asn1::{OctetString, SetOfVec};
    use x509_cert::der::{Any, Encode};

    use super::Profile;
    use crate::ios::code_signature::tests::identity;
    use crate::ios::entitlements::Entitlement;
    use crate::ios::entitlements::tests::shell_entitlements;

    /// The test profile: for the test identity, any bundle ID, and
    /// debugging, made as tests/fixtures/README.md says.
    pub const PROFILE: &[u8] = include_bytes!("../../tests/fixtures/profile.mobileprovision");
    /// The test App Clip profile: for the test identity, the shell's App
    /// Clip, `dev.omnibuild.shell.Clip`, of the shell, and debugging.
    pub const CLIP_PROFILE: &[u8] = include_bytes!("../../tests/fixtures/clip.mobileprovision");

    /// A profile of this property list, unsigned.
    fn profile_of(plist: &str) -> Vec<u8> {
        let signed_data = SignedData {
            version: CmsVersion::V1,
            digest_algorithms: SetOfVec::new(),
            encap_content_info: EncapsulatedContentInfo {
                econtent_type: ID_DATA,
                econtent: Some(
                    Any::encode_from(&OctetString::new(plist.as_bytes()).unwrap()).unwrap(),
                ),
            },
            certificates: None,
            crls: None,
            signer_infos: SignerInfos(SetOfVec::new()),
        };
        ContentInfo {
            content_type: ID_SIGNED_DATA,
            content: Any::encode_from(&signed_data).unwrap(),
        }
        .to_der()
        .unwrap()
    }

    /// A profile for this App ID, allowing the test identity's certificate,
    /// with a distribution profile's entitlements.
    pub fn profile_for(app_id: &str) -> Profile {
        let certificate = data(identity().certificate());
        Profile::parse(&profile_of(&format!(
            "<plist><dict>
                <key>DeveloperCertificates</key><array><data>{certificate}</data></array>
                <key>Entitlements</key><dict>
                    <key>application-identifier</key><string>{app_id}</string>
                    <key>beta-reports-active</key><true/>
                    <key>aps-environment</key><string>production</string>
                    <key>get-task-allow</key><string>not a boolean</string>
                </dict>
            </dict></plist>"
        )))
        .unwrap()
    }

    /// An App Clip's profile for this App ID, of the app `parent`, as Apple
    /// makes one, allowing the test identity's certificate, unsigned.
    pub fn app_clip_profile(app_id: &str, parent: &str) -> Vec<u8> {
        let certificate = data(identity().certificate());
        profile_of(&format!(
            "<plist><dict>
                <key>DeveloperCertificates</key><array><data>{certificate}</data></array>
                <key>Entitlements</key><dict>
                    <key>application-identifier</key><string>{app_id}</string>
                    <key>com.apple.developer.on-demand-install-capable</key><true/>
                    <key>com.apple.developer.parent-application-identifiers</key>
                    <array><string>{parent}</string></array>
                </dict>
            </dict></plist>"
        ))
    }

    fn data(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn gives_an_app_its_entitlements_from_the_profile() {
        let profile = Profile::parse(PROFILE).unwrap();

        let entitlements = profile
            .entitlements("dev.omnibuild.shell", identity().certificate())
            .unwrap();

        assert_eq!(profile.bytes, PROFILE);
        // Without its keychain access groups, which the app doesn't use.
        assert_eq!(entitlements, shell_entitlements());
    }

    #[test]
    fn takes_an_app_id_or_a_prefix_of_one() {
        let certificate = identity().certificate().to_vec();

        let exact = profile_for("TEAM.dev.example.app")
            .entitlements("dev.example.app", &certificate)
            .unwrap();
        let prefix = profile_for("TEAM.dev.example.*")
            .entitlements("dev.example.app", &certificate)
            .unwrap();

        assert_eq!(exact, prefix);
        assert_eq!(
            exact.get("application-identifier"),
            Some(&Entitlement::String("TEAM.dev.example.app".to_owned()))
        );
        assert_eq!(
            exact.get("beta-reports-active"),
            Some(&Entitlement::Boolean(true))
        );
        assert_eq!(exact.get("aps-environment"), None);
        assert_eq!(exact.get("get-task-allow"), None);
    }

    #[test]
    fn fits_a_bundle_closer_the_more_its_app_id_says() {
        let fit = |app_id: &str, bundle_id: &str| profile_for(app_id).fit(bundle_id);

        assert_eq!(
            fit("TEAM.dev.example.app", "dev.example.app"),
            Some(usize::MAX)
        );
        assert_eq!(fit("TEAM.dev.example.*", "dev.example.app"), Some(12));
        assert_eq!(fit("TEAM.*", "dev.example.app"), Some(0));
        assert_eq!(fit("TEAM.dev.example.app", "dev.example.app.Share"), None);
        assert_eq!(fit("TEAM.dev.other.*", "dev.example.app"), None);
        assert_eq!(fit("no prefix", "no prefix"), None);
        assert_eq!(profile_for("TEAM.*").app_id(), "TEAM.*");
    }

    #[test]
    fn gives_an_app_clip_the_entitlements_that_make_it_one() {
        let profile = Profile::parse(CLIP_PROFILE).unwrap();

        let entitlements = profile
            .app_clip_entitlements(
                "dev.omnibuild.shell.Clip",
                "dev.omnibuild.shell",
                identity().certificate(),
            )
            .unwrap();

        // An app's, and that it's an App Clip, and whose.
        let mut expected = profile
            .entitlements("dev.omnibuild.shell.Clip", identity().certificate())
            .unwrap();
        expected.insert(
            "com.apple.developer.on-demand-install-capable",
            Entitlement::Boolean(true),
        );
        expected.insert(
            "com.apple.developer.parent-application-identifiers",
            Entitlement::Strings(vec!["ABCDE12345.dev.omnibuild.shell".to_owned()]),
        );
        assert_eq!(entitlements, expected);
        assert_eq!(profile.prefix(), "ABCDE12345");
        assert_eq!(profile_for("no prefix").prefix(), "");
    }

    #[test]
    fn refuses_an_app_clip_a_profile_that_isnt_an_app_clips() {
        let certificate = identity().certificate().to_vec();
        let not_a_clips = Profile::parse(PROFILE).unwrap().app_clip_entitlements(
            "dev.omnibuild.shell.Clip",
            "dev.omnibuild.shell",
            &certificate,
        );
        let stranger = Profile::parse(CLIP_PROFILE).unwrap().app_clip_entitlements(
            "dev.omnibuild.shell.Clip",
            "dev.omnibuild.shell",
            b"another certificate",
        );

        assert_eq!(
            not_a_clips.unwrap_err().to_string(),
            "it isn't an App Clip's: it doesn't allow com.apple.developer.on-demand-install-capable, as the profile of an App ID registered as an App Clip's does"
        );
        assert_eq!(
            stranger.unwrap_err().to_string(),
            "it doesn't allow the signing key's certificate (DeveloperCertificates)"
        );
    }

    #[test]
    fn reports_a_profile_that_doesnt_allow_the_certificate() {
        let error = profile_for("TEAM.*")
            .entitlements("dev.example.app", b"another certificate")
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "it doesn't allow the signing key's certificate (DeveloperCertificates)"
        );
    }

    #[test]
    fn reports_what_isnt_a_profile() {
        let cases = [
            (
                b"not a profile".to_vec(),
                "it isn't a provisioning profile, a property list in CMS signed data",
            ),
            (
                profile_of("not a plist"),
                "it isn't a provisioning profile, a property list in CMS signed data",
            ),
            (
                profile_of("<plist><dict/></plist>"),
                "it has no App ID (application-identifier)",
            ),
        ];

        for (bytes, message) in cases {
            assert_eq!(Profile::parse(&bytes).err().unwrap().to_string(), message);
        }
    }
}
