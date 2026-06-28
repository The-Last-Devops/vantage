//! Agent reverse-tunnel endpoint + registry (shell/exec Tier 1 — see docs/exec-design.md).
//!
//! An agent with `ALLOW_SHELL=1` holds one outbound WebSocket here (`/pub/tunnel`,
//! authed by its API key). The hub multiplexes byte streams over it: to reach a
//! host's local sshd it calls [`TunnelRegistry::open_stream`], which returns a
//! [`TunnelStream`] (AsyncRead + AsyncWrite) that the SSH bridge (Phase 3) drives.
//!
//! Nothing opens a stream yet — this phase just lets agents register. The endpoint
//! is inert until the SSH bridge + RBAC gate land, and only agents started with the
//! opt-in flag connect at all.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use shared::tunnel::TunnelFrame;
use shared::API_KEY_HEADER;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;

use crate::AppState;

/// How long [`open_stream`](TunnelRegistry::open_stream) waits for the agent to
/// confirm (or reject) the local connection before giving up.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Inbound message routed to a single stream's reader.
enum StreamMsg {
    Data(Vec<u8>),
    Close,
}

/// State the hub keeps for one open stream while awaiting/using it.
struct StreamSlot {
    /// Fires once when the agent answers the `Open` (Ok) or rejects it (Err(msg)).
    open: Option<oneshot::Sender<Result<(), String>>>,
    /// Carries inbound `Data`/`Close` to the [`TunnelStream`] reader.
    data: mpsc::UnboundedSender<StreamMsg>,
}

/// One connected agent tunnel.
struct AgentConn {
    /// Encoded frames queued to the agent's WebSocket writer.
    out: mpsc::UnboundedSender<Vec<u8>>,
    next_stream: AtomicU32,
    streams: Mutex<HashMap<u32, StreamSlot>>,
}

/// Live agent tunnels, keyed by `(api key id, hostname)` — the same identity that
/// uniquely names a system (`systems(key_id, hostname)`).
#[derive(Clone, Default)]
pub struct TunnelRegistry {
    agents: Arc<Mutex<HashMap<(Uuid, String), Arc<AgentConn>>>>,
}

impl TunnelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// True if a live tunnel exists for this system identity.
    pub async fn has(&self, key_id: Uuid, hostname: &str) -> bool {
        self.agents
            .lock()
            .await
            .contains_key(&(key_id, hostname.to_string()))
    }

    /// Open a multiplexed stream to `127.0.0.1:port` on the agent's host and return
    /// it as an async byte stream. `Err` if no tunnel is connected, the agent
    /// rejects the connection, or it times out.
    pub async fn open_stream(
        &self,
        key_id: Uuid,
        hostname: &str,
        port: u16,
    ) -> Result<TunnelStream, String> {
        let agent = {
            let agents = self.agents.lock().await;
            agents
                .get(&(key_id, hostname.to_string()))
                .cloned()
                .ok_or_else(|| "no live agent tunnel for this system".to_string())?
        };

        let stream = agent.next_stream.fetch_add(1, Ordering::Relaxed);
        let (open_tx, open_rx) = oneshot::channel();
        let (data_tx, data_rx) = mpsc::unbounded_channel();
        agent.streams.lock().await.insert(
            stream,
            StreamSlot {
                open: Some(open_tx),
                data: data_tx,
            },
        );

        if agent
            .out
            .send(TunnelFrame::Open { stream, port }.encode())
            .is_err()
        {
            agent.streams.lock().await.remove(&stream);
            return Err("tunnel closed".to_string());
        }

        match tokio::time::timeout(OPEN_TIMEOUT, open_rx).await {
            Ok(Ok(Ok(()))) => Ok(TunnelStream {
                stream,
                out: agent.out.clone(),
                rx: data_rx,
                leftover: Vec::new(),
                pos: 0,
                eof: false,
                shut: false,
            }),
            Ok(Ok(Err(msg))) => {
                agent.streams.lock().await.remove(&stream);
                Err(msg)
            }
            _ => {
                agent.streams.lock().await.remove(&stream);
                Err("tunnel open timed out".to_string())
            }
        }
    }
}

#[derive(Deserialize)]
pub struct TunnelQuery {
    hostname: String,
}

