//! Uploading an app built for devices to App Store Connect, for TestFlight
//! and the App Store, through its API's build uploads, with the user's API
//! key, rather than Xcode or Transporter: App Store Connect reserves room
//! for the `.ipa`, OmniBuild sends it to Apple's storage in the parts App
//! Store Connect asks for, and commits it, then waits while App Store
//! Connect checks it, which is where it refuses an app it won't take.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::json;

use super::Release;
use crate::app_store_connect::{Client, Resource, UploadOperation};
use crate::report::Event;

const APPS: &str = "/v1/apps";
const BUILD_UPLOADS: &str = "/v1/buildUploads";
const BUILD_UPLOAD_FILES: &str = "/v1/buildUploadFiles";
/// How often OmniBuild asks how App Store Connect's checks go.
pub const POLL: Duration = Duration::from_secs(10);
/// How long it waits for them. They take minutes, and an upload App Store
/// Connect is still checking after this has likely stalled.
pub const PATIENCE: Duration = Duration::from_mins(30);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppAttributes {
    bundle_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileAttributes {
    upload_operations: Vec<UploadOperation>,
}

#[derive(Deserialize)]
struct UploadAttributes {
    /// Absent until App Store Connect has something to say, as it may be
    /// before the file arrives.
    #[serde(default)]
    state: Status,
}

/// Where App Store Connect is with an upload, and what it found.
#[derive(Default, Deserialize)]
struct Status {
    /// `AWAITING_UPLOAD` and `PROCESSING`, then `COMPLETE` or `FAILED`.
    #[serde(default)]
    state: String,
    #[serde(default)]
    errors: Vec<Detail>,
    #[serde(default)]
    warnings: Vec<Detail>,
}

/// Something App Store Connect found in an app, with the number of its
/// kind (`90062`), as Transporter reports them.
#[derive(Deserialize)]
struct Detail {
    code: String,
    description: String,
}

/// Uploads apps to App Store Connect, with `client`, and reports what it
/// waits for, and what App Store Connect warns of, to `report`.
pub struct Uploader<'a> {
    pub client: &'a Client,
    pub report: &'a dyn Fn(Event),
    /// How often it asks how App Store Connect's checks go: `POLL`, but in
    /// tests.
    pub poll: Duration,
    /// How long it waits for them: `PATIENCE`, but in tests.
    pub patience: Duration,
}

