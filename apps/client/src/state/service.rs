//! Service connection status and the shared data-change signal.
//!
//! Refresh semantics (no push channel in API v1):
//! - `data_changed` bumps `data_version`; every view observing `ServiceState` refetches
//!   when the version changes. Call it after any successful mutation so other tabs and
//!   the sidebar converge.
//! - `refresh` (⌘R, Retry, window re-activation) re-checks `/v1/meta` and bumps the
//!   version, which is how CLI edits made outside the app become visible.
//! - Views must not overwrite dirty local edits on refetch; see the frozen interface guide.

use gpui_kit::*;
use protocol::ServiceMeta;

use crate::api::{ApiClient, ApiError};

const META_PATH: &str = "/v1/meta";

#[derive(Debug, Clone)]
pub enum ServiceStatus {
    Connecting,
    Ready,
    Unavailable(String),
    Incompatible(ServiceMeta),
}

impl ServiceStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn message(&self) -> Option<String> {
        match self {
            Self::Connecting | Self::Ready => None,
            Self::Unavailable(reason) => Some(reason.clone()),
            Self::Incompatible(meta) => Some(format!(
                "Service {} speaks API v{}; this client needs v{}.",
                meta.service_version,
                meta.api_version,
                protocol::API_VERSION
            )),
        }
    }
}

pub struct ServiceState {
    api: ApiClient,
    status: ServiceStatus,
    data_version: u64,
    _check: Option<Task<()>>,
}

impl ServiceState {
    pub fn new(api: ApiClient, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            api,
            status: ServiceStatus::Connecting,
            data_version: 0,
            _check: None,
        };
        this.refresh(cx);
        this
    }

    pub fn status(&self) -> &ServiceStatus {
        &self.status
    }

    pub fn data_version(&self) -> u64 {
        self.data_version
    }

    /// Signal that stored data changed; observers refetch.
    pub fn data_changed(&mut self, cx: &mut Context<Self>) {
        self.data_version += 1;
        cx.notify();
    }

    /// Re-checks the service and, once ready, signals a data refresh.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.status.is_ready() {
            self.status = ServiceStatus::Connecting;
            cx.notify();
        }
        let api = self.api.clone();
        self._check = Some(cx.spawn(async move |this, cx| {
            let status = match api.get::<ServiceMeta>(META_PATH).await {
                Ok(meta) if meta.is_compatible() => ServiceStatus::Ready,
                Ok(meta) => ServiceStatus::Incompatible(meta),
                Err(ApiError::Unavailable(_)) => ServiceStatus::Unavailable(format!(
                    "Can't reach the Karabiner service at {}. Start it with `bun run service`.",
                    api.base_url()
                )),
                Err(error) => ServiceStatus::Unavailable(error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.status = status;
                this.data_version += 1;
                cx.notify();
            })
            .ok();
        }));
    }
}
