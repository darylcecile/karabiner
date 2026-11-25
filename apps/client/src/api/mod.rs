//! Loopback HTTP client for the service `/v1` API. The only module that performs I/O.

use std::sync::Arc;

use futures::AsyncReadExt as _;
use gpui_kit::http_client::{AsyncBody, HttpClient, Method, Request, http};
use serde::{Serialize, de::DeserializeOwned};

pub use protocol::ApiErrorBody;
use protocol::{ApiErrorEnvelope, DEFAULT_SERVICE_URL as DEFAULT_BASE_URL, Deleted, Reply};
const BASE_URL_ENV: &str = "KARABINER_SERVICE_URL";

#[derive(Debug, Clone)]
pub enum ApiError {
    /// Connection refused, timeout, or another transport failure.
    Unavailable(String),
    /// The service answered with a non-2xx status.
    Status { status: u16, body: Option<ApiErrorBody> },
    /// The response body did not match the expected type.
    Decode(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(f, "Service unavailable: {message}"),
            Self::Status { status, body: Some(body) } => {
                write!(f, "{} ({status}): {}", body.code, body.message)
            }
            Self::Status { status, body: None } => write!(f, "Request failed with status {status}"),
            Self::Decode(message) => write!(f, "Unexpected response: {message}"),
        }
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

/// Cheap to clone; share one instance through [`crate::state::AppState`].
#[derive(Clone)]
pub struct ApiClient {
    base_url: Arc<str>,
    http: Arc<dyn HttpClient>,
}

impl ApiClient {
    pub fn from_env() -> anyhow::Result<Self> {
        let base_url = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.into());
        // No read timeout: reqwest's body timeout needs a Tokio reactor on the polling
        // thread, and bodies are read on GPUI's executors (it panics otherwise).
        let http = reqwest_client::ReqwestClient::user_agent(concat!("karabiner-client/", env!("CARGO_PKG_VERSION")))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').into(),
            http: Arc::new(http),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let bytes = self.request(Method::GET, path, None).await?;
        decode(&bytes)
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> ApiResult<T> {
        self.send(Method::POST, path, body).await
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> ApiResult<T> {
        self.send(Method::PUT, path, body).await
    }

    pub async fn patch<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> ApiResult<T> {
        self.send(Method::PATCH, path, body).await
    }

    pub async fn delete(&self, path: &str) -> ApiResult<()> {
        let bytes = self.request(Method::DELETE, path, None).await?;
        decode::<Deleted>(&bytes).map(|_| ())
    }

    /// Opens a server-sent events stream and returns its body unread, for incremental reads.
    pub async fn stream(&self, path: &str) -> ApiResult<AsyncBody> {
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("{}{path}", self.base_url))
            .header(http::header::ACCEPT, "text/event-stream")
            .body(AsyncBody::empty())
            .map_err(|error| ApiError::Unavailable(error.to_string()))?;
        let response = self.http.send(request).await.map_err(|error| ApiError::Unavailable(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Status { status: status.as_u16(), body: None });
        }
        Ok(response.into_body())
    }

    async fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> ApiResult<T> {
        let json = serde_json::to_vec(body).map_err(|error| ApiError::Decode(error.to_string()))?;
        let bytes = self.request(method, path, Some(json)).await?;
        decode(&bytes)
    }

    async fn request(&self, method: Method, path: &str, body: Option<Vec<u8>>) -> ApiResult<Vec<u8>> {
        let builder = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base_url))
            .header(http::header::ACCEPT, "application/json");
        let request = match body {
            Some(json) => builder
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(AsyncBody::from(json)),
            None => builder.body(AsyncBody::empty()),
        }
        .map_err(|error| ApiError::Unavailable(error.to_string()))?;

        let mut response = self
            .http
            .send(request)
            .await
            .map_err(|error| ApiError::Unavailable(error.to_string()))?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| ApiError::Unavailable(error.to_string()))?;

        let status = response.status();
        if status.is_success() {
            return Ok(bytes);
        }
        Err(ApiError::Status {
            status: status.as_u16(),
            body: serde_json::from_slice::<ApiErrorEnvelope>(&bytes).ok().map(|envelope| envelope.error),
        })
    }
}

impl ApiError {
    /// 409: the record changed since it was read (stale `revision`).
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Status { status: 409, .. })
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Status { status: 404, .. })
    }

    /// 501: reserved execution/integration operation the service does not implement.
    pub fn is_not_implemented(&self) -> bool {
        matches!(self, Self::Status { status: 501, .. })
    }
}

/// Successful bodies are `{"data": T}`.
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> ApiResult<T> {
    serde_json::from_slice::<Reply<T>>(bytes)
        .map(|reply| reply.data)
        .map_err(|error| ApiError::Decode(error.to_string()))
}

/// Percent-encodes a query value (ids are opaque strings).
pub fn query_value(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
