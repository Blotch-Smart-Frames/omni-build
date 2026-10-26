//! App Store Connect's API, through which OmniBuild manages what an Apple
//! developer account has for signing apps: certificates, App IDs, devices
//! and provisioning profiles. Requests carry a token the user's API key
//! signs, a key the account's Admin creates in App Store Connect, under
//! Users and Access > Integrations: a P-256 key, as a `.p8` file, its ID,
//! and for a team's key, the team's issuer ID. Resources are JSON:API
//! documents.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use ureq::Agent;
use ureq::http::Response;

use crate::http;

/// Where the API is.
pub const URL: &str = "https://api.appstoreconnect.apple.com";
/// Who a token is for, as Apple asks.
const AUDIENCE: &str = "appstoreconnect-v1";
/// How long a token lasts. Apple refuses one that lasts more than 20
/// minutes, and each request gets a new one, so a few will do, and leave
/// room for a clock that's a little off.
const TOKEN_LIFETIME: u64 = 10 * 60;
/// The most resources a page of a list has: the most Apple allows.
const PAGE: &str = "200";
/// Why OmniBuild can't take an answer that's JSON, but not of the shape
/// App Store Connect's API gives.
const UNDOCUMENTED: &str = "App Store Connect answered with what its API doesn't document";

/// An App Store Connect API key.
pub struct ApiKey {
    id: String,
    /// The team's issuer ID, for a team key. An individual key, which acts
    /// as the user who created it, has none.
    issuer: Option<String>,
    key: SigningKey,
}

impl ApiKey {
    /// The key in the file at `path`, as App Store Connect gives it
    /// (`AuthKey_<key ID>.p8`), with its ID, `id`, or the one its file is
    /// named by, and the team's issuer ID, `issuer`, for a team key.
    pub fn read(path: &Path, id: Option<&str>, issuer: Option<&str>) -> Result<Self> {
        let pem = fs::read_to_string(path)
            .with_context(|| format!("couldn't read the API key {}", path.display()))?;
        let key = SigningKey::from_pkcs8_pem(&pem).ok().with_context(|| {
            format!(
                "{} isn't an App Store Connect API key: a P-256 private key in PEM, as App Store Connect gives one",
                path.display()
            )
        })?;
        let id = match id {
            Some(id) => id,
            None => path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("AuthKey_"))
                .filter(|id| !id.is_empty())
                .with_context(|| {
                    format!(
                        "give the API key's ID with --api-key-id: {} isn't named by it, as AuthKey_<key ID>.p8",
                        path.display()
                    )
                })?,
        };
        Ok(Self {
            id: id.to_owned(),
            issuer: issuer.map(str::to_owned),
            key,
        })
    }

    /// A token for requests made from `now`, in seconds since 1970: a JSON
    /// Web Token, signed with ES256.
    fn token(&self, now: u64) -> String {
        let header = json!({ "alg": "ES256", "kid": self.id, "typ": "JWT" });
        let expires = now + TOKEN_LIFETIME;
        let claims = match &self.issuer {
            Some(issuer) => {
                json!({ "iss": issuer, "iat": now, "exp": expires, "aud": AUDIENCE })
            }
            None => json!({ "sub": "user", "iat": now, "exp": expires, "aud": AUDIENCE }),
        };
        let message = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature: Signature = self.key.sign(message.as_bytes());
        format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }
}

/// A resource, as App Store Connect gives one: its ID, and its attributes,
/// of which each kind's caller reads those it needs.
#[derive(Debug, Deserialize)]
pub struct Resource<A> {
    pub id: String,
    pub attributes: A,
}

/// A page of a list, its resources as JSON.
#[derive(Deserialize)]
struct Page {
    data: Vec<Value>,
    #[serde(default)]
    links: Links,
}

/// A resource that was created, as JSON.
#[derive(Deserialize)]
struct Created {
    data: Value,
}

#[derive(Default, Deserialize)]
struct Links {
    /// The URL of a list's next page, if it has more.
    next: Option<String>,
}

/// Why App Store Connect refused a request.
#[derive(Deserialize)]
struct Errors {
    errors: Vec<Error>,
}

#[derive(Deserialize)]
struct Error {
    title: String,
    #[serde(default)]
    detail: String,
}

/// Requests to App Store Connect's API, with an API key.
pub struct Client {
    agent: Agent,
    /// The API's URL: Apple's, or a test's.
    url: String,
    key: ApiKey,
}

