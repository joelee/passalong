//! The HTTP client: TLS as [`crate::tls`] sets it up, the API key as a
//! bearer token, an `X-Request-Id` on every request, and the retries of
//! [`crate::error`].

use std::time::Duration;

use passalong_core::api_key::ApiKey;
use passalong_core::config::HttpsConfig;
use passalong_core::random::{RandomSource, StdRandom};
use reqwest::header::RETRY_AFTER;
use serde::de::DeserializeOwned;

use crate::api::{Viewer, Workspace};
use crate::error::{self, HttpsError, Next, Problem};
use crate::tls;

/// How long a connection, TLS included, may take to open.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the server may stay silent in the middle of an answer.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// A client of one server, with one API key.
pub struct Client {
    http: reqwest::Client,
    base: String,
    key: ApiKey,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for the server `config` names, authenticating with `key`.
    ///
    /// # Errors
    ///
    /// [`HttpsError::Transport`] when TLS cannot be set up.
    pub fn new(config: &HttpsConfig, key: ApiKey) -> Result<Self, HttpsError> {
        let setup = |reason: String| HttpsError::Transport {
            url: config.url.clone(),
            reason,
            retryable: false,
        };
        let tls = tls::client_config(config.tls_pin)
            .map_err(|err| setup(format!("setting up TLS: {err}")))?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(format!("passalong/{}", passalong_core::VERSION))
            .build()
            .map_err(|err| setup(chain(&err)))?;
        Ok(Self {
            http,
            base: config.url.clone(),
            key,
        })
    }

    /// The API key in use.
    pub fn key(&self) -> &ApiKey {
        &self.key
    }

    /// The URL of the API route `path`, such as `/items`.
    pub fn url(&self, path: &str) -> String {
        format!("{}/v1{path}", self.base)
    }

    /// Sends the request `build` makes, with the API key, repeating it as
    /// [`error::next`] says; `build` is called for every attempt. Returns
    /// the answer when it is a success.
    ///
    /// # Errors
    ///
    /// The last attempt's [`HttpsError`].
    pub async fn send<F>(&self, what: &str, build: F) -> Result<reqwest::Response, HttpsError>
    where
        F: Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let request_id = format!("{:016x}", StdRandom::new().next_u64());
            tracing::debug!(request = %request_id, what, attempt = attempts, "sending");
            let result = build(&self.http)
                .bearer_auth(self.key.expose())
                .header("X-Request-Id", &request_id)
                .send()
                .await;
            let err = match result {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => refused(response).await,
                Err(err) => transport(&self.base, &err),
            };
            match error::next(&err, attempts) {
                Next::Stop => return Err(err),
                Next::RetryAfter(wait) => {
                    tracing::info!(request = %request_id, what, error = %err, wait_ms = wait.as_millis(), "retrying");
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }

    /// `GET` of a JSON document.
    ///
    /// # Errors
    ///
    /// As [`Client::send`], and [`HttpsError::Protocol`] for a document
    /// that does not read.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpsError> {
        let url = self.url(path);
        let response = self.send(path, |http| http.get(&url)).await?;
        json(response).await
    }

    /// `getViewer`: the key in use and the server.
    ///
    /// # Errors
    ///
    /// As [`Client::get_json`].
    pub async fn viewer(&self) -> Result<Viewer, HttpsError> {
        self.get_json("/viewer").await
    }

    /// `getWorkspace`: the workspace the key opens.
    ///
    /// # Errors
    ///
    /// As [`Client::get_json`].
    pub async fn workspace(&self) -> Result<Workspace, HttpsError> {
        self.get_json("/workspace").await
    }
}

/// Reads a JSON answer.
///
/// # Errors
///
/// [`HttpsError::Protocol`] when it does not read as `T`, and
/// [`HttpsError::Transport`] when the answer breaks off.
pub async fn json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, HttpsError> {
    let url = response.url().to_string();
    let body = response
        .bytes()
        .await
        .map_err(|err| transport(&url, &err))?;
    serde_json::from_slice(&body).map_err(|err| HttpsError::Protocol(format!("{url}: {err}")))
}

/// The error for a refusal: the problem the server sent.
async fn refused(response: reqwest::Response) -> HttpsError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let url = response.url().to_string();
    match response.bytes().await {
        Ok(body) => match serde_json::from_slice::<Problem>(&body) {
            Ok(problem) => HttpsError::Refused {
                problem,
                retry_after,
            },
            Err(_) => HttpsError::Protocol(format!("{url}: HTTP {status} without a problem")),
        },
        Err(err) => transport(&url, &err),
    }
}

/// The error for a request that got no answer. A refused certificate is
/// not retried: it would be refused again.
fn transport(url: &str, err: &reqwest::Error) -> HttpsError {
    let certificate = sources(err).any(refuses_certificate);
    HttpsError::Transport {
        url: url.to_owned(),
        reason: chain(err),
        retryable: !certificate
            && (err.is_connect() || err.is_timeout() || err.is_request() || err.is_body()),
    }
}

/// Whether `err` is rustls refusing the server's certificate. hyper-rustls
/// wraps that in `io::Error`s, whose `source()` skips what they wrap, so
/// they are unwrapped here.
fn refuses_certificate(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(err);
    while let Some(error) = current {
        if error
            .downcast_ref::<rustls::Error>()
            .is_some_and(|tls| matches!(tls, rustls::Error::InvalidCertificate(_)))
        {
            return true;
        }
        current = error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|inner| inner as &(dyn std::error::Error + 'static));
    }
    false
}

/// `err` and the errors it wraps.
fn sources<'a>(
    err: &'a (dyn std::error::Error + 'static),
) -> impl Iterator<Item = &'a (dyn std::error::Error + 'static)> {
    std::iter::successors(Some(err), |err| err.source())
}

/// `err` and every error it wraps, in one line, so a TLS reason such as a
/// pin mismatch is shown.
pub(crate) fn chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = String::new();
    for source in sources(err) {
        let part = source.to_string();
        if !text.contains(&part) {
            if !text.is_empty() {
                text.push_str(": ");
            }
            text.push_str(&part);
        }
    }
    text
}