impl Uploader<'_> {
    /// Uploads `ipa`, the file named `name`, an app built for devices of
    /// the release `release`, and waits until App Store Connect has taken
    /// it.
    pub fn upload(&self, name: &str, ipa: &[u8], release: &Release) -> Result<()> {
        let app = self.app(&release.id)?;
        let upload: Resource<IgnoredAny> = self
            .client
            .create(
                BUILD_UPLOADS,
                &json!({
                    "type": "buildUploads",
                    "attributes": {
                        "cfBundleShortVersionString": release.version,
                        "cfBundleVersion": release.build,
                        "platform": "IOS",
                    },
                    "relationships": { "app": { "data": { "type": "apps", "id": app } } },
                }),
            )
            .context("couldn't start an upload to App Store Connect")?;
        let file: Resource<FileAttributes> = self
            .client
            .create(
                BUILD_UPLOAD_FILES,
                &json!({
                    "type": "buildUploadFiles",
                    "attributes": {
                        "assetType": "ASSET",
                        "fileName": name,
                        "fileSize": ipa.len(),
                        "uti": "com.apple.ipa",
                    },
                    "relationships": {
                        "buildUpload": { "data": { "type": "buildUploads", "id": upload.id } },
                    },
                }),
            )
            .context("couldn't reserve room for the app in App Store Connect")?;
        for operation in &file.attributes.upload_operations {
            self.client
                .upload(operation, ipa)
                .context("couldn't upload the app to App Store Connect")?;
        }
        self.client
            .update::<IgnoredAny>(
                &format!("{BUILD_UPLOAD_FILES}/{}", file.id),
                &json!({
                    "type": "buildUploadFiles",
                    "id": file.id,
                    "attributes": { "uploaded": true },
                }),
            )
            .context("couldn't commit the upload to App Store Connect")?;
        self.wait(&upload.id, name)
    }

    /// The ID of the account's app with the bundle ID `bundle_id`, which an
    /// upload is of.
    fn app(&self, bundle_id: &str) -> Result<String> {
        let apps = self
            .client
            .list::<AppAttributes>(APPS, &[("filter[bundleId]", bundle_id)])
            .context("couldn't list the account's apps")?;
        apps.into_iter()
            .find(|app| app.attributes.bundle_id == bundle_id)
            .map(|app| app.id)
            .with_context(|| {
                format!(
                    "App Store Connect has no app {bundle_id}: create it there, under Apps, with that bundle ID, which App Store Connect's API can't do"
                )
            })
    }

    /// Waits while App Store Connect checks the upload `id` of the file
    /// named `name`, and says what it found.
    fn wait(&self, id: &str, name: &str) -> Result<()> {
        let started = Instant::now();
        let mut waiting = false;
        loop {
            let upload: Resource<UploadAttributes> = self
                .client
                .get(&format!("{BUILD_UPLOADS}/{id}"))
                .context("couldn't ask App Store Connect how the upload goes")?;
            let state = upload.attributes.state;
            if let "COMPLETE" | "FAILED" = state.state.as_str() {
                for warning in state.warnings {
                    (self.report)(Event::UploadWarning {
                        code: warning.code,
                        description: warning.description,
                    });
                }
                if state.state == "COMPLETE" {
                    return Ok(());
                }
                let reasons: Vec<String> = state
                    .errors
                    .iter()
                    .map(|error| format!("{} ({})", error.description, error.code))
                    .collect();
                if reasons.is_empty() {
                    bail!(
                        "App Store Connect refused {name} without saying why: see the upload in App Store Connect, under TestFlight"
                    );
                }
                bail!("App Store Connect refused {name}: {}", reasons.join(" "));
            }
            if !waiting {
                (self.report)(Event::Processing {
                    app: name.to_owned(),
                });
                waiting = true;
            }
            if started.elapsed() >= self.patience {
                bail!(
                    "App Store Connect was still checking {name} when OmniBuild stopped waiting: see whether it took it in App Store Connect, under TestFlight"
                );
            }
            thread::sleep(self.poll);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::Uploader;
    use crate::app_store_connect::Client;
    use crate::app_store_connect::tests::{api_key, document};
    use crate::http::tests::{Request, serve};
    use crate::ios::Release;
    use crate::report::Event;

    /// The app the tests upload, in bytes.
    const IPA: &[u8] = b"an app for devices";

    fn release() -> Release {
        Release {
            id: "dev.example.app".to_owned(),
            version: "1.2.0".to_owned(),
            build: "42".to_owned(),
        }
    }

    /// A made-up App Store Connect, with an app, `dev.example.app`, whose
    /// storage takes an upload in two parts.
    struct Store {
        /// What it says of the upload each time it's asked, the last again
        /// once the rest are said.
        states: Vec<Value>,
        /// The requests it refuses, by their method and the end of their
        /// path.
        refuses: Option<(&'static str, &'static str)>,
    }

    impl Store {
        fn new(states: &[Value]) -> Self {
            Self {
                states: states.to_vec(),
                refuses: None,
            }
        }

        fn answer(&mut self, request: &Request, url: &str) -> (u16, String) {
            if self.refuses.is_some_and(|(method, path)| {
                request.method == method && request.path.ends_with(path)
            }) {
                let errors = json!([{ "title": "Refused.", "detail": "The test says so." }]);
                return (409, json!({ "errors": errors }).to_string());
            }
            let data = match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/v1/apps") => json!([
                    // A bundle ID the filter matches the start of.
                    { "type": "apps", "id": "A2", "attributes": { "bundleId": "dev.example.app.other" } },
                    { "type": "apps", "id": "A1", "attributes": { "bundleId": "dev.example.app" } },
                ]),
                ("POST", "/v1/buildUploads") => {
                    json!({ "type": "buildUploads", "id": "U1", "attributes": request.body["data"]["attributes"] })
                }
                ("POST", "/v1/buildUploadFiles") => {
                    let operation = |part: u32, offset: usize, length: usize| {
                        json!({
                            "method": "PUT",
                            "url": format!("{url}/storage/{part}"),
                            "offset": offset,
                            "length": length,
                            "requestHeaders": [{ "name": "Content-Type", "value": "application/octet-stream" }],
                            "partNumber": part,
                        })
                    };
                    let operations = [operation(1, 0, 6), operation(2, 6, IPA.len() - 6)];
                    json!({ "type": "buildUploadFiles", "id": "F1", "attributes": { "uploadOperations": operations } })
                }
                ("PUT", path) if path.starts_with("/storage/") => return (200, String::new()),
                ("PATCH", "/v1/buildUploadFiles/F1") => {
                    json!({ "type": "buildUploadFiles", "id": "F1", "attributes": request.body["data"]["attributes"] })
                }
                // The one request left: how the upload goes.
                _ => {
                    let state = if self.states.len() > 1 {
                        self.states.remove(0)
                    } else {
                        self.states[0].clone()
                    };
                    // Null for none at all.
                    let attributes = if state.is_null() {
                        json!({})
                    } else {
                        json!({ "state": state })
                    };
                    json!({ "type": "buildUploads", "id": "U1", "attributes": attributes })
                }
            };
            (200, document(&data))
        }
    }

    /// Uploads the test app to `store`, with `patience`, and returns how it
    /// went, what it reported, and the requests the store received.
    fn upload(store: Store, patience: Duration) -> (Result<()>, Vec<Event>, Vec<Request>) {
        upload_release(store, patience, &release())
    }

    /// Uploads the test app, of `release`, as `upload` does.
    fn upload_release(
        store: Store,
        patience: Duration,
        release: &Release,
    ) -> (Result<()>, Vec<Event>, Vec<Request>) {
        let store = Mutex::new(store);
        let (url, requests) = serve(move |request, url| store.lock().unwrap().answer(request, url));
        let events = Arc::new(Mutex::new(Vec::new()));
        let reported = Arc::clone(&events);
        let report = move |event| reported.lock().unwrap().push(event);
        let result = Uploader {
            client: &Client::new(api_key(), &url),
            report: &report,
            poll: Duration::ZERO,
            patience,
        }
        .upload("app.ipa", IPA, release);
        let events = std::mem::take(&mut *events.lock().unwrap());
        let requests = requests.lock().unwrap().clone();
        (result, events, requests)
    }

    fn state(state: &str) -> Value {
        json!({ "state": state, "errors": [], "warnings": [], "infos": [] })
    }

    #[test]
    fn uploads_an_app_and_waits_for_app_store_connect_to_take_it() {
        let complete = json!({
            "state": "COMPLETE",
            "warnings": [{ "code": "90078", "description": "Missing Push Notification Entitlement." }],
        });
        let store = Store::new(&[
            Value::Null,
            json!({}),
            state("AWAITING_UPLOAD"),
            state("PROCESSING"),
            complete,
        ]);

        let (result, events, requests) = upload(store, Duration::MAX);

        result.unwrap();
        assert_eq!(
            events,
            [
                Event::Processing {
                    app: "app.ipa".to_owned()
                },
                Event::UploadWarning {
                    code: "90078".to_owned(),
                    description: "Missing Push Notification Entitlement.".to_owned(),
                },
            ]
        );
        let summary: Vec<_> = requests
            .iter()
            .map(|request| format!("{} {}", request.method, request.path))
            .collect();
        assert_eq!(
            summary,
            [
                "GET /v1/apps",
                "POST /v1/buildUploads",
                "POST /v1/buildUploadFiles",
                "PUT /storage/1",
                "PUT /storage/2",
                "PATCH /v1/buildUploadFiles/F1",
                "GET /v1/buildUploads/U1",
                "GET /v1/buildUploads/U1",
                "GET /v1/buildUploads/U1",
                "GET /v1/buildUploads/U1",
                "GET /v1/buildUploads/U1",
            ]
        );
        assert_eq!(requests[0].query["filter[bundleId]"], "dev.example.app");
        assert_eq!(
            requests[1].body,
            json!({ "data": {
                "type": "buildUploads",
                "attributes": { "cfBundleShortVersionString": "1.2.0", "cfBundleVersion": "42", "platform": "IOS" },
                "relationships": { "app": { "data": { "type": "apps", "id": "A1" } } },
            } })
        );
        assert_eq!(
            requests[2].body,
            json!({ "data": {
                "type": "buildUploadFiles",
                "attributes": { "assetType": "ASSET", "fileName": "app.ipa", "fileSize": IPA.len(), "uti": "com.apple.ipa" },
                "relationships": { "buildUpload": { "data": { "type": "buildUploads", "id": "U1" } } },
            } })
        );
        // The parts, which make the app, with the headers App Store Connect
        // gave, and without the API's token.
        assert_eq!(
            [&requests[3].bytes[..], &requests[4].bytes[..]].concat(),
            IPA
        );
        for part in &requests[3..5] {
            assert_eq!(part.headers["content-type"], "application/octet-stream");
            assert_eq!(part.authorization, "");
        }
        assert_eq!(
            requests[5].body,
            json!({ "data": { "type": "buildUploadFiles", "id": "F1", "attributes": { "uploaded": true } } })
        );
        for request in requests
            .iter()
            .filter(|request| request.path.starts_with("/v1/"))
        {
            assert!(request.authorization.starts_with("Bearer "));
        }
    }

    #[test]
    fn says_nothing_of_the_wait_when_app_store_connect_has_taken_it_already() {
        let (result, events, _) = upload(Store::new(&[state("COMPLETE")]), Duration::MAX);

        result.unwrap();
        assert_eq!(events, []);
    }

    #[test]
    fn says_why_app_store_connect_refused_the_app() {
        let failed = json!({
            "state": "FAILED",
            "errors": [
                { "code": "90062", "description": "This bundle is invalid. The value for key CFBundleShortVersionString [1.2.0] must contain a higher version than that of the previously approved version [1.3.0]." },
                { "code": "90186", "description": "Invalid Pre-Release Train. The train version '1.2.0' is closed for new build submissions." },
            ],
            "warnings": [{ "code": "90078", "description": "Missing Push Notification Entitlement." }],
        });

        let (result, events, _) = upload(Store::new(&[failed]), Duration::MAX);
        let (silent, ..) = upload(Store::new(&[state("FAILED")]), Duration::MAX);

        assert_eq!(
            format!("{:#}", result.unwrap_err()),
            "App Store Connect refused app.ipa: This bundle is invalid. The value for key CFBundleShortVersionString [1.2.0] must contain a higher version than that of the previously approved version [1.3.0]. (90062) Invalid Pre-Release Train. The train version '1.2.0' is closed for new build submissions. (90186)"
        );
        assert_eq!(
            events,
            [Event::UploadWarning {
                code: "90078".to_owned(),
                description: "Missing Push Notification Entitlement.".to_owned(),
            }]
        );
        assert_eq!(
            format!("{:#}", silent.unwrap_err()),
            "App Store Connect refused app.ipa without saying why: see the upload in App Store Connect, under TestFlight"
        );
    }

    #[test]
    fn stops_waiting_when_app_store_connect_takes_too_long() {
        let (result, events, requests) = upload(Store::new(&[state("PROCESSING")]), Duration::ZERO);

        assert_eq!(
            format!("{:#}", result.unwrap_err()),
            "App Store Connect was still checking app.ipa when OmniBuild stopped waiting: see whether it took it in App Store Connect, under TestFlight"
        );
        assert_eq!(
            events,
            [Event::Processing {
                app: "app.ipa".to_owned()
            }]
        );
        assert_eq!(requests.last().unwrap().path, "/v1/buildUploads/U1");
    }

    #[test]
    fn says_when_app_store_connect_has_no_app_by_the_id() {
        let other = Release {
            id: "dev.example".to_owned(),
            ..release()
        };

        let (result, _, requests) =
            upload_release(Store::new(&[state("COMPLETE")]), Duration::MAX, &other);

        assert_eq!(
            format!("{:#}", result.unwrap_err()),
            "App Store Connect has no app dev.example: create it there, under Apps, with that bundle ID, which App Store Connect's API can't do"
        );
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn says_what_app_store_connect_refused() {
        let refused = "App Store Connect refused it (409). It said: Refused. The test says so.";
        let cases = [
            (("GET", "/v1/apps"), format!("couldn't list the account's apps: {refused}")),
            (
                ("POST", "/v1/buildUploads"),
                format!("couldn't start an upload to App Store Connect: {refused}"),
            ),
            (
                ("POST", "/v1/buildUploadFiles"),
                format!("couldn't reserve room for the app in App Store Connect: {refused}"),
            ),
            (
                ("PUT", "/storage/2"),
                "couldn't upload the app to App Store Connect: Apple's storage refused a part of the app (409)".to_owned(),
            ),
            (
                ("PATCH", "/v1/buildUploadFiles/F1"),
                format!("couldn't commit the upload to App Store Connect: {refused}"),
            ),
            (
                ("GET", "/v1/buildUploads/U1"),
                format!("couldn't ask App Store Connect how the upload goes: {refused}"),
            ),
        ];
        for (refuses, message) in cases {
            let store = Store {
                refuses: Some(refuses),
                ..Store::new(&[state("COMPLETE")])
            };

            let (result, ..) = upload(store, Duration::MAX);

            assert_eq!(format!("{:#}", result.unwrap_err()), message);
        }
    }
}
