//! Google Play's Developer API, through which OmniBuild uploads app bundles
//! and releases them to a track. Requests carry an access token that
//! Google's OAuth 2.0 server issues to a Google Cloud service account, for
//! a token the account's own key signs, as Google's libraries get one. The
//! key is a JSON file Google Cloud gives, with the account's email address
//! and an RSA private key, and the Play Console has to have invited the
//! account, under Users and permissions, with permission to release the
//! app.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::RsaPrivateKey;
use rsa::pkcs1v15;
use rsa::pkcs8::DecodePrivateKey;
use rsa::sha2::Sha256;
use rsa::signature::{SignatureEncoding, Signer};
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::json;
use ureq::http::Response;
use ureq::typestate::WithBody;
use ureq::{Agent, AsSendBody, RequestBuilder};

use crate::http;

/// Where the API is.
pub const URL: &str = "https://androidpublisher.googleapis.com";
/// What the access token allows: Google Play's API, and nothing else of
/// the account's.
const SCOPE: &str = "https://www.googleapis.com/auth/androidpublisher";
/// How the service account asks for an access token: with a token of its
/// own, a JSON Web Token, as RFC 7523 has it.
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// How long the token the key signs lasts: an hour, the most Google
/// allows. It's spent at once, so this only leaves room for a clock that's
/// a little off.
const ASSERTION_LIFETIME: u64 = 60 * 60;
/// Why OmniBuild can't take an answer that's JSON, but not of the shape
/// Google Play's API gives.
const UNDOCUMENTED: &str = "Google Play answered with what its API doesn't document";

/// A Google Cloud service account's key, as the JSON file Google Cloud
/// gives has it.
#[derive(Deserialize)]
struct KeyFile {
    #[serde(rename = "type")]
    kind: String,
    client_email: String,
    private_key_id: String,
    /// PKCS #8, in PEM.
    private_key: String,
    /// Where Google issues access tokens for the key.
    token_uri: String,
}

/// A Google Cloud service account, with its key.
pub struct ServiceAccount {
    email: String,
    /// The key's ID, which tells Google which of the account's keys signed.
    key_id: String,
    key: pkcs1v15::SigningKey<Sha256>,
    token_uri: String,
}

impl ServiceAccount {
    /// The account whose key is in the file at `path`, as Google Cloud
    /// gives it.
    pub fn read(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path).with_context(|| {
            format!("couldn't read the service account's key {}", path.display())
        })?;
        let not_a_key = || {
            format!(
                "{} isn't a Google Cloud service account's key: the JSON file Google Cloud creates for one, under its Keys",
                path.display()
            )
        };
        let file = serde_json::from_str::<KeyFile>(&json)
            .ok()
            .filter(|file| file.kind == "service_account")
            .with_context(not_a_key)?;
        let key = RsaPrivateKey::from_pkcs8_pem(&file.private_key)
            .ok()
            .with_context(not_a_key)?;
        Ok(Self {
            email: file.client_email,
            key_id: file.private_key_id,
            key: pkcs1v15::SigningKey::new(key),
            token_uri: file.token_uri,
        })
    }

    /// The token that asks for access from `now`, in seconds since 1970: a
    /// JSON Web Token, signed with RS256.
    fn assertion(&self, now: u64) -> String {
        let header = json!({ "alg": "RS256", "kid": self.key_id, "typ": "JWT" });
        let claims = json!({
            "iss": self.email,
            "scope": SCOPE,
            "aud": self.token_uri,
            "iat": now,
            "exp": now + ASSERTION_LIFETIME,
        });
        let message = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature = self.key.sign(message.as_bytes());
        format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }
}

/// The access token Google issued.
#[derive(Deserialize)]
struct Token {
    access_token: String,
}

/// An edit of an app: changes Google Play publishes together, once it's
/// committed.
#[derive(Deserialize)]
struct Edit {
    id: String,
}

/// An app bundle Google Play took.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    version_code: u32,
}

