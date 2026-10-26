//! Provisioning an app through App Store Connect's API, with the user's API
//! key, as Xcode's automatic signing does with an Apple ID: a certificate
//! for the user's key, an App ID for the app and each bundle nested in it,
//! the capabilities the entitlements they claim need, and a provisioning
//! profile for each. It reuses what the account has, and creates only
//! what's missing: an account has only a few certificates of each kind,
//! and profiles pile up.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::json;
use x509_cert::Certificate;
use x509_cert::der::Decode;

use super::Signed;
use super::entitlements::Entitlements;
use super::profile::Profile;
use crate::app_store_connect::{Client, Resource};
use crate::key::{self, SigningKey, display_name};
use crate::report::{Event, KeyKind};

const CERTIFICATES: &str = "/v1/certificates";
const BUNDLE_IDS: &str = "/v1/bundleIds";
/// Every kind of certificate that signs iOS apps: Apple's, which Xcode 11
/// and later make, and the iOS-only kinds before them.
const SIGNING_CERTIFICATES: &str = "DEVELOPMENT,IOS_DEVELOPMENT,DISTRIBUTION,IOS_DISTRIBUTION";
/// The request a key's certificate is issued for names the key so. Apple
/// issues the certificate in the team's name whatever it is.
const REQUEST_NAME: &str = "App signing key";

/// The capabilities an App ID has to have for an app to claim an
/// entitlement, by entitlement: those App Store Connect's API turns on
/// without settings, with the names developer.apple.com gives them. Others,
/// such as iCloud, take settings, and some of these, such as App Groups,
/// also take identifiers that only developer.apple.com assigns to an App
/// ID, which the profile then allows.
const CAPABILITIES: [(&str, &str, &str); 19] = [
    (
        "aps-environment",
        "PUSH_NOTIFICATIONS",
        "Push Notifications",
    ),
    (
        "com.apple.developer.associated-domains",
        "ASSOCIATED_DOMAINS",
        "Associated Domains",
    ),
    (
        "com.apple.security.application-groups",
        "APP_GROUPS",
        "App Groups",
    ),
    ("com.apple.developer.healthkit", "HEALTHKIT", "HealthKit"),
    (
        "com.apple.developer.healthkit.access",
        "HEALTHKIT",
        "HealthKit",
    ),
    ("com.apple.developer.homekit", "HOMEKIT", "HomeKit"),
    (
        "com.apple.developer.networking.wifi-info",
        "ACCESS_WIFI_INFORMATION",
        "Access Wi-Fi Information",
    ),
    (
        "com.apple.developer.nfc.readersession.formats",
        "NFC_TAG_READING",
        "NFC Tag Reading",
    ),
    ("com.apple.developer.siri", "SIRIKIT", "SiriKit"),
    (
        "com.apple.developer.game-center",
        "GAME_CENTER",
        "Game Center",
    ),
    (
        "com.apple.developer.networking.HotspotConfiguration",
        "HOT_SPOT",
        "Hotspot",
    ),
    (
        "com.apple.developer.networking.multipath",
        "MULTIPATH",
        "Multipath",
    ),
    (
        "com.apple.developer.networking.vpn.api",
        "PERSONAL_VPN",
        "Personal VPN",
    ),
    (
        "com.apple.developer.networking.networkextension",
        "NETWORK_EXTENSIONS",
        "Network Extensions",
    ),
    (
        "com.apple.developer.ClassKit-environment",
        "CLASSKIT",
        "ClassKit",
    ),
    (
        "com.apple.developer.authentication-services.autofill-credential-provider",
        "AUTOFILL_CREDENTIAL_PROVIDER",
        "AutoFill Credential Provider",
    ),
    (
        "com.apple.developer.in-app-payments",
        "APPLE_PAY",
        "Apple Pay Payment Processing",
    ),
    (
        "com.apple.developer.pass-type-identifiers",
        "WALLET",
        "Wallet",
    ),
    (
        "com.apple.external-accessory.wireless-configuration",
        "WIRELESS_ACCESSORY_CONFIGURATION",
        "Wireless Accessory Configuration",
    ),
];

/// What the app is provisioned for, which decides the kind of profile and
/// certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Distribution {
    /// Uploading to App Store Connect, for `TestFlight` and the App Store.
    AppStore,
    /// Installing on the account's devices, outside the App Store.
    AdHoc,
    /// Running on the account's devices, where it can be debugged.
    Development,
}

impl Distribution {
    /// Its name, as a profile's kind.
    pub fn name(self) -> &'static str {
        match self {
            Self::AppStore => "App Store",
            Self::AdHoc => "Ad Hoc",
            Self::Development => "Development",
        }
    }

    /// The kind of profile App Store Connect makes for it.
    fn profile_type(self) -> &'static str {
        match self {
            Self::AppStore => "IOS_APP_STORE",
            Self::AdHoc => "IOS_APP_ADHOC",
            Self::Development => "IOS_APP_DEVELOPMENT",
        }
    }

    /// The kinds of certificate its profiles allow: first the one
    /// OmniBuild creates, Apple's, then the iOS-only kind before it.
    fn certificate_types(self) -> [&'static str; 2] {
        match self {
            Self::AppStore | Self::AdHoc => ["DISTRIBUTION", "IOS_DISTRIBUTION"],
            Self::Development => ["DEVELOPMENT", "IOS_DEVELOPMENT"],
        }
    }

    /// Whether its profiles list the devices the app installs on.
    fn lists_devices(self) -> bool {
        self != Self::AppStore
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CertificateAttributes {
    certificate_type: String,
    /// The certificate, DER in base64.
    certificate_content: String,
}

#[derive(Deserialize)]
struct BundleIdAttributes {
    identifier: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityAttributes {
    capability_type: String,
}

#[derive(Deserialize)]
pub struct DeviceAttributes {
    udid: String,
}

#[derive(Deserialize)]
struct ProfileAttributes {
    #[serde(rename = "profileType")]
    kind: String,
    #[serde(rename = "profileState")]
    state: String,
    /// The profile, in base64.
    #[serde(rename = "profileContent")]
    content: String,
}

/// What an app is provisioned with: the key to sign it with, and a profile
/// for each of its bundles, by the bundle's ID.
pub struct Provisioned {
    pub key: SigningKey,
    pub profiles: Vec<(String, Vec<u8>)>,
}

/// Provisions apps for `distribution` through App Store Connect, with
/// `client`, and reports what it creates to `report`.
pub struct Provisioner<'a> {
    pub client: &'a Client,
    pub distribution: Distribution,
    pub report: &'a dyn Fn(Event),
}

/// What every profile allows: the key's certificate, by its ID in App Store
/// Connect, and for a development or ad hoc profile, the account's devices.
struct Identity<'a> {
    key: &'a SigningKey,
    certificate: &'a str,
    devices: &'a [Resource<DeviceAttributes>],
}

