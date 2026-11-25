//! Service connection status and the shared data-change signal.
//!
//! Refresh semantics:
//! - `data_changed` bumps `data_version`; every view observing `ServiceState` refetches
//!   when the version changes. Call it after any successful mutation so other tabs and
//!   the sidebar converge.
//! - When the service advertises `events`, `GET /v1/events` (SSE) invalidates too: runtime
//!   writes, CLI edits and other clients bump the version. Bursts (live run output) are
//!   coalesced into one bump per [`COALESCE`]; `ready` (each (re)connect) bumps once, since
//!   events are not replayed. The stream reconnects with backoff while the service is up.
//! - `refresh` (⌘R, Retry, window re-activation) re-checks `/v1/meta` and bumps the version.
//! - Views must not overwrite dirty local edits on refetch; see the frozen interface guide.

use std::time::Duration;

use futures::{AsyncBufReadExt as _, StreamExt as _, channel::mpsc, io::BufReader};
use gpui_kit::*;
use protocol::ServiceMeta;

use crate::api::{ApiClient, ApiError};

const META_PATH: &str = "/v1/meta";
const EVENTS_PATH: &str = "/v1/events";
/// Longest wait between an invalidation and the refetch it triggers.
const COALESCE: Duration = Duration::from_millis(400);
const RECONNECT_MIN: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(30);

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
    /// What the connected service supports; empty (all off) until `/v1/meta` answers.
    capabilities: protocol::Capabilities,
    _check: Option<Task<()>>,
    /// SSE reader and its coalescer; dropped (cancelled) with the state or on re-check.
    _events: Option<(Task<()>, Task<()>)>,
}

impl ServiceState {
    pub fn new(api: ApiClient, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            api,
            status: ServiceStatus::Connecting,
            data_version: 0,
            capabilities: Default::default(),
            _check: None,
            _events: None,
        };
        this.refresh(cx);
        this
    }

    pub fn status(&self) -> &ServiceStatus {
        &self.status
    }

    pub fn capabilities(&self) -> protocol::Capabilities {
        self.capabilities
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
            let mut capabilities = protocol::Capabilities::default();
            let status = match api.get::<ServiceMeta>(META_PATH).await {
                Ok(meta) if meta.is_compatible() => { capabilities = meta.capabilities; ServiceStatus::Ready }
                Ok(meta) => ServiceStatus::Incompatible(meta),
                Err(ApiError::Unavailable(_)) => ServiceStatus::Unavailable(format!(
                    "Can't reach the Karabiner service at {}. Start it with `bun run service`.",
                    api.base_url()
                )),
                Err(error) => ServiceStatus::Unavailable(error.to_string()),
            };
            let events = capabilities.events;
            this.update(cx, |this, cx| {
                this.status = status;
                this.capabilities = capabilities;
                this.data_version += 1;
                if events && this._events.is_none() { this.watch_events(cx) }
                if !events { this._events = None }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Reads `/v1/events` and bumps the version once per burst of invalidations.
    fn watch_events(&mut self, cx: &mut Context<Self>) {
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let api = self.api.clone();
        let executor = cx.background_executor().clone();
        let reader = cx.background_spawn(async move {
            let mut backoff = RECONNECT_MIN;
            loop {
                match api.stream(EVENTS_PATH).await {
                    // An older service without the stream: stop; ⌘R still refreshes.
                    Err(error) if error.is_not_found() || error.is_not_implemented() => return,
                    Err(_) => {}
                    Ok(body) => {
                        backoff = RECONNECT_MIN;
                        // Line framing survives chunk splits; payloads are ignored.
                        let mut lines = BufReader::new(body);
                        let mut line = String::new();
                        while lines.read_line(&mut line).await.is_ok_and(|read| read > 0) {
                            if is_invalidation(&line) && sender.unbounded_send(()).is_err() { return }
                            line.clear();
                        }
                    }
                }
                executor.timer(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        });
        let coalescer = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                cx.background_executor().timer(COALESCE).await;
                while receiver.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.data_changed(cx)).is_err() { return }
            }
        });
        self._events = Some((reader, coalescer));
    }
}

/// `event: ready` (each connect) and `event: change` both mean "refetch".
fn is_invalidation(line: &str) -> bool {
    matches!(line.strip_prefix("event:").map(str::trim), Some("ready" | "change"))
}

#[cfg(test)]
mod tests {
    use futures::{AsyncBufReadExt as _, TryStreamExt as _, executor::block_on, io::BufReader, stream};

    use super::is_invalidation;

    /// An `event:` line split across chunks still counts once.
    #[test]
    fn events_split_across_chunks_are_read_as_lines() {
        let chunks = ["data: {}\n\neve", "nt: cha", "nge\ndata: {\"path\":\"/v1/runs\"}\n\nevent: ready\n"];
        let body = stream::iter(chunks.map(|chunk| Ok::<_, std::io::Error>(chunk.as_bytes()))).into_async_read();
        let lines: Vec<String> = block_on(async {
            let (mut reader, mut lines, mut line) = (BufReader::new(body), Vec::new(), String::new());
            while reader.read_line(&mut line).await.unwrap() > 0 { lines.push(std::mem::take(&mut line)) }
            lines
        });
        assert_eq!(lines.iter().filter(|line| is_invalidation(line)).count(), 2);
    }
}