/// Why Google refused, in either kind of document Google writes it in: its
/// APIs' (`{"error": {"message": ...}}`) or its OAuth 2.0 server's
/// (`{"error": "invalid_grant", "error_description": ...}`).
#[derive(Deserialize)]
#[serde(untagged)]
enum Errors {
    Api {
        error: ApiError,
    },
    OAuth {
        error: String,
        #[serde(default)]
        error_description: String,
    },
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

/// Requests to Google Play's API, as a service account.
pub struct Client {
    agent: Agent,
    /// The API's URL: Google's, or a test's.
    url: String,
    /// The header that authenticates each request.
    authorization: String,
}

impl Client {
    /// Signs in to Google as `account`, for requests to the API at `url`,
    /// Google Play's (`URL`) but in tests.
    pub fn sign_in(account: &ServiceAccount, url: &str) -> Result<Self> {
        let agent = http::agent();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let response = agent.post(&account.token_uri).send_form([
            ("grant_type", GRANT_TYPE),
            ("assertion", &account.assertion(now)),
        ]);
        let (status, body) = answer(response, "Google")?;
        if !is_success(status) {
            bail!(
                "Google didn't accept the service account's key: check that neither the key nor the account has been deleted or disabled in Google Cloud{}",
                said(&body)
            );
        }
        let token: Token = serde_json::from_str(&body)
            .context("Google answered with what its OAuth 2.0 server doesn't document")?;
        Ok(Self {
            agent,
            url: url.trim_end_matches('/').to_owned(),
            authorization: format!("Bearer {}", token.access_token),
        })
    }

    /// Uploads `bundle`, the app bundle of the app `package`, and releases
    /// it to `track`: rolled out, or if `draft`, as a draft to roll out in
    /// the Play Console. Returns the version code Google Play read from it.
    pub fn release(&self, package: &str, bundle: &[u8], track: &str, draft: bool) -> Result<u32> {
        // The changes go in an edit, which Google Play publishes all at once
        // when it's committed, so a failure on the way leaves the app as it
        // was. An edit that isn't committed expires by itself.
        let app = format!("androidpublisher/v3/applications/{package}");
        let request = self
            .agent
            .post(format!("{}/{app}/edits", self.url))
            .content_type("application/json");
        let (status, body) = self.send(request, "{}")?;
        // The app is all the request names, so it's what's missing. Google
        // Play knows an app by its ID only once the Play Console has had a
        // bundle of it.
        if status == 404 {
            bail!(
                "Google Play has no app {package} that the service account can release: create the app in the Play Console, and upload its first app bundle there, by hand, which gives it its ID. Then invite the service account under Users and permissions, with permission to release it"
            );
        }
        let edit: Edit = read(status, &body)
            .with_context(|| format!("couldn't open an edit of {package} on Google Play"))?;
        let edit = format!("{app}/edits/{}", edit.id);
        // A plain upload, of the whole bundle in one request, which Google
        // Play's API takes up to its limit for app bundles.
        let request = http::upload(
            self.agent
                .post(format!("{}/upload/{edit}/bundles", self.url)),
        )
        .query("uploadType", "media")
        .content_type("application/octet-stream");
        let uploaded: Bundle = self
            .json(request, bundle)
            .context("couldn't upload the app bundle to Google Play")?;
        let release = json!({
            "track": track,
            "releases": [{
                "versionCodes": [uploaded.version_code.to_string()],
                "status": if draft { "draft" } else { "completed" },
            }],
        });
        let request = self
            .agent
            .put(format!("{}/{edit}/tracks/{}", self.url, segment(track)))
            .content_type("application/json");
        self.json::<IgnoredAny>(request, release.to_string())
            .with_context(|| format!("couldn't release it to the {track} track"))?;
        let request = self.agent.post(format!("{}/{edit}:commit", self.url));
        self.json::<IgnoredAny>(request, &[][..])
            .context("couldn't commit the edit, which publishes the release")?;
        Ok(uploaded.version_code)
    }

    /// Sends `request`, with `body`, and returns the status Google Play
    /// answered with, and what it answered.
    fn send(
        &self,
        request: RequestBuilder<WithBody>,
        body: impl AsSendBody,
    ) -> Result<(u16, String)> {
        let request = request.header("Authorization", &self.authorization);
        answer(request.send(body), "Google Play")
    }

    /// The document Google Play answered `request` with, or why it refused.
    fn json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder<WithBody>,
        body: impl AsSendBody,
    ) -> Result<T> {
        let (status, body) = self.send(request, body)?;
        read(status, &body)
    }
}