impl Identity<'_> {
    /// Whether `profile`, which hasn't expired, allows the key's
    /// certificate and lists every device.
    fn fits(&self, profile: &Profile) -> bool {
        profile.allows(self.key.certificate())
            && !profile.has_expired(SystemTime::now())
            && self
                .devices
                .iter()
                .all(|device| profile.lists(&device.attributes.udid))
    }
}

impl Provisioner<'_> {
    /// Provisions the app whose bundles are `bundles`, the app's first, to
    /// be signed with the key in the keystore at `keystore`, with
    /// `password`, which it creates there if there's none.
    pub fn provision(
        &self,
        bundles: &[Signed],
        keystore: &Path,
        password: &str,
    ) -> Result<Provisioned> {
        let (key, certificate) = self.certificate(keystore, password)?;
        let devices = if self.distribution.lists_devices() {
            self.devices()?
        } else {
            Vec::new()
        };
        let identity = Identity {
            key: &key,
            certificate: &certificate,
            devices: &devices,
        };
        let app = &bundles[0].id;
        let mut profiles = Vec::new();
        for bundle in bundles {
            let app_id = self.app_id(bundle, app)?;
            let changed = self.capabilities(&app_id, bundle)?;
            let profile = self.profile(&identity, bundle, app, &app_id, changed)?;
            profiles.push((bundle.id.clone(), profile));
        }
        Ok(Provisioned { key, profiles })
    }

    /// The key in the keystore at `path`, which it creates if there's none,
    /// with a certificate of a kind the profiles allow, and the
    /// certificate's ID: the account's certificate for the key, or a new
    /// one, which the keystore then has too.
    fn certificate(&self, path: &Path, password: &str) -> Result<(SigningKey, String)> {
        let found = key::read_apple_key(path, password)?;
        // Before a key is created, so that an API key App Store Connect
        // refuses leaves nothing behind.
        let certificates = self
            .client
            .list::<CertificateAttributes>(
                CERTIFICATES,
                &[("filter[certificateType]", SIGNING_CERTIFICATES)],
            )
            .context("couldn't list the account's certificates")?;
        let (key, own) = if let Some(found) = found {
            found
        } else {
            let key = key::create_apple_key(path, REQUEST_NAME, password)?;
            (self.report)(Event::KeyCreated {
                key: KeyKind::Apple,
                path: path.display().to_string(),
            });
            (key, None)
        };
        let public_key = key::public_key(&key);
        let theirs: Vec<_> = certificates
            .into_iter()
            .filter_map(|resource| {
                let certificate = decode_certificate(&resource.attributes.certificate_content)?;
                key::is_certificate_of(&certificate, &public_key).then_some((resource, certificate))
            })
            .collect();
        let wanted = self.distribution.certificate_types();
        if let Some((resource, certificate)) = theirs
            .iter()
            .find(|(resource, _)| wanted.contains(&resource.attributes.certificate_type.as_str()))
        {
            // Saved again only if the keystore doesn't have it yet.
            let key = if own.as_ref() == Some(certificate) {
                SigningKey::new(key, certificate.clone(), Vec::new())
            } else {
                SigningKey::store(path, key, certificate.clone(), password)?
            };
            return Ok((key, resource.id.clone()));
        }
        // A key has one certificate in its keystore, which signs one kind
        // of build.
        if let Some(own) = own.filter(|own| theirs.iter().any(|(_, theirs)| theirs == own)) {
            bail!(
                "the certificate in {}, \"{}\", isn't one {} profiles allow: give them another keystore with --signing-key, which OmniBuild creates if there's none",
                path.display(),
                display_name(&own.tbs_certificate.subject),
                self.distribution.name()
            );
        }
        let subject = format!("CN={REQUEST_NAME}")
            .parse()
            .expect("the request's subject is a valid name");
        let created: Resource<CertificateAttributes> = self
            .client
            .create(
                CERTIFICATES,
                &json!({
                    "type": "certificates",
                    "attributes": {
                        "certificateType": wanted[0],
                        "csrContent": key::signing_request(&key, &subject),
                    },
                }),
            )
            .with_context(|| {
                format!(
                    "couldn't create a certificate for the key in {}",
                    path.display()
                )
            })?;
        let certificate = decode_certificate(&created.attributes.certificate_content)
            .context("the certificate App Store Connect created isn't one")?;
        let key = SigningKey::store(path, key, certificate, password)?;
        (self.report)(Event::CertificateCreated {
            path: path.display().to_string(),
            certificate: display_name(&key.x509().tbs_certificate.subject),
            expires: key.expires(),
        });
        Ok((key, created.id))
    }

    /// The account's iOS devices, which development and ad hoc profiles
    /// list.
    fn devices(&self) -> Result<Vec<Resource<DeviceAttributes>>> {
        let devices = self
            .client
            .list(
                "/v1/devices",
                &[("filter[platform]", "IOS"), ("filter[status]", "ENABLED")],
            )
            .context("couldn't list the account's devices")?;
        if devices.is_empty() {
            bail!(
                "the account has no devices for {} profiles to list: register each by its UDID at developer.apple.com, under Devices",
                self.distribution.name()
            );
        }
        Ok(devices)
    }

    /// The ID of the bundle's App ID, which it registers if the account has
    /// none. The App ID of an App Clip of the app `app` can't be registered
    /// through the API.
    fn app_id(&self, bundle: &Signed, app: &str) -> Result<String> {
        let found = self
            .client
            .list::<BundleIdAttributes>(BUNDLE_IDS, &[("filter[identifier]", &bundle.id)])
            .with_context(|| format!("couldn't look for the App ID {}", bundle.id))?
            .into_iter()
            // The filter takes IDs that start with it too.
            .find(|found| found.attributes.identifier == bundle.id);
        if let Some(found) = found {
            return Ok(found.id);
        }
        if bundle.app_clip {
            bail!(
                "{} is an App Clip's ID, which App Store Connect's API can't register: register it at developer.apple.com, under Identifiers, as an App Clip of {app}, then provision again",
                bundle.id
            );
        }
        // Its description, as developer.apple.com shows it, which takes
        // letters, digits and spaces.
        let name: String = format!("OmniBuild {}", bundle.id)
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    ' '
                }
            })
            .collect();
        let created: Resource<IgnoredAny> = self
            .client
            .create(
                BUNDLE_IDS,
                &json!({
                    "type": "bundleIds",
                    "attributes": { "identifier": bundle.id, "name": name, "platform": "IOS" },
                }),
            )
            .with_context(|| format!("couldn't register the App ID {}", bundle.id))?;
        (self.report)(Event::AppIdRegistered {
            bundle_id: bundle.id.clone(),
        });
        Ok(created.id)
    }

    /// Turns on the capabilities the entitlements the bundle claims need,
    /// for its App ID, `app_id`, where it has them off. Returns whether it
    /// turned any on.
    fn capabilities(&self, app_id: &str, bundle: &Signed) -> Result<bool> {
        let needed: BTreeSet<(&str, &str)> = bundle
            .claims
            .iter()
            .flat_map(|claims| claims.entitlements.keys())
            .filter_map(|key| {
                CAPABILITIES
                    .iter()
                    .find(|(entitlement, ..)| entitlement == key)
            })
            .map(|&(_, kind, name)| (kind, name))
            .collect();
        if needed.is_empty() {
            return Ok(false);
        }
        let on: Vec<String> = self
            .client
            .list::<CapabilityAttributes>(
                &format!("{BUNDLE_IDS}/{app_id}/bundleIdCapabilities"),
                &[],
            )
            .with_context(|| format!("couldn't list the capabilities of {}", bundle.id))?
            .into_iter()
            .map(|capability| capability.attributes.capability_type)
            .collect();
        let mut changed = false;
        for (kind, name) in needed {
            if on.iter().any(|on| on == kind) {
                continue;
            }
            self.client
                .create::<IgnoredAny>(
                    "/v1/bundleIdCapabilities",
                    &json!({
                        "type": "bundleIdCapabilities",
                        "attributes": { "capabilityType": kind },
                        "relationships": {
                            "bundleId": { "data": { "type": "bundleIds", "id": app_id } },
                        },
                    }),
                )
                .with_context(|| format!("couldn't turn on {name} for {}", bundle.id))?;
            (self.report)(Event::CapabilityEnabled {
                bundle_id: bundle.id.clone(),
                capability: name.to_owned(),
            });
            changed = true;
        }
        Ok(changed)
    }

    /// A profile for the bundle, of the app `app`, whose App ID is
    /// `app_id`, that allows what `identity` has and what the bundle claims:
    /// one the account has, unless its App ID's capabilities `changed`,
    /// which leaves the App ID's profiles behind, or a new one.
    fn profile(
        &self,
        identity: &Identity,
        bundle: &Signed,
        app: &str,
        app_id: &str,
        changed: bool,
    ) -> Result<Vec<u8>> {
        let short = || {
            format!(
                "{}'s App ID lacks what it claims: turn it on at developer.apple.com, under Identifiers, where the App ID's App Groups, iCloud containers and merchant IDs are chosen too, then provision again",
                bundle.id
            )
        };
        let profile_type = self.distribution.profile_type();
        if !changed {
            let profiles = self
                .client
                .list::<ProfileAttributes>(&format!("{BUNDLE_IDS}/{app_id}/profiles"), &[])
                .with_context(|| format!("couldn't list the profiles of {}", bundle.id))?;
            let mut refused = None;
            for (bytes, profile) in profiles
                .iter()
                .filter(|found| {
                    found.attributes.kind == profile_type && found.attributes.state == "ACTIVE"
                })
                .filter_map(|found| decode_profile(&found.attributes.content))
                .filter(|(_, profile)| identity.fits(profile))
            {
                match allows_claims(&profile, bundle, app, identity.key) {
                    Ok(()) => return Ok(bytes),
                    Err(error) => refused = Some(error),
                }
            }
            // A new profile would allow no more than the App ID has, which
            // makes an old one invalid when it changes.
            if let Some(error) = refused {
                return Err(error).with_context(short);
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Unique, as profiles' names are.
        let name = format!("OmniBuild {} {} {now}", self.distribution.name(), bundle.id);
        let mut relationships = json!({
            "bundleId": { "data": { "type": "bundleIds", "id": app_id } },
            "certificates": { "data": [{ "type": "certificates", "id": identity.certificate }] },
        });
        if self.distribution.lists_devices() {
            let devices: Vec<_> = identity
                .devices
                .iter()
                .map(|device| json!({ "type": "devices", "id": device.id }))
                .collect();
            relationships["devices"] = json!({ "data": devices });
        }
        let created: Resource<ProfileAttributes> = self
            .client
            .create(
                "/v1/profiles",
                &json!({
                    "type": "profiles",
                    "attributes": { "name": name, "profileType": profile_type },
                    "relationships": relationships,
                }),
            )
            .with_context(|| format!("couldn't create a provisioning profile for {}", bundle.id))?;
        (self.report)(Event::ProfileCreated {
            bundle_id: bundle.id.clone(),
            name,
        });
        let (bytes, profile) = decode_profile(&created.attributes.content).with_context(|| {
            format!(
                "the provisioning profile App Store Connect created for {} isn't one",
                bundle.id
            )
        })?;
        allows_claims(&profile, bundle, app, identity.key).with_context(short)?;
        Ok(bytes)
    }
}

/// Whether `profile` allows what the bundle, of the app `app`, claims, as
/// signing checks it: an App Clip's what makes it one too. The certificate
/// it allows is `Identity::fits`'s to check.
fn allows_claims(profile: &Profile, bundle: &Signed, app: &str, key: &SigningKey) -> Result<()> {
    if bundle.app_clip {
        profile.app_clip_entitlements(&bundle.id, app, key.certificate())?;
    }
    bundle.claims.as_ref().map_or(Ok(()), |claims| {
        profile.claim(
            &mut Entitlements::default(),
            &bundle.id,
            &claims.entitlements,
            &claims.source,
        )
    })
}

/// A certificate App Store Connect gives, in base64.
fn decode_certificate(content: &str) -> Option<Certificate> {
    let der = STANDARD.decode(content).ok()?;
    Certificate::from_der(&der).ok()
}

/// A profile App Store Connect gives, in base64, as it is and read.
fn decode_profile(content: &str) -> Option<(Vec<u8>, Profile)> {
    let bytes = STANDARD.decode(content).ok()?;
    let profile = Profile::parse(&bytes).ok()?;
    Some((bytes, profile))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use anyhow::Result;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::{Value, json};
    use x509_cert::der::Encode;

    use super::{CAPABILITIES, Distribution, Provisioned, Provisioner};
    use crate::app_store_connect::Client;
    use crate::app_store_connect::tests::api_key;
    use crate::config::Entitlement;
    use crate::http::tests::{Request, serve};
    use crate::ios::Signed;
    use crate::ios::profile::Profile;
    use crate::ios::profile::tests::profile_of;
    use crate::ios::signing::Claims;
    use crate::key::tests::issue_for;
    use crate::key::{self, SigningKey};
    use crate::report::{Event, KeyKind};

    const PASSWORD: &str = "a password";
    const TEAM: &str = "ABCDE12345";

    /// A made-up account, as the made-up App Store Connect keeps it: its
    /// resources, as the API gives them.
    #[derive(Default)]
    struct Account {
        certificates: Vec<Value>,
        bundle_ids: Vec<Value>,
        /// The App IDs registered as App Clips', each with its app's.
        app_clips: BTreeMap<String, String>,
        /// Each capability's kind, after the ID of the App ID that has it.
        capabilities: Vec<(String, String)>,
        devices: Vec<Value>,
        /// Each profile, after the ID of its App ID.
        profiles: Vec<(String, Value)>,
        /// The requests it refuses, by their method and the end of their
        /// path.
        refuses: Option<(&'static str, &'static str)>,
        /// Whether what it creates comes out garbled.
        garbles: bool,
        /// The last ID it gave.
        last: u32,
    }

    impl Account {
        fn new_id(&mut self, kind: &str) -> String {
            self.last += 1;
            format!("{kind}{}", self.last)
        }

        /// Its answer to `request`, as App Store Connect's.
        fn answer(&mut self, request: &Request) -> (u16, String) {
            if self.refuses.is_some_and(|(method, path)| {
                request.method == method && request.path.ends_with(path)
            }) {
                let errors = json!([{ "title": "Refused.", "detail": "The test says so." }]);
                return (409, json!({ "errors": errors }).to_string());
            }
            let attributes = request.body["data"]["attributes"].clone();
            let path: Vec<&str> = request.path.split('/').skip(2).collect();
            let data = match (request.method.as_str(), path.as_slice()) {
                ("GET", ["certificates"]) => json!(self.certificates),
                ("POST", ["certificates"]) => {
                    let kind = attributes["certificateType"].as_str().unwrap();
                    let name = if kind == "DISTRIBUTION" {
                        "Distribution"
                    } else {
                        "Development"
                    };
                    let id = self.new_id("C");
                    // A serial number of its own, as each certificate has.
                    let certificate = issue_for(
                        attributes["csrContent"].as_str().unwrap(),
                        &format!("CN=Apple {name}: OmniBuild Test ({TEAM})"),
                        self.last,
                    );
                    let content = self.garbled(&certificate.to_der().unwrap());
                    let resource = certificate_resource(&id, kind, &content);
                    self.certificates.push(resource.clone());
                    resource
                }
                ("GET", ["bundleIds"]) => {
                    // As App Store Connect filters them: IDs that start with
                    // the one asked for.
                    let wanted = &request.query["filter[identifier]"];
                    json!(
                        self.bundle_ids
                            .iter()
                            .filter(|found| identifier(found).starts_with(wanted.as_str()))
                            .collect::<Vec<_>>()
                    )
                }
                ("POST", ["bundleIds"]) => {
                    let id = self.new_id("B");
                    let resource = bundle_id_resource(&id, attributes["identifier"].as_str().unwrap());
                    self.bundle_ids.push(resource.clone());
                    resource
                }
                ("GET", ["bundleIds", app_id, "bundleIdCapabilities"]) => json!(
                    self.capabilities
                        .iter()
                        .filter(|(of, _)| of == app_id)
                        .map(|(_, kind)| json!({ "type": "bundleIdCapabilities", "id": kind, "attributes": { "capabilityType": kind } }))
                        .collect::<Vec<_>>()
                ),
                ("POST", ["bundleIdCapabilities"]) => {
                    let of = request.body["data"]["relationships"]["bundleId"]["data"]["id"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                    let kind = attributes["capabilityType"].as_str().unwrap().to_owned();
                    // As Apple does, the App ID's profiles no longer do.
                    for (_, profile) in self.profiles.iter_mut().filter(|(app_id, _)| *app_id == of) {
                        profile["attributes"]["profileState"] = json!("INVALID");
                    }
                    self.capabilities.push((of, kind.clone()));
                    json!({ "type": "bundleIdCapabilities", "id": kind, "attributes": { "capabilityType": kind } })
                }
                ("GET", ["devices"]) => json!(self.devices),
                ("GET", ["bundleIds", app_id, "profiles"]) => json!(
                    self.profiles
                        .iter()
                        .filter(|(of, _)| of == app_id)
                        .map(|(_, profile)| profile)
                        .collect::<Vec<_>>()
                ),
                // The rest: making a profile.
                _ => self.make_profile(&request.body["data"]),
            };
            let status = if request.method == "GET" { 200 } else { 201 };
            (status, json!({ "data": data }).to_string())
        }

        /// A profile it makes, as `data` asks: one that allows what the
        /// App ID has.
        fn make_profile(&mut self, data: &Value) -> Value {
            let relationships = &data["relationships"];
            let of = relationships["bundleId"]["data"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let found = self
                .bundle_ids
                .iter()
                .find(|found| found["id"] == of)
                .unwrap();
            let bundle_id = identifier(found).to_owned();
            let certificates: String = relationships["certificates"]["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|wanted| {
                    let found = self
                        .certificates
                        .iter()
                        .find(|found| found["id"] == wanted["id"])
                        .unwrap();
                    format!(
                        "<data>{}</data>",
                        found["attributes"]["certificateContent"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>()
                .concat();
            let devices: String = relationships["devices"]["data"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|wanted| {
                    let found = self
                        .devices
                        .iter()
                        .find(|found| found["id"] == wanted["id"])
                        .unwrap();
                    format!(
                        "<string>{}</string>",
                        found["attributes"]["udid"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>()
                .concat();
            // What each capability the App ID has allows, any value.
            let mut entitlements: Vec<String> = self
                .capabilities
                .iter()
                .filter(|(app_id, _)| *app_id == of)
                .flat_map(|(_, kind)| {
                    CAPABILITIES
                        .iter()
                        .filter(move |(_, capability, _)| capability == kind)
                })
                .map(|(entitlement, ..)| format!("<key>{entitlement}</key><string>*</string>"))
                .collect();
            if let Some(app) = self.app_clips.get(&bundle_id) {
                entitlements.push(format!(
                    "<key>com.apple.developer.on-demand-install-capable</key><true/>
                    <key>com.apple.developer.parent-application-identifiers</key>
                    <array><string>{TEAM}.{app}</string></array>"
                ));
            }
            let entitlements = entitlements.concat();
            let profile = profile_plist(
                &bundle_id,
                &certificates,
                &entitlements,
                &devices,
                "2030-01-01T00:00:00Z",
            );
            let content = self.garbled(&profile);
            let id = self.new_id("P");
            let kind = data["attributes"]["profileType"].as_str().unwrap();
            let resource = profile_resource(&id, kind, "ACTIVE", &content);
            self.profiles.push((of, resource.clone()));
            resource
        }

        /// `bytes` in base64, unless the account garbles what it makes.
        fn garbled(&self, bytes: &[u8]) -> String {
            if self.garbles {
                "garbled".to_owned()
            } else {
                STANDARD.encode(bytes)
            }
        }
    }

    fn identifier(bundle_id: &Value) -> &str {
        bundle_id["attributes"]["identifier"].as_str().unwrap()
    }

    fn certificate_resource(id: &str, kind: &str, content: &str) -> Value {
        json!({ "type": "certificates", "id": id, "attributes": { "certificateType": kind, "certificateContent": content } })
    }

    fn bundle_id_resource(id: &str, identifier: &str) -> Value {
        json!({ "type": "bundleIds", "id": id, "attributes": { "identifier": identifier } })
    }

    fn profile_resource(id: &str, kind: &str, state: &str, content: &str) -> Value {
        json!({ "type": "profiles", "id": id, "attributes": { "profileType": kind, "profileState": state, "profileContent": content } })
    }

    /// A profile for the App ID of `bundle_id`, allowing `certificates` and
    /// `entitlements` (a property list's elements), listing `devices`, and
    /// expiring at `expires`.
    fn profile_plist(
        bundle_id: &str,
        certificates: &str,
        entitlements: &str,
        devices: &str,
        expires: &str,
    ) -> Vec<u8> {
        profile_of(&format!(
            "<plist><dict>
                <key>DeveloperCertificates</key><array>{certificates}</array>
                <key>Entitlements</key><dict>
                    <key>application-identifier</key><string>{TEAM}.{bundle_id}</string>
                    <key>com.apple.developer.team-identifier</key><string>{TEAM}</string>
                    {entitlements}
                </dict>
                <key>ExpirationDate</key><date>{expires}</date>
                <key>ProvisionedDevices</key><array>{devices}</array>
            </dict></plist>"
        ))
    }

    /// What a test and the made-up App Store Connect share.
    type Shared<T> = Arc<Mutex<T>>;

    /// The made-up App Store Connect, with `account`, and a client of it,
    /// with the account and the requests it received.
    fn app_store_connect(account: Account) -> (Client, Shared<Account>, Shared<Vec<Request>>) {
        let account = Arc::new(Mutex::new(account));
        let shared = Arc::clone(&account);
        let (url, requests) = serve(move |request, _| shared.lock().unwrap().answer(request));
        (Client::new(api_key(), &url), account, requests)
    }

    /// An account with the App Clip's App ID, which only developer.apple.com
    /// registers, and two devices.
    fn new_account() -> Account {
        let mut account = Account::default();
        account
            .bundle_ids
            .push(bundle_id_resource("CLIP", "dev.example.app.Clip"));
        account.app_clips.insert(
            "dev.example.app.Clip".to_owned(),
            "dev.example.app".to_owned(),
        );
        for number in [1, 2] {
            account.devices.push(json!({ "type": "devices", "id": format!("D{number}"), "attributes": { "udid": format!("UDID{number}") } }));
        }
        account
    }

    /// What the app config claims for the app, in `ios.entitlements`.
    fn claims(claims: &[(&str, Entitlement)]) -> Claims {
        Claims {
            source: "ios.entitlements".to_owned(),
            entitlements: claims
                .iter()
                .map(|(key, claim)| ((*key).to_owned(), claim.clone()))
                .collect(),
        }
    }

    /// The app's bundles: the app, which claims `app_claims`, an extension,
    /// and an App Clip.
    fn bundles(app_claims: Option<Claims>) -> Vec<Signed> {
        vec![
            Signed {
                id: "dev.example.app".to_owned(),
                app_clip: false,
                claims: app_claims,
            },
            Signed {
                id: "dev.example.app.Share".to_owned(),
                app_clip: false,
                claims: None,
            },
            Signed {
                id: "dev.example.app.Clip".to_owned(),
                app_clip: true,
                claims: None,
            },
        ]
    }

    /// Push notifications, and the links of the app's website.
    fn push_and_links() -> Claims {
        claims(&[
            (
                "aps-environment",
                Entitlement::String("production".to_owned()),
            ),
            (
                "com.apple.developer.associated-domains",
                Entitlement::Strings(vec!["applinks:example.com".to_owned()]),
            ),
        ])
    }

    /// Provisions `bundles` for `distribution` through App Store Connect
    /// with `client`, with the keystore at `keystore`. Returns what it
    /// provisioned, or why it couldn't, and what it reported, with the time
    /// it named profiles by left out.
    fn provision(
        client: &Client,
        distribution: Distribution,
        bundles: &[Signed],
        keystore: &Path,
    ) -> (Result<Provisioned>, Vec<Event>) {
        provision_with(client, distribution, bundles, keystore, PASSWORD)
    }

    /// As `provision` does, with the keystore's password, `password`.
    fn provision_with(
        client: &Client,
        distribution: Distribution,
        bundles: &[Signed],
        keystore: &Path,
        password: &str,
    ) -> (Result<Provisioned>, Vec<Event>) {
        let events = RefCell::new(Vec::new());
        let report = |event| events.borrow_mut().push(event);
        let provisioned = Provisioner {
            client,
            distribution,
            report: &report,
        }
        .provision(bundles, keystore, password);
        let events = events
            .into_inner()
            .into_iter()
            .map(|event| match event {
                Event::ProfileCreated { bundle_id, name } => Event::ProfileCreated {
                    bundle_id,
                    name: name.rsplit_once(' ').unwrap().0.to_owned(),
                },
                event => event,
            })
            .collect();
        (provisioned, events)
    }

    fn profile_created(bundle_id: &str, kind: &str) -> Event {
        Event::ProfileCreated {
            bundle_id: bundle_id.to_owned(),
            name: format!("OmniBuild {kind} {bundle_id}"),
        }
    }

    fn app_id_registered(bundle_id: &str) -> Event {
        Event::AppIdRegistered {
            bundle_id: bundle_id.to_owned(),
        }
    }

    /// The requests that changed the account: those that created something.
    fn creations(requests: &Mutex<Vec<Request>>) -> Vec<Request> {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .cloned()
            .collect()
    }

    /// What provisioning the app on a new account reports, with the
    /// keystore at `keystore`.
    fn created_on_a_new_account(keystore: &Path) -> Vec<Event> {
        let path = keystore.display().to_string();
        vec![
            Event::KeyCreated {
                key: KeyKind::Apple,
                path: path.clone(),
            },
            Event::CertificateCreated {
                path,
                certificate: "Apple Distribution: OmniBuild Test (ABCDE12345)".to_owned(),
                expires: "2027-10-22".to_owned(),
            },
            app_id_registered("dev.example.app"),
            Event::CapabilityEnabled {
                bundle_id: "dev.example.app".to_owned(),
                capability: "Associated Domains".to_owned(),
            },
            Event::CapabilityEnabled {
                bundle_id: "dev.example.app".to_owned(),
                capability: "Push Notifications".to_owned(),
            },
            profile_created("dev.example.app", "App Store"),
            app_id_registered("dev.example.app.Share"),
            profile_created("dev.example.app.Share", "App Store"),
            profile_created("dev.example.app.Clip", "App Store"),
        ]
    }

    #[test]
    fn provisions_an_app_on_a_new_account() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("keys/apple.p12");
        let (client, _, requests) = app_store_connect(new_account());

        let (provisioned, events) = provision(
            &client,
            Distribution::AppStore,
            &bundles(Some(push_and_links())),
            &keystore,
        );

        let provisioned = provisioned.unwrap();
        assert_eq!(events, created_on_a_new_account(&keystore));
        // The keystore has the key, and the certificate Apple issued.
        let read = SigningKey::read(&keystore, PASSWORD).unwrap();
        assert_eq!(read.certificate(), provisioned.key.certificate());
        let ids: Vec<_> = provisioned
            .profiles
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "dev.example.app",
                "dev.example.app.Share",
                "dev.example.app.Clip"
            ]
        );
        for (id, bytes) in &provisioned.profiles {
            let profile = Profile::parse(bytes).unwrap();
            assert_eq!(profile.app_id(), format!("{TEAM}.{id}"));
            assert!(profile.allows(read.certificate()));
        }
        let created = creations(&requests);
        let paths: Vec<_> = created
            .iter()
            .map(|request| request.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "/v1/certificates",
                "/v1/bundleIds",
                "/v1/bundleIdCapabilities",
                "/v1/bundleIdCapabilities",
                "/v1/profiles",
                "/v1/bundleIds",
                "/v1/profiles",
                "/v1/profiles",
            ]
        );
        let certificate = &created[0].body["data"];
        assert_eq!(certificate["type"], "certificates");
        assert_eq!(certificate["attributes"]["certificateType"], "DISTRIBUTION");
        assert!(
            certificate["attributes"]["csrContent"]
                .as_str()
                .unwrap()
                .starts_with("-----BEGIN CERTIFICATE REQUEST-----\n")
        );
        assert_eq!(
            created[1].body["data"],
            json!({ "type": "bundleIds", "attributes": { "identifier": "dev.example.app", "name": "OmniBuild dev example app", "platform": "IOS" } })
        );
        assert_eq!(
            created[2].body["data"]["relationships"]["bundleId"]["data"],
            json!({ "type": "bundleIds", "id": "B2" })
        );
        let profile = &created[4].body["data"];
        assert_eq!(profile["attributes"]["profileType"], "IOS_APP_STORE");
        assert_eq!(
            profile["relationships"],
            json!({
                "bundleId": { "data": { "type": "bundleIds", "id": "B2" } },
                "certificates": { "data": [{ "type": "certificates", "id": "C1" }] },
            })
        );
    }

    #[test]
    fn reuses_what_the_account_has() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let (client, account, requests) = app_store_connect(new_account());
        let bundles = bundles(Some(push_and_links()));
        let (first, _) = provision(&client, Distribution::AppStore, &bundles, &keystore);
        let first = first.unwrap();
        // Before the app's profile, profiles that won't do: of another kind,
        // invalid, unreadable, expired, or for another certificate.
        {
            let mut account = account.lock().unwrap();
            let profile = |certificates: &str, expires| {
                STANDARD.encode(profile_plist(
                    "dev.example.app",
                    certificates,
                    "",
                    "",
                    expires,
                ))
            };
            let own = format!("<data>{}</data>", STANDARD.encode(first.key.certificate()));
            let others = [
                profile_resource(
                    "P90",
                    "IOS_APP_DEVELOPMENT",
                    "ACTIVE",
                    &profile(&own, "2030-01-01T00:00:00Z"),
                ),
                profile_resource(
                    "P91",
                    "IOS_APP_STORE",
                    "INVALID",
                    &profile(&own, "2030-01-01T00:00:00Z"),
                ),
                profile_resource("P92", "IOS_APP_STORE", "ACTIVE", "garbled"),
                profile_resource(
                    "P93",
                    "IOS_APP_STORE",
                    "ACTIVE",
                    &STANDARD.encode("not a profile"),
                ),
                profile_resource(
                    "P94",
                    "IOS_APP_STORE",
                    "ACTIVE",
                    &profile(&own, "2020-01-01T00:00:00Z"),
                ),
                profile_resource(
                    "P95",
                    "IOS_APP_STORE",
                    "ACTIVE",
                    &profile("", "2030-01-01T00:00:00Z"),
                ),
            ];
            for other in others.into_iter().rev() {
                account.profiles.insert(0, ("B2".to_owned(), other));
            }
        }
        requests.lock().unwrap().clear();

        let (second, events) = provision(&client, Distribution::AppStore, &bundles, &keystore);

        let second = second.unwrap();
        assert_eq!(events, []);
        assert!(creations(&requests).is_empty());
        assert_eq!(second.key.certificate(), first.key.certificate());
        assert_eq!(second.profiles, first.profiles);
    }

    #[test]
    fn provisions_for_development_on_the_accounts_devices() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let (client, account, requests) = app_store_connect(new_account());

        let (provisioned, events) = provision(
            &client,
            Distribution::Development,
            &bundles(None),
            &keystore,
        );

        let provisioned = provisioned.unwrap();
        assert_eq!(
            events[1],
            Event::CertificateCreated {
                path: keystore.display().to_string(),
                certificate: "Apple Development: OmniBuild Test (ABCDE12345)".to_owned(),
                expires: "2027-10-22".to_owned(),
            }
        );
        assert_eq!(events[3], profile_created("dev.example.app", "Development"));
        let created = creations(&requests);
        assert_eq!(
            created[0].body["data"]["attributes"]["certificateType"],
            "DEVELOPMENT"
        );
        let devices = json!({ "data": [{ "type": "devices", "id": "D1" }, { "type": "devices", "id": "D2" }] });
        assert_eq!(
            created[2].body["data"]["attributes"]["profileType"],
            "IOS_APP_DEVELOPMENT"
        );
        assert_eq!(created[2].body["data"]["relationships"]["devices"], devices);
        let profile = Profile::parse(&provisioned.profiles[0].1).unwrap();
        assert!(profile.lists("UDID1") && profile.lists("UDID2"));
        let requests = requests.lock().unwrap();
        let devices = requests
            .iter()
            .find(|request| request.path == "/v1/devices")
            .unwrap();
        assert_eq!(devices.query["filter[platform]"], "IOS");
        assert_eq!(devices.query["filter[status]"], "ENABLED");
        drop(requests);

        // A device registered since: each profile is made again to list it.
        account
            .lock()
            .unwrap()
            .devices
            .push(json!({ "type": "devices", "id": "D3", "attributes": { "udid": "UDID3" } }));

        let (again, events) = provision(
            &client,
            Distribution::Development,
            &bundles(None),
            &keystore,
        );

        assert_eq!(
            events,
            [
                profile_created("dev.example.app", "Development"),
                profile_created("dev.example.app.Share", "Development"),
                profile_created("dev.example.app.Clip", "Development"),
            ]
        );
        assert!(
            Profile::parse(&again.unwrap().profiles[0].1)
                .unwrap()
                .lists("UDID3")
        );
    }

    #[test]
    fn provisions_for_ad_hoc_distribution_on_the_accounts_devices() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, requests) = app_store_connect(new_account());

        let (provisioned, events) = provision(
            &client,
            Distribution::AdHoc,
            &bundles(None),
            &dir.path().join("apple.p12"),
        );

        let profile = Profile::parse(&provisioned.unwrap().profiles[0].1).unwrap();
        assert!(profile.lists("UDID1") && profile.lists("UDID2"));
        assert_eq!(events[3], profile_created("dev.example.app", "Ad Hoc"));
        let created = creations(&requests);
        assert_eq!(
            created[0].body["data"]["attributes"]["certificateType"],
            "DISTRIBUTION"
        );
        assert_eq!(
            created[2].body["data"]["attributes"]["profileType"],
            "IOS_APP_ADHOC"
        );
    }

    #[test]
    fn says_when_the_account_has_no_devices() {
        let dir = tempfile::tempdir().unwrap();
        let mut account = new_account();
        account.devices.clear();
        let (client, ..) = app_store_connect(account);

        let (provisioned, _) = provision(
            &client,
            Distribution::AdHoc,
            &bundles(None),
            &dir.path().join("apple.p12"),
        );

        assert_eq!(
            provisioned.err().unwrap().to_string(),
            "the account has no devices for Ad Hoc profiles to list: register each by its UDID at developer.apple.com, under Devices"
        );
    }

    #[test]
    fn reuses_the_certificate_issued_for_a_keys_request() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let request = dir.path().join("apple.certSigningRequest");
        key::request(&keystore, &request, &"CN=Key".parse().unwrap(), PASSWORD).unwrap();
        // Issued at developer.apple.com, of the iOS-only kind, among another
        // key's and one App Store Connect can't give.
        let issued = issue_for(
            &fs::read_to_string(&request).unwrap(),
            "CN=iPhone Distribution: OmniBuild Test (ABCDE12345)",
            9,
        );
        let other = crate::ios::code_signature::tests::identity();
        let mut account = new_account();
        account.certificates = vec![
            certificate_resource("C7", "DISTRIBUTION", &STANDARD.encode(other.certificate())),
            certificate_resource("C8", "DISTRIBUTION", "garbled"),
            certificate_resource(
                "C9",
                "IOS_DISTRIBUTION",
                &STANDARD.encode(issued.to_der().unwrap()),
            ),
        ];
        let (client, _, requests) = app_store_connect(account);

        let (provisioned, events) =
            provision(&client, Distribution::AppStore, &bundles(None), &keystore);

        let provisioned = provisioned.unwrap();
        assert_eq!(provisioned.key.x509(), &issued);
        assert_eq!(
            SigningKey::read(&keystore, PASSWORD).unwrap().x509(),
            &issued
        );
        assert_eq!(events[0], app_id_registered("dev.example.app"));
        assert_eq!(
            creations(&requests)[1].body["data"]["relationships"]["certificates"],
            json!({ "data": [{ "type": "certificates", "id": "C9" }] })
        );
    }

    #[test]
    fn keeps_a_keys_certificate_for_its_own_kind_of_profile() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let (client, ..) = app_store_connect(new_account());
        provision(
            &client,
            Distribution::Development,
            &bundles(None),
            &keystore,
        )
        .0
        .unwrap();

        let (provisioned, events) =
            provision(&client, Distribution::AppStore, &bundles(None), &keystore);

        assert_eq!(
            provisioned.err().unwrap().to_string(),
            format!(
                "the certificate in {}, \"Apple Development: OmniBuild Test (ABCDE12345)\", isn't one App Store profiles allow: give them another keystore with --signing-key, which OmniBuild creates if there's none",
                keystore.display()
            )
        );
        assert_eq!(events, []);
    }

    #[test]
    fn replaces_a_certificate_the_account_no_longer_has() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let (client, account, requests) = app_store_connect(new_account());
        let first = provision(&client, Distribution::AppStore, &bundles(None), &keystore)
            .0
            .unwrap();
        // Revoked.
        account.lock().unwrap().certificates.clear();
        requests.lock().unwrap().clear();

        let (second, events) =
            provision(&client, Distribution::AppStore, &bundles(None), &keystore);

        let second = second.unwrap();
        assert_eq!(
            events[0],
            Event::CertificateCreated {
                path: keystore.display().to_string(),
                certificate: "Apple Distribution: OmniBuild Test (ABCDE12345)".to_owned(),
                expires: "2027-10-22".to_owned(),
            }
        );
        assert_eq!(events[1], profile_created("dev.example.app", "App Store"));
        // For the same key, and allowed by the profiles made again.
        assert_ne!(second.key.certificate(), first.key.certificate());
        assert_eq!(second.key.public_key(), first.key.public_key());
        let created = creations(&requests);
        let new_id = &account.lock().unwrap().certificates[0]["id"];
        assert_eq!(
            created[1].body["data"]["relationships"]["certificates"]["data"][0]["id"],
            *new_id
        );
        let read = SigningKey::read(&keystore, PASSWORD).unwrap();
        assert_eq!(read.certificate(), second.key.certificate());
    }

    #[test]
    fn makes_the_profiles_of_an_app_id_whose_capabilities_changed_again() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let (client, ..) = app_store_connect(new_account());
        provision(&client, Distribution::AppStore, &bundles(None), &keystore)
            .0
            .unwrap();

        let (provisioned, events) = provision(
            &client,
            Distribution::AppStore,
            &bundles(Some(push_and_links())),
            &keystore,
        );

        provisioned.unwrap();
        assert_eq!(
            events,
            [
                Event::CapabilityEnabled {
                    bundle_id: "dev.example.app".to_owned(),
                    capability: "Associated Domains".to_owned(),
                },
                Event::CapabilityEnabled {
                    bundle_id: "dev.example.app".to_owned(),
                    capability: "Push Notifications".to_owned(),
                },
                profile_created("dev.example.app", "App Store"),
            ]
        );
    }

    #[test]
    fn says_what_an_app_id_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let (client, ..) = app_store_connect(new_account());
        // Push notifications, which it turns on, and iCloud, which takes
        // settings it doesn't know.
        let app_claims = || {
            Some(claims(&[
                (
                    "aps-environment",
                    Entitlement::String("production".to_owned()),
                ),
                (
                    "com.apple.developer.icloud-services",
                    Entitlement::Strings(vec!["CloudKit".to_owned()]),
                ),
            ]))
        };
        let lacks = "dev.example.app's App ID lacks what it claims: turn it on at developer.apple.com, under Identifiers, where the App ID's App Groups, iCloud containers and merchant IDs are chosen too, then provision again: the provisioning profile for dev.example.app doesn't allow com.apple.developer.icloud-services, which it claims in ios.entitlements: turn the capability on for its App ID, and make the profile again";

        let (provisioned, events) = provision(
            &client,
            Distribution::AppStore,
            &bundles(app_claims()),
            &keystore,
        );

        assert_eq!(format!("{:#}", provisioned.err().unwrap()), lacks);
        assert_eq!(
            events.last(),
            Some(&profile_created("dev.example.app", "App Store"))
        );

        // The profile it made is no better the next time, so it makes none.
        let (again, events) = provision(
            &client,
            Distribution::AppStore,
            &bundles(app_claims()),
            &keystore,
        );

        assert_eq!(format!("{:#}", again.err().unwrap()), lacks);
        assert_eq!(events, []);
    }

    #[test]
    fn says_when_an_app_clips_app_id_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let (client, ..) = app_store_connect(Account::default());

        let (provisioned, _) = provision(
            &client,
            Distribution::AppStore,
            &bundles(None),
            &dir.path().join("apple.p12"),
        );

        assert_eq!(
            provisioned.err().unwrap().to_string(),
            "dev.example.app.Clip is an App Clip's ID, which App Store Connect's API can't register: register it at developer.apple.com, under Identifiers, as an App Clip of dev.example.app, then provision again"
        );
    }

    #[test]
    fn says_when_an_app_clips_app_id_isnt_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut account = new_account();
        // Registered as an app's.
        account.app_clips.clear();
        let (client, ..) = app_store_connect(account);

        let (provisioned, _) = provision(
            &client,
            Distribution::AppStore,
            &bundles(None),
            &dir.path().join("apple.p12"),
        );

        let error = format!("{:#}", provisioned.err().unwrap());
        assert!(
            error.starts_with("dev.example.app.Clip's App ID lacks what it claims: ")
                && error.ends_with(": it isn't an App Clip's: it doesn't allow com.apple.developer.on-demand-install-capable, as the profile of an App ID registered as an App Clip's does"),
            "{error}"
        );
    }

    #[test]
    fn says_what_app_store_connect_refused() {
        let refused = ": App Store Connect refused it (409). It said: Refused. The test says so.";
        let cases = [
            (
                "GET",
                "/v1/certificates",
                Distribution::AppStore,
                "couldn't list the account's certificates",
            ),
            (
                "POST",
                "/v1/certificates",
                Distribution::AppStore,
                "couldn't create a certificate for the key in ",
            ),
            (
                "GET",
                "/v1/devices",
                Distribution::Development,
                "couldn't list the account's devices",
            ),
            (
                "GET",
                "/v1/bundleIds",
                Distribution::AppStore,
                "couldn't look for the App ID dev.example.app",
            ),
            (
                "POST",
                "/v1/bundleIds",
                Distribution::AppStore,
                "couldn't register the App ID dev.example.app",
            ),
            (
                "GET",
                "/bundleIdCapabilities",
                Distribution::AppStore,
                "couldn't list the capabilities of dev.example.app",
            ),
            (
                "POST",
                "/v1/bundleIdCapabilities",
                Distribution::AppStore,
                "couldn't turn on Associated Domains for dev.example.app",
            ),
            (
                "GET",
                "/profiles",
                Distribution::AppStore,
                "couldn't list the profiles of dev.example.app.Share",
            ),
            (
                "POST",
                "/v1/profiles",
                Distribution::AppStore,
                "couldn't create a provisioning profile for dev.example.app",
            ),
        ];
        for (method, path, distribution, says) in cases {
            let dir = tempfile::tempdir().unwrap();
            let mut account = new_account();
            account.refuses = Some((method, path));
            let (client, ..) = app_store_connect(account);

            let (provisioned, _) = provision(
                &client,
                distribution,
                &bundles(Some(push_and_links())),
                &dir.path().join("apple.p12"),
            );

            let error = format!("{:#}", provisioned.err().unwrap());
            assert!(
                error.starts_with(says) && error.ends_with(refused),
                "{error}"
            );
        }
    }

    #[test]
    fn says_when_what_app_store_connect_made_is_garbled() {
        let dir = tempfile::tempdir().unwrap();
        let keystore = dir.path().join("apple.p12");
        let mut account = new_account();
        account.garbles = true;
        let (client, account, _) = app_store_connect(account);

        let (certificate, _) =
            provision(&client, Distribution::AppStore, &bundles(None), &keystore);

        assert_eq!(
            certificate.err().unwrap().to_string(),
            "the certificate App Store Connect created isn't one"
        );

        // With a certificate, the profile it makes is garbled.
        account.lock().unwrap().garbles = false;
        provision(&client, Distribution::AppStore, &bundles(None), &keystore)
            .0
            .unwrap();
        let mut garbling = account.lock().unwrap();
        garbling.garbles = true;
        garbling.profiles.clear();
        drop(garbling);

        let (profile, _) = provision(&client, Distribution::AppStore, &bundles(None), &keystore);

        assert_eq!(
            profile.err().unwrap().to_string(),
            "the provisioning profile App Store Connect created for dev.example.app isn't one"
        );
    }

    #[test]
    fn says_what_it_cant_do_with_the_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let (client, ..) = app_store_connect(new_account());
        let keystore = dir.path().join("apple.p12");
        provision(&client, Distribution::AppStore, &bundles(None), &keystore)
            .0
            .unwrap();
        let attempt = |keystore: &Path, password| {
            provision_with(
                &client,
                Distribution::AppStore,
                &bundles(None),
                keystore,
                password,
            )
        };

        let (wrong_password, wrong_events) = attempt(&keystore, "wrong password");
        let (short_password, short_events) = attempt(&dir.path().join("new.p12"), "short");

        assert!(
            format!("{:#}", wrong_password.err().unwrap())
                .ends_with("apple.p12: the password isn't its password (OMNI_BUILD_KEY_PASSWORD)")
        );
        assert!(short_password.err().unwrap().to_string().starts_with(
            "the keystore's password (OMNI_BUILD_KEY_PASSWORD) has to have at least 6 characters"
        ));
        assert_eq!(wrong_events, []);
        assert_eq!(short_events, []);
    }

    #[cfg(unix)]
    #[test]
    fn says_when_it_cant_save_the_certificate() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        // A key whose certificate the account has, and one it has none for.
        let issued = keys.join("issued.p12");
        let request = dir.path().join("issued.certSigningRequest");
        key::request(&issued, &request, &"CN=Key".parse().unwrap(), PASSWORD).unwrap();
        let certificate = issue_for(
            &fs::read_to_string(&request).unwrap(),
            "CN=Apple Distribution: OmniBuild Test (ABCDE12345)",
            9,
        );
        key::create_apple_key(&keys.join("new.p12"), "Key", PASSWORD).unwrap();
        let mut account = new_account();
        account.certificates.push(certificate_resource(
            "C9",
            "DISTRIBUTION",
            &STANDARD.encode(certificate.to_der().unwrap()),
        ));
        let (client, ..) = app_store_connect(account);
        // A folder no file can be added to.
        fs::set_permissions(&keys, fs::Permissions::from_mode(0o500)).unwrap();

        let errors: Vec<_> = ["issued.p12", "new.p12"]
            .iter()
            .map(|name| {
                provision(
                    &client,
                    Distribution::AppStore,
                    &bundles(None),
                    &keys.join(name),
                )
                .0
                .err()
                .unwrap()
                .to_string()
            })
            .collect();

        fs::set_permissions(&keys, fs::Permissions::from_mode(0o700)).unwrap();
        for error in errors {
            assert!(
                error.starts_with("couldn't save the keystore to "),
                "{error}"
            );
        }
    }
}
