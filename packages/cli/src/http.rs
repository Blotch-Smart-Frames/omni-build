//! HTTPS for the stores' APIs, App Store Connect's and Google Play's: TLS
//! through rustls with the same pure-Rust provider wireless debugging has,
//! rather than ureq's (ring, with C and assembly), and time limits, so that
//! a network that drops everything fails the command rather than hanging
//! it.

use std::sync::Arc;
use std::time::Duration;

use ureq::Agent;
use ureq::RequestBuilder;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// How long a request may take before OmniBuild gives up on it.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// How long a store may take to answer an upload once it has it all: it
/// checks the file before it answers, and Google asks for two minutes.
const CHECKING: Duration = Duration::from_mins(5);

/// An agent for a store's API.
pub fn agent() -> Agent {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::WebPki)
        .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
        .build();
    Agent::config_builder()
        .tls_config(tls)
        // An error's response says why, which ureq would drop.
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("omni/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// `request`, which uploads a file, with time limits for one: an app takes
/// as long to send as the network is slow, so rather than a time for the
/// whole request, each write has to go out within `TIMEOUT`, and the answer
/// come within `CHECKING` of the last.
pub fn upload<B>(request: RequestBuilder<B>) -> RequestBuilder<B> {
    request
        .config()
        .timeout_global(None)
        .timeout_connect(Some(TIMEOUT))
        .timeout_per_write(Some(TIMEOUT))
        .timeout_recv_response(Some(CHECKING))
        .timeout_recv_body(Some(TIMEOUT))
        .build()
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use serde_json::Value;

    /// A request the test server received.
    #[derive(Clone, Debug)]
    pub struct Request {
        pub method: String,
        /// Without the query.
        pub path: String,
        pub query: BTreeMap<String, String>,
        /// By their names in lowercase.
        pub headers: BTreeMap<String, String>,
        pub authorization: String,
        /// The body as JSON, or null if it isn't.
        pub body: Value,
        /// The body as it came.
        pub bytes: Vec<u8>,
    }

    /// Serves HTTP on a port of its own, answering each request with the
    /// status and body `answer` gives it, given the request and the
    /// server's own URL. Returns its URL, and the requests it received, in
    /// order.
    pub fn serve(
        answer: impl Fn(&Request, &str) -> (u16, String) + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<Request>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&requests);
        let own = url.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut reader = BufReader::new(stream.unwrap());
                let request = read_request(&mut reader);
                received.lock().unwrap().push(request.clone());
                let (status, body) = answer(&request, &own);
                // One request a connection, which ureq then doesn't reuse.
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (url, requests)
    }

    pub fn read_request(reader: &mut impl BufRead) -> Request {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut parts = line.split(' ');
        let method = parts.next().unwrap().to_owned();
        let target = parts.next().unwrap();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let query = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (decode(key), decode(value)))
            .collect();
        let mut headers = BTreeMap::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            let Some((name, value)) = header.trim_end().split_once(": ") else {
                break;
            };
            headers.insert(name.to_lowercase(), value.to_owned());
        }
        let length = headers
            .get("content-length")
            .map_or(0, |length| length.parse().unwrap());
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).unwrap();
        Request {
            method,
            path: decode(path),
            query,
            authorization: headers.get("authorization").cloned().unwrap_or_default(),
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            bytes,
        }
    }

    /// A path's or a query's text, percent-decoded.
    fn decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut decoded = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%' {
                decoded.push(u8::from_str_radix(&text[at + 1..at + 3], 16).unwrap());
                at += 3;
            } else {
                decoded.push(bytes[at]);
                at += 1;
            }
        }
        String::from_utf8(decoded).unwrap()
    }
}