/// Whether `status` says a request succeeded, as HTTP's 2xx do.
fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// The status `service` answered with, and what it answered.
fn answer(
    response: Result<Response<ureq::Body>, ureq::Error>,
    service: &str,
) -> Result<(u16, String)> {
    let mut response = response.with_context(|| format!("couldn't reach {service}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .with_context(|| format!("couldn't read {service}'s answer"))?;
    Ok((response.status().as_u16(), body))
}

/// The document Google Play answered with, given its status, or why it
/// refused.
fn read<T: DeserializeOwned>(status: u16, body: &str) -> Result<T> {
    if !is_success(status) {
        bail!(refusal(status, body));
    }
    serde_json::from_str(body).context(UNDOCUMENTED)
}

/// Why Google Play refused a request, given the status it answered with,
/// and the document that says why, if it is one.
fn refusal(status: u16, body: &str) -> String {
    let said = said(body);
    // The service account is refused the same way whichever is missing.
    if status == 403 {
        format!(
            "Google Play refused the service account (403): the Play Console has to invite it, under Users and permissions, with permission to release the app, and the Google Play Android Developer API has to be on in its Google Cloud project{said}"
        )
    } else {
        format!("Google Play refused it ({status}){said}")
    }
}

/// What Google said, if `body` is a document of why it refused, as the
/// end of a sentence.
fn said(body: &str) -> String {
    let reason = match serde_json::from_str::<Errors>(body) {
        Ok(Errors::Api { error }) => error.message,
        Ok(Errors::OAuth {
            error,
            error_description,
        }) => format!("{error}: {error_description}")
            .trim_end_matches([':', ' '])
            .to_owned(),
        Err(_) => String::new(),
    };
    if reason.is_empty() {
        String::new()
    } else {
        format!(". It said: {reason}")
    }
}

/// `text` as one segment of a URL's path, percent-encoded: a track of the
/// app's own may have any name.
fn segment(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufReader, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use rsa::RsaPublicKey;
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::sha2::Sha256;
    use rsa::signature::Verifier;
    use serde_json::{Value, json};

    use super::{Client, ServiceAccount, segment};
    use crate::http::tests::{Request, read_request, serve};

    /// The test identity's private key, in PEM: an RSA key in PKCS #8, as a
    /// service account's is.
    fn private_key() -> String {
        let pem = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/identity.pem"),
        )
        .unwrap();
        let end = "-----END PRIVATE KEY-----";
        pem[..pem.find(end).unwrap() + end.len()].to_owned()
    }

    /// A service account's key, as Google Cloud gives one, with the test
    /// identity's private key, asking `token_uri` for access tokens. It's
    /// made as tests run, rather than kept as a fixture, which GitHub's
    /// push protection would take for a real one.
    fn key_file(token_uri: &str) -> Value {
        json!({
            "type": "service_account",
            "project_id": "omnibuild-test",
            "private_key_id": "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c",
            "private_key": private_key(),
            "client_email": "upload@omnibuild-test.iam.gserviceaccount.com",
            "client_id": "100000000000000000001",
            "auth_uri": "https://accounts.google.com/o/oauth2/auth",
            "token_uri": token_uri,
            "universe_domain": "googleapis.com",
        })
    }

    /// The key `key_file` gives, written to `dir`.
    fn write_key(dir: &Path, key: &Value) -> PathBuf {
        let path = dir.join("service-account.json");
        fs::write(&path, key.to_string()).unwrap();
        path
    }

    /// The test service account, which asks for access tokens at `url`.
    fn account(url: &str) -> ServiceAccount {
        let dir = tempfile::tempdir().unwrap();
        ServiceAccount::read(&write_key(dir.path(), &key_file(&format!("{url}/token")))).unwrap()
    }

    /// The token's header and claims, decoded, what it signs, and its
    /// signature.
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

    /// A form's fields, as `send_form` encodes them.
    fn form(bytes: &[u8]) -> Vec<(String, String)> {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        text.split('&')
            .map(|field| {
                let (name, value) = field.split_once('=').unwrap();
                (name.to_owned(), value.replace("%3A", ":"))
            })
            .collect()
    }

    /// A made-up Google, which issues access tokens and keeps an app, as
    /// its OAuth 2.0 server and Google Play's API answer.
    #[derive(Default)]
    struct Google {
        /// The requests it refuses, by their method and the end of their
        /// path, with the status and body it answers them with.
        refuses: Option<(&'static str, &'static str, u16, String)>,
    }

    impl Google {
        fn answer(&self, request: &Request) -> (u16, String) {
            if let Some((method, path, status, body)) = &self.refuses
                && request.method == *method
                && request.path.ends_with(path)
            {
                return (*status, body.clone());
            }
            let app = "/androidpublisher/v3/applications/dev.example.app";
            let answer = match (request.method.as_str(), request.path.as_str()) {
                ("POST", "/token") => json!({
                    "access_token": "ya29.access",
                    "expires_in": 3599,
                    "token_type": "Bearer",
                }),
                ("POST", path) if path == format!("{app}/edits") => {
                    json!({ "id": "E1", "expiryTimeSeconds": "1761403600" })
                }
                ("POST", path) if path == format!("/upload{app}/edits/E1/bundles") => {
                    json!({ "versionCode": 42, "sha1": "", "sha256": "" })
                }
                ("PUT", path) if path.starts_with(&format!("{app}/edits/E1/tracks/")) => {
                    request.body.clone()
                }
                ("POST", path) if path == format!("{app}/edits/E1:commit") => {
                    json!({ "id": "E1" })
                }
                _ => return (404, json!({ "error": { "code": 404, "message": "Package not found: dev.other.app." } }).to_string()),
            };
            (200, answer.to_string())
        }
    }

    /// The made-up Google, with what it refuses, and its URL and the
    /// requests it received.
    fn google(google: Google) -> (String, Arc<Mutex<Vec<Request>>>) {
        serve(move |request, _| google.answer(request))
    }

    fn api_error(code: u16, message: &str) -> String {
        json!({ "error": { "code": code, "message": message, "status": "FAILED_PRECONDITION" } })
            .to_string()
    }

    #[test]
    fn signs_a_token_asking_for_access_to_google_play() {
        let account = account("https://oauth2.example.com");

        let (header, claims, message, signature) = token_parts(&account.assertion(1_761_400_000));

        assert_eq!(
            header,
            json!({ "alg": "RS256", "kid": "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c", "typ": "JWT" })
        );
        assert_eq!(
            claims,
            json!({
                "iss": "upload@omnibuild-test.iam.gserviceaccount.com",
                "scope": "https://www.googleapis.com/auth/androidpublisher",
                "aud": "https://oauth2.example.com/token",
                "iat": 1_761_400_000,
                "exp": 1_761_403_600,
            })
        );
        let key = VerifyingKey::<Sha256>::new(RsaPublicKey::from(account.key.as_ref()));
        assert!(
            key.verify(
                message.as_bytes(),
                &Signature::try_from(&signature[..]).unwrap()
            )
            .is_ok()
        );
    }

    #[test]
    fn reads_the_key_google_cloud_gives() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_file("https://oauth2.googleapis.com/token");

        let account = ServiceAccount::read(&write_key(dir.path(), &key)).unwrap();

        assert_eq!(account.token_uri, "https://oauth2.googleapis.com/token");
        assert_eq!(
            account.email,
            "upload@omnibuild-test.iam.gserviceaccount.com"
        );
        assert_eq!(account.key_id, "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c");
    }

    #[test]
    fn says_what_it_cant_read_of_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, change: &dyn Fn(&mut Value)| {
            let mut key = key_file("https://oauth2.googleapis.com/token");
            change(&mut key);
            let path = dir.path().join(name);
            fs::write(&path, key.to_string()).unwrap();
            path
        };
        let not_json = dir.path().join("not.json");
        fs::write(&not_json, "{").unwrap();
        // A user's own credentials, as the gcloud CLI keeps them.
        let user = write("user.json", &|key| key["type"] = json!("authorized_user"));
        let no_email = write("no-email.json", &|key| {
            key.as_object_mut().unwrap().remove("client_email");
        });
        let not_rsa = write("not-rsa.json", &|key| key["private_key"] = json!("a key"));
        let error = |path: &Path| format!("{:#}", ServiceAccount::read(path).err().unwrap());

        assert!(
            error(&dir.path().join("missing.json"))
                .starts_with("couldn't read the service account's key ")
        );
        for path in [not_json, user, no_email, not_rsa] {
            assert_eq!(
                error(&path),
                format!(
                    "{} isn't a Google Cloud service account's key: the JSON file Google Cloud creates for one, under its Keys",
                    path.display()
                )
            );
        }
    }

    #[test]
    fn signs_in_and_releases_an_app_bundle_to_a_track() {
        let (url, requests) = google(Google::default());

        let client = Client::sign_in(&account(&url), &format!("{url}/")).unwrap();
        let version_code = client
            .release("dev.example.app", b"an app bundle", "internal", false)
            .unwrap();

        assert_eq!(version_code, 42);
        let requests = requests.lock().unwrap();
        let summary: Vec<_> = requests
            .iter()
            .map(|request| format!("{} {}", request.method, request.path))
            .collect();
        let app = "/androidpublisher/v3/applications/dev.example.app";
        assert_eq!(
            summary,
            [
                "POST /token".to_owned(),
                format!("POST {app}/edits"),
                format!("POST /upload{app}/edits/E1/bundles"),
                format!("PUT {app}/edits/E1/tracks/internal"),
                format!("POST {app}/edits/E1:commit"),
            ]
        );
        // The service account asks for the token with one its key signs.
        let fields = form(&requests[0].bytes);
        assert_eq!(fields[0].0, "grant_type");
        assert_eq!(fields[0].1, "urn:ietf:params:oauth:grant-type:jwt-bearer");
        assert_eq!(fields[1].0, "assertion");
        let (_, claims, ..) = token_parts(&fields[1].1);
        assert_eq!(claims["aud"], format!("{url}/token"));
        assert_eq!(
            requests[0].headers["content-type"],
            "application/x-www-form-urlencoded"
        );
        // Then each request to Google Play carries the token Google issued.
        for request in &requests[1..] {
            assert_eq!(request.authorization, "Bearer ya29.access");
        }
        assert_eq!(requests[1].body, json!({}));
        assert_eq!(requests[2].query["uploadType"], "media");
        assert_eq!(
            requests[2].headers["content-type"],
            "application/octet-stream"
        );
        assert_eq!(requests[2].bytes, b"an app bundle");
        assert_eq!(
            requests[3].body,
            json!({
                "track": "internal",
                "releases": [{ "versionCodes": ["42"], "status": "completed" }],
            })
        );
        assert_eq!(requests[4].bytes, b"");
        assert_eq!(requests[4].headers["content-length"], "0");
    }

    #[test]
    fn releases_a_draft_to_a_track_of_the_apps_own() {
        let (url, requests) = google(Google::default());

        Client::sign_in(&account(&url), &url)
            .unwrap()
            .release("dev.example.app", b"an app bundle", "Team QA/1", true)
            .unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[3].path,
            "/androidpublisher/v3/applications/dev.example.app/edits/E1/tracks/Team QA/1"
        );
        assert_eq!(
            requests[3].body,
            json!({
                "track": "Team QA/1",
                "releases": [{ "versionCodes": ["42"], "status": "draft" }],
            })
        );
    }

    #[test]
    fn encodes_a_track_as_one_segment_of_a_path() {
        assert_eq!(segment("wear:beta"), "wear%3Abeta");
        assert_eq!(segment("Team QA/1"), "Team%20QA%2F1");
        assert_eq!(segment("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn says_why_google_didnt_sign_it_in() {
        let refused = |status, body: &str| {
            let (url, _) = google(Google {
                refuses: Some(("POST", "/token", status, body.to_owned())),
            });
            format!("{:#}", Client::sign_in(&account(&url), &url).err().unwrap())
        };
        let refusal = "Google didn't accept the service account's key: check that neither the key nor the account has been deleted or disabled in Google Cloud";

        assert_eq!(
            refused(
                400,
                &json!({ "error": "invalid_grant", "error_description": "Invalid JWT Signature." })
                    .to_string()
            ),
            format!("{refusal}. It said: invalid_grant: Invalid JWT Signature.")
        );
        assert_eq!(
            refused(401, &json!({ "error": "invalid_client" }).to_string()),
            format!("{refusal}. It said: invalid_client")
        );
        assert_eq!(refused(500, "<html>Error</html>"), refusal);
        assert_eq!(
            refused(200, "{}"),
            "Google answered with what its OAuth 2.0 server doesn't document: missing field `access_token` at line 1 column 2"
        );
    }

    #[test]
    fn says_when_google_play_has_no_app_by_the_id() {
        let (url, requests) = google(Google::default());

        let error = Client::sign_in(&account(&url), &url)
            .unwrap()
            .release("dev.other.app", b"an app bundle", "internal", false)
            .err()
            .unwrap();

        assert_eq!(
            format!("{error:#}"),
            "Google Play has no app dev.other.app that the service account can release: create the app in the Play Console, and upload its first app bundle there, by hand, which gives it its ID. Then invite the service account under Users and permissions, with permission to release it"
        );
        // Nothing uploaded.
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn says_why_google_play_refused() {
        let cases = [
            (
                (
                    "POST",
                    "/edits",
                    403,
                    api_error(403, "The caller does not have permission"),
                ),
                "couldn't open an edit of dev.example.app on Google Play: Google Play refused the service account (403): the Play Console has to invite it, under Users and permissions, with permission to release the app, and the Google Play Android Developer API has to be on in its Google Cloud project. It said: The caller does not have permission",
            ),
            (
                ("POST", "/edits", 200, "{}".to_owned()),
                "couldn't open an edit of dev.example.app on Google Play: Google Play answered with what its API doesn't document: missing field `id` at line 1 column 2",
            ),
            (
                (
                    "POST",
                    "/bundles",
                    400,
                    api_error(
                        400,
                        "You uploaded an APK or Android App Bundle that was signed in debug mode. You need to sign your APK or Android App Bundle in release mode.",
                    ),
                ),
                "couldn't upload the app bundle to Google Play: Google Play refused it (400). It said: You uploaded an APK or Android App Bundle that was signed in debug mode. You need to sign your APK or Android App Bundle in release mode.",
            ),
            (
                (
                    "PUT",
                    "/tracks/internal",
                    400,
                    api_error(
                        400,
                        "Only releases with status draft may be created on draft app.",
                    ),
                ),
                "couldn't release it to the internal track: Google Play refused it (400). It said: Only releases with status draft may be created on draft app.",
            ),
            (
                (
                    "POST",
                    ":commit",
                    502,
                    "<html>Bad gateway</html>".to_owned(),
                ),
                "couldn't commit the edit, which publishes the release: Google Play refused it (502)",
            ),
            (
                (
                    "POST",
                    ":commit",
                    409,
                    json!({ "error": { "code": 409 } }).to_string(),
                ),
                "couldn't commit the edit, which publishes the release: Google Play refused it (409)",
            ),
        ];
        for (refuses, message) in cases {
            let (url, _) = google(Google {
                refuses: Some(refuses),
            });

            let error = Client::sign_in(&account(&url), &url)
                .unwrap()
                .release("dev.example.app", b"an app bundle", "internal", false)
                .err()
                .unwrap();

            assert_eq!(format!("{error:#}"), message);
        }
    }

    #[test]
    fn says_when_it_cant_reach_google() {
        // A port nothing listens on any more.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let nothing_there = format!("http://127.0.0.1:{port}");
        let (url, _) = google(Google::default());

        let signing_in = Client::sign_in(&account(&nothing_there), &url)
            .err()
            .unwrap();
        let releasing = Client::sign_in(&account(&url), &nothing_there)
            .unwrap()
            .release("dev.example.app", b"an app bundle", "internal", false)
            .err()
            .unwrap();

        assert!(format!("{signing_in:#}").starts_with("couldn't reach Google: "));
        assert!(format!("{releasing:#}").starts_with("couldn't reach Google Play: "));
    }

    #[test]
    fn says_when_google_play_drops_out_partway() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // It answers signing in and opening the edit, then stops listening.
        let google = Google::default();
        thread::spawn(move || {
            for _ in 0..2 {
                let mut reader = BufReader::new(listener.accept().unwrap().0);
                let (status, body) = google.answer(&read_request(&mut reader));
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });

        // The upload finds no one listening, or a connection the listener
        // left behind, which is reset as it goes.
        let error = Client::sign_in(&account(&url), &url)
            .unwrap()
            .release("dev.example.app", b"an app bundle", "internal", false)
            .err()
            .unwrap();

        assert!(format!("{error:#}").starts_with(
            "couldn't upload the app bundle to Google Play: couldn't reach Google Play: "
        ));
    }

    #[test]
    fn says_when_it_cant_read_googles_answer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // An answer that ends before the body it promises.
        thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            read_request(&mut BufReader::new(&mut stream));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"access")
                .unwrap();
        });

        let error = Client::sign_in(&account(&url), &url).err().unwrap();

        assert!(format!("{error:#}").starts_with("couldn't read Google's answer: "));
    }
}