/// `GET /pub/tunnel` — agent opens its reverse tunnel here. Authenticated by the
/// API key (same as ingest); identified by `?hostname=`.
pub async fn tunnel_ws(
    State(state): State<AppState>,
    Query(q): Query<TunnelQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    let key = match headers.get(API_KEY_HEADER).and_then(|v| v.to_str().ok()) {
        Some(k) => k.to_string(),
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let key_id: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM api_keys WHERE key = $1")
        .bind(&key)
        .fetch_optional(&state.config)
        .await
        .unwrap_or(None);
    let Some((key_id,)) = key_id else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let hostname = if q.hostname.is_empty() {
        "unknown".to_string()
    } else {
        q.hostname.clone()
    };
    ws.on_upgrade(move |socket| serve(state, key_id, hostname, socket))
}

/// Drive one agent connection: register it, pump frames both ways, clean up on close.
async fn serve(state: AppState, key_id: Uuid, hostname: String, mut socket: WebSocket) {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let agent = Arc::new(AgentConn {
        out: out_tx,
        next_stream: AtomicU32::new(1),
        streams: Mutex::new(HashMap::new()),
    });
    state
        .tunnels
        .agents
        .lock()
        .await
        .insert((key_id, hostname.clone()), agent.clone());
    tracing::info!(%hostname, "agent tunnel connected");

    loop {
        tokio::select! {
            // hub -> agent: forward queued frames.
            queued = out_rx.recv() => match queued {
                Some(buf) => {
                    if socket.send(Message::Binary(buf.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            // agent -> hub: decode and route.
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Binary(b))) => {
                    if let Some(frame) = TunnelFrame::decode(&b) {
                        route(&agent, frame).await;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ping/pong/text — ignore
                Some(Err(_)) => break,
            },
        }
    }

    // Deregister and tear down every open stream so readers see EOF.
    state
        .tunnels
        .agents
        .lock()
        .await
        .remove(&(key_id, hostname.clone()));
    for (_, slot) in agent.streams.lock().await.drain() {
        let _ = slot.data.send(StreamMsg::Close);
    }
    tracing::info!(%hostname, "agent tunnel closed");
}

/// Route one agent->hub frame to its stream slot.
async fn route(agent: &Arc<AgentConn>, frame: TunnelFrame) {
    let mut streams = agent.streams.lock().await;
    match frame {
        TunnelFrame::OpenOk { stream } => {
            if let Some(slot) = streams.get_mut(&stream) {
                if let Some(tx) = slot.open.take() {
                    let _ = tx.send(Ok(()));
                }
            }
        }
        TunnelFrame::OpenErr { stream, msg } => {
            if let Some(mut slot) = streams.remove(&stream) {
                if let Some(tx) = slot.open.take() {
                    let _ = tx.send(Err(msg));
                }
            }
        }
        TunnelFrame::Data { stream, bytes } => {
            if let Some(slot) = streams.get(&stream) {
                let _ = slot.data.send(StreamMsg::Data(bytes));
            }
        }
        TunnelFrame::Close { stream } => {
            if let Some(slot) = streams.remove(&stream) {
                let _ = slot.data.send(StreamMsg::Close);
            }
        }
        // The agent is forward-only; it never sends Open.
        TunnelFrame::Open { .. } => {}
    }
}

/// A multiplexed byte stream over an agent tunnel, presented as a normal async
/// socket so the SSH client (Phase 3) can use it as its transport.
pub struct TunnelStream {
    stream: u32,
    out: mpsc::UnboundedSender<Vec<u8>>,
    rx: mpsc::UnboundedReceiver<StreamMsg>,
    leftover: Vec<u8>,
    pos: usize,
    eof: bool,
    shut: bool,
}

impl AsyncRead for TunnelStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // Drain any leftover from a previous Data chunk first.
        if self.pos < self.leftover.len() {
            let n = (self.leftover.len() - self.pos).min(buf.remaining());
            let start = self.pos;
            buf.put_slice(&self.leftover[start..start + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        if self.eof {
            return Poll::Ready(Ok(())); // 0 bytes = EOF
        }
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(StreamMsg::Data(bytes))) => {
                let n = bytes.len().min(buf.remaining());
                buf.put_slice(&bytes[..n]);
                if n < bytes.len() {
                    self.leftover = bytes;
                    self.pos = n;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(StreamMsg::Close)) | Poll::Ready(None) => {
                self.eof = true;
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for TunnelStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let frame = TunnelFrame::Data {
            stream: self.stream,
            bytes: buf.to_vec(),
        };
        match self.out.send(frame.encode()) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "tunnel closed",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.shut {
            self.shut = true;
            let _ = self.out.send(
                TunnelFrame::Close {
                    stream: self.stream,
                }
                .encode(),
            );
        }
        Poll::Ready(Ok(()))
    }
}