impl Client {
    /// A client of the API at `url`, Apple's (`URL`) but in tests, with
    /// `key`.
    pub fn new(key: ApiKey, url: &str) -> Self {
        Self {
            agent: http::agent(),
            url: url.trim_end_matches('/').to_owned(),
            key,
        }
    }

    /// Every resource at `path`, such as `/v1/devices`, that `filters`
    /// select, from every page of the list.
    pub fn list<A: DeserializeOwned>(
        &self,
        path: &str,
        filters: &[(&str, &str)],
    ) -> Result<Vec<Resource<A>>> {
        // Requested as JSON, and typed in one go, so that the code each kind
        // of resource has its own copy of is only this.
        let resources = self.list_json(path, filters)?;
        serde_json::from_value(Value::Array(resources)).context(UNDOCUMENTED)
    }

    /// Creates the resource `data` describes at `path`, such as
    /// `/v1/profiles`, and returns it as App Store Connect made it.
    pub fn create<A: DeserializeOwned>(&self, path: &str, data: &Value) -> Result<Resource<A>> {
        let created = self.create_json(path, data)?;
        serde_json::from_value(created).context(UNDOCUMENTED)
    }

    /// Every resource `list` gives, as JSON.
    fn list_json(&self, path: &str, filters: &[(&str, &str)]) -> Result<Vec<Value>> {
        let first = self
            .agent
            .get(format!("{}{path}", self.url))
            .header("Authorization", self.authorization())
            .query_pairs(filters.iter().copied().chain([("limit", PAGE)]))
            .call();
        let mut page = page(first)?;
        let mut resources = Vec::new();
        loop {
            resources.append(&mut page.data);
            // The next page's URL has the query that selects it.
            let Some(next) = page.links.next else {
                return Ok(resources);
            };
            let response = self
                .agent
                .get(next)
                .header("Authorization", self.authorization())
                .call();
            page = self::page(response)?;
        }
    }

    /// The resource `create` creates, as JSON.
    fn create_json(&self, path: &str, data: &Value) -> Result<Value> {
        let response = self
            .agent
            .post(format!("{}{path}", self.url))
            .header("Authorization", self.authorization())
            .content_type("application/json")
            .send(json!({ "data": data }).to_string());
        let created: Created = serde_json::from_value(read(response)?).context(UNDOCUMENTED)?;
        Ok(created.data)
    }

    /// The header that authenticates a request made now.
    fn authorization(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("Bearer {}", self.key.token(now))
    }
}

/// A page of a list App Store Connect answered with, or why it refused.
fn page(response: Result<Response<ureq::Body>, ureq::Error>) -> Result<Page> {
    serde_json::from_value(read(response)?).context(UNDOCUMENTED)
}

/// The document App Store Connect answered with, or why it refused.
fn read(response: Result<Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    let mut response = response.context("couldn't reach App Store Connect")?;
    let body = response
        .body_mut()
        .read_to_string()
        .context("couldn't read App Store Connect's answer")?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        bail!(refusal(status, &body));
    }
    serde_json::from_str(&body).context(UNDOCUMENTED)
}

/// Why App Store Connect refused a request, given the status it answered
/// with, and the document that says why, if it is one.
fn refusal(status: u16, body: &str) -> String {
    let reasons = serde_json::from_str::<Errors>(body)
        .map(|errors| {
            errors
                .errors
                .iter()
                .map(|error| {
                    format!("{} {}", error.title, error.detail)
                        .trim()
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let said = if reasons.is_empty() {
        String::new()
    } else {
        format!(". It said: {reasons}")
    };
    // The key is refused the same way whichever of its parts is wrong.
    if status == 401 {
        format!(
            "App Store Connect didn't accept the API key: check its ID (--api-key-id) and its issuer ID (--api-issuer), as App Store Connect shows them under Users and Access > Integrations, and that it hasn't been revoked{said}"
        )
    } else {
        format!("App Store Connect refused it ({status}){said}")
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufReader, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::thread;

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::{Signature, VerifyingKey};
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::{ApiKey, Client, Resource};
    use crate::http::tests::{read_request, serve};

    /// The test API key, as App Store Connect would name it.
    pub fn key_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/AuthKey_TESTKEY123.p8")
    }

    pub fn api_key() -> ApiKey {
        ApiKey::read(&key_path(), None, Some("issuer")).unwrap()
    }

    /// A JSON:API document of `data`.
    pub fn document(data: &Value) -> String {
        json!({ "data": data }).to_string()
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Device {
        udid: String,
    }

    fn device(id: u32) -> Value {
        json!({ "type": "devices", "id": format!("D{id}"), "attributes": { "udid": format!("UDID{id}") } })
    }

    fn client(url: &str) -> Client {
        Client::new(api_key(), url)
    }

    /// The token's three parts, its header and claims decoded.
    fn token_parts(token: &str) -> (Value, Value, String, Vec<u8>) {
        let parts: Vec<_> = token.split('.').collect();
        let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).unwrap();
        (
            serde_json::from_slice(&decode(parts[0])).unwrap(),
            serde_json::from_slice(&decode(parts[1])).unwrap(),
            format!("{}.{}", parts[0], parts[1]),
            decode(parts[2]),
        )
    }

    #[test]
    fn signs_a_token_for_a_teams_key() {
        let key = api_key();

        let (header, claims, message, signature) = token_parts(&key.token(1_761_400_000));

        assert_eq!(
            header,
            json!({ "alg": "ES256", "kid": "TESTKEY123", "typ": "JWT" })
        );
        assert_eq!(
            claims,
            json!({ "iss": "issuer", "iat": 1_761_400_000, "exp": 1_761_400_600, "aud": "appstoreconnect-v1" })
        );
        let signature = Signature::from_slice(&signature).unwrap();
        assert!(
            VerifyingKey::from(&key.key)
                .verify(message.as_bytes(), &signature)
                .is_ok()
        );
    }

    #[test]
    fn signs_a_token_for_an_individual_key() {
        let key = ApiKey::read(&key_path(), Some("TESTKEY456"), None).unwrap();

        let (header, claims, ..) = token_parts(&key.token(1_761_400_000));

        assert_eq!(header["kid"], "TESTKEY456");
        assert_eq!(
            claims,
            json!({ "sub": "user", "iat": 1_761_400_000, "exp": 1_761_400_600, "aud": "appstoreconnect-v1" })
        );
    }

    #[test]
    fn says_what_it_cant_read_of_an_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let unnamed = dir.path().join("key.p8");
        std::fs::copy(key_path(), &unnamed).unwrap();
        let bare = dir.path().join("AuthKey_.p8");
        std::fs::copy(key_path(), &bare).unwrap();
        let rsa = dir.path().join("AuthKey_RSA.p8");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/identity.pem"),
            &rsa,
        )
        .unwrap();
        let error = |path: &Path| format!("{:#}", ApiKey::read(path, None, None).err().unwrap());

        assert!(error(&dir.path().join("missing.p8")).starts_with("couldn't read the API key "));
        assert!(error(&rsa).ends_with(
            "AuthKey_RSA.p8 isn't an App Store Connect API key: a P-256 private key in PEM, as App Store Connect gives one"
        ));
        for path in [unnamed, bare] {
            assert!(error(&path).starts_with("give the API key's ID with --api-key-id: "));
            assert!(error(&path).ends_with(".p8 isn't named by it, as AuthKey_<key ID>.p8"));
        }
    }

    #[test]
    fn lists_every_page() {
        let (url, requests) = serve(|request, url| match request.query.get("cursor") {
            None => (
                200,
                json!({
                    "data": [device(1), device(2)],
                    "links": { "self": "", "next": format!("{url}/v1/devices?cursor=2&limit=200") },
                })
                .to_string(),
            ),
            Some(_) => (200, document(&json!([device(3)]))),
        });

        let devices: Vec<Resource<Device>> = client(&format!("{url}/"))
            .list(
                "/v1/devices",
                &[
                    ("filter[platform]", "IOS"),
                    ("filter[status]", "ENABLED,DISABLED"),
                ],
            )
            .unwrap();

        let udids: Vec<_> = devices
            .iter()
            .map(|device| device.attributes.udid.as_str())
            .collect();
        assert_eq!(udids, ["UDID1", "UDID2", "UDID3"]);
        assert_eq!(devices[2].id, "D3");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/v1/devices");
        assert_eq!(
            requests[0].query,
            BTreeMap::from([
                ("filter[platform]".to_owned(), "IOS".to_owned()),
                ("filter[status]".to_owned(), "ENABLED,DISABLED".to_owned()),
                ("limit".to_owned(), "200".to_owned()),
            ])
        );
        assert_eq!(requests[1].query["cursor"], "2");
        // Each request with a token of its own, which the key signed.
        for request in requests.iter() {
            let token = request.authorization.strip_prefix("Bearer ").unwrap();
            let (header, claims, ..) = token_parts(token);
            assert_eq!(header["kid"], "TESTKEY123");
            assert_eq!(claims["iss"], "issuer");
        }
    }

    #[test]
    fn says_why_it_couldnt_list_a_later_page() {
        let (url, _) = serve(|request, url| {
            match request.query.get("cursor") {
            None => (
                200,
                json!({ "data": [device(1)], "links": { "next": format!("{url}/v1/devices?cursor=2") } })
                    .to_string(),
            ),
            Some(_) => (500, String::new()),
        }
        });

        let error = client(&url)
            .list::<Device>("/v1/devices", &[])
            .err()
            .unwrap();

        assert_eq!(format!("{error:#}"), "App Store Connect refused it (500)");
    }

    #[test]
    fn creates_a_resource() {
        let (url, requests) = serve(|_, _| (201, document(&device(4))));

        let created: Resource<Device> = client(&url)
            .create(
                "/v1/devices",
                &json!({ "type": "devices", "attributes": { "udid": "UDID4" } }),
            )
            .unwrap();

        assert_eq!(created.id, "D4");
        assert_eq!(
            created.attributes,
            Device {
                udid: "UDID4".to_owned()
            }
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/v1/devices");
        assert_eq!(
            requests[0].body,
            json!({ "data": { "type": "devices", "attributes": { "udid": "UDID4" } } })
        );
    }

    #[test]
    fn says_why_app_store_connect_refused() {
        let errors = |errors: Value| json!({ "errors": errors }).to_string();
        let answers = [
            (
                401,
                errors(json!([{
                    "status": "401",
                    "code": "NOT_AUTHORIZED",
                    "title": "Authentication credentials are missing or invalid.",
                    "detail": "Provide a properly configured and signed bearer token.",
                }])),
                "App Store Connect didn't accept the API key: check its ID (--api-key-id) and its issuer ID (--api-issuer), as App Store Connect shows them under Users and Access > Integrations, and that it hasn't been revoked. It said: Authentication credentials are missing or invalid. Provide a properly configured and signed bearer token.",
            ),
            (
                401,
                "Unauthenticated".to_owned(),
                "App Store Connect didn't accept the API key: check its ID (--api-key-id) and its issuer ID (--api-issuer), as App Store Connect shows them under Users and Access > Integrations, and that it hasn't been revoked",
            ),
            (
                409,
                errors(json!([
                    { "title": "There is a problem with the request entity", "detail": "You already have a current Distribution certificate." },
                    { "title": "Another." },
                ])),
                "App Store Connect refused it (409). It said: There is a problem with the request entity You already have a current Distribution certificate. Another.",
            ),
            (
                502,
                "<html>Bad gateway</html>".to_owned(),
                "App Store Connect refused it (502)",
            ),
            (
                200,
                "{\"data\": 1}".to_owned(),
                "App Store Connect answered with what its API doesn't document: invalid type: integer `1`, expected a sequence",
            ),
            (
                200,
                "<html>Maintenance</html>".to_owned(),
                "App Store Connect answered with what its API doesn't document: expected value at line 1 column 1",
            ),
            (
                200,
                json!({ "data": [{ "id": "D1" }] }).to_string(),
                "App Store Connect answered with what its API doesn't document: missing field `attributes`",
            ),
        ];
        for (status, body, message) in answers {
            let (url, _) = serve(move |_, _| (status, body.clone()));

            let error = client(&url)
                .list::<Device>("/v1/devices", &[])
                .err()
                .unwrap();

            assert_eq!(format!("{error:#}"), message);
        }
    }

    #[test]
    fn says_when_what_it_created_isnt_documented() {
        let (url, _) = serve(|_, _| (201, "{}".to_owned()));

        let error = client(&url)
            .create::<Device>("/v1/devices", &json!({}))
            .err()
            .unwrap();

        assert_eq!(
            format!("{error:#}"),
            "App Store Connect answered with what its API doesn't document: missing field `data`"
        );
    }

    #[test]
    fn says_when_it_cant_reach_app_store_connect() {
        // A port nothing listens on any more.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        let error = client(&format!("http://127.0.0.1:{port}"))
            .create::<Device>("/v1/devices", &json!({}))
            .err()
            .unwrap();

        assert!(format!("{error:#}").starts_with("couldn't reach App Store Connect: "));
    }

    #[test]
    fn says_when_it_cant_read_the_answer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // An answer that ends before the body it promises.
        thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            read_request(&mut BufReader::new(&mut stream));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"data\"")
                .unwrap();
        });

        let error = client(&url)
            .list::<Device>("/v1/devices", &[])
            .err()
            .unwrap();

        assert!(format!("{error:#}").starts_with("couldn't read App Store Connect's answer: "));
    }
}
