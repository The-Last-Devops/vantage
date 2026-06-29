//! Interactive SSH console bridge (shell/exec Phase 3c — docs/exec-design.md).
//!
//! Flow: the user does a step-up (POST `…/console/ticket` with their password) which
//! unseals their SSH key and mints a single-use, short-lived [`ExecTicket`]. The
//! browser then opens the console WebSocket with that ticket; here we open a stream
//! through the agent tunnel to the host's loopback sshd, run an SSH client (`russh`)
//! as the user's own account, request a PTY + shell, and bridge it to xterm.js.
//!
//! Security: the decrypted key lives only in the ticket (≤30s, single-use) and in
//! this task's memory for the session. We only record the **output** stream to the
//! transcript — never raw keystrokes — so a typed (echo-off) password is never
//! captured, while echoed commands still appear. The host's own SSH/sudo model
//! governs what the session can do.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
};
use rand::RngCore;
use russh::client::{self, Handler};
use russh::{ChannelMsg, CryptoVec};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::{rbac, AppState};

/// How the user chose to authenticate to the host's sshd for this session.
/// Both variants hold a transient secret that lives only inside the ticket (≤ TTL)
/// and this task's memory.
pub enum ExecAuth {
    /// SSH password auth — the host password the user typed at connect.
    Password(String),
    /// SSH publickey auth — the user's private key (unsealed PEM/OpenSSH) + its
    /// passphrase if the key itself is encrypted.
    Key {
        pem: String,
        passphrase: Option<String>,
    },
}

/// A step-up ticket: holds the connection target + chosen auth for one console
/// session. Single-use and short-lived.
pub struct ExecTicket {
    pub system_id: Uuid,
    pub user_id: Uuid,
    pub user_email: String,
    pub ssh_user: String,
    pub auth: ExecAuth,
    pub key_id: Uuid, // the system's agent api-key id (tunnel routing)
    pub hostname: String,
    pub ssh_port: u16,
    pub created: Instant,
}

const TICKET_TTL: Duration = Duration::from_secs(30);

/// In-memory store of pending console tickets.
#[derive(Clone, Default)]
pub struct ExecTickets {
    inner: Arc<Mutex<HashMap<String, ExecTicket>>>,
}

impl ExecTickets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a ticket, returning its opaque id.
    pub async fn issue(&self, t: ExecTicket) -> String {
        let mut raw = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut raw);
        let id = hex(&raw);
        self.inner.lock().await.insert(id.clone(), t);
        id
    }

    /// Consume a ticket (single use); `None` if missing or expired.
    pub async fn take(&self, id: &str) -> Option<ExecTicket> {
        let t = self.inner.lock().await.remove(id)?;
        if t.created.elapsed() > TICKET_TTL {
            None
        } else {
            Some(t)
        }
    }
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

/// SSH client handler. We reached the host through our own agent tunnel, which only
/// ever connects to `127.0.0.1` on that specific host — so the transport itself
/// authenticates the host identity and we accept its key. (Future: pin on first use.)
struct ConsoleHandler;
#[async_trait::async_trait]
impl Handler for ConsoleHandler {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[derive(Deserialize)]
pub struct ConsoleQuery {
    ticket: String,
}

/// `GET /api/systems/:id/console?ticket=…` — the console WebSocket. The ticket must
/// belong to the logged-in user and this system; we re-check `require_exec` too.
pub async fn console_ws(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(q): Query<ConsoleQuery>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    let Some(ticket) = state.exec_tickets.take(&q.ticket).await else {
        return (StatusCode::FORBIDDEN, "invalid or expired ticket").into_response();
    };
    if ticket.user_id != user.id || ticket.system_id != id {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Defense in depth: re-verify exec rights on the system's namespace.
    let ns: Option<(Uuid,)> = sqlx::query_as("SELECT namespace_id FROM systems WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .unwrap_or(None);
    let Some((ns,)) = ns else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if rbac::require_exec(&state, &user, ns).await.is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| run(state, ns, ticket, socket))
}

/// Record one transcript chunk (output only — see module docs).
async fn record(state: &AppState, session: Uuid, seq: &mut i32, data: &[u8]) {
    *seq += 1;
    let _ = sqlx::query(
        "INSERT INTO exec_transcript (session_id, seq, stream, data) VALUES ($1, $2, 'out', $3)",
    )
    .bind(session)
    .bind(*seq)
    .bind(data)
    .execute(&state.config)
    .await;
}

async fn run(state: AppState, ns: Uuid, t: ExecTicket, mut socket: WebSocket) {
    // Open the audit session row up front (immutable record).
    let system_name: String = sqlx::query_scalar("SELECT name FROM systems WHERE id = $1")
        .bind(t.system_id)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| t.hostname.clone());
    let session_id: Uuid = match sqlx::query_scalar(
        "INSERT INTO exec_sessions \
         (system_id, system_name, namespace_id, user_id, user_email, transport, ssh_user) \
         VALUES ($1, $2, $3, $4, $5, 'ssh', $6) RETURNING id",
    )
    .bind(t.system_id)
    .bind(&system_name)
    .bind(ns)
    .bind(t.user_id)
    .bind(&t.user_email)
    .bind(&t.ssh_user)
    .fetch_one(&state.config)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "exec: could not open session row");
            let _ = socket
                .send(Message::Text(
                    "\r\nInternal error opening session.\r\n".into(),
                ))
                .await;
            return;
        }
    };
    tracing::warn!(%system_name, user = %t.user_email, %session_id, "exec: console session opened");

    let (status, err) = bridge(&state, session_id, &t, &mut socket).await;
    let _ = sqlx::query(
        "UPDATE exec_sessions SET ended_at = now(), status = $2, error = $3 WHERE id = $1",
    )
    .bind(session_id)
    .bind(status)
    .bind(err)
    .execute(&state.config)
    .await;
    tracing::warn!(%session_id, status, "exec: console session ended");
}

/// Returns (status, error) for the audit row. Sends a human line to the socket on
/// setup failures so the terminal shows why.
async fn bridge(
    state: &AppState,
    session_id: Uuid,
    t: &ExecTicket,
    socket: &mut WebSocket,
) -> (&'static str, Option<String>) {
    macro_rules! fail {
        ($msg:expr) => {{
            let _ = socket
                .send(Message::Text(format!("\r\n{}\r\n", $msg).into()))
                .await;
            return ("error", Some($msg.to_string()));
        }};
    }

    // 1) Open a byte stream to the host's loopback sshd through the agent tunnel.
    let stream = match state
        .tunnels
        .open_stream(t.key_id, &t.hostname, t.ssh_port)
        .await
    {
        Ok(s) => s,
        Err(e) => fail!(format!("Agent tunnel unavailable: {e}")),
    };

    // 2) SSH handshake + publickey auth as the user's own account.
    let config = Arc::new(client::Config::default());
    let mut handle = match client::connect_stream(config, stream, ConsoleHandler).await {
        Ok(h) => h,
        Err(e) => fail!(format!("SSH connect failed: {e}")),
    };
    let authed = match &t.auth {
        ExecAuth::Password(pw) => handle.authenticate_password(&t.ssh_user, pw.clone()).await,
        ExecAuth::Key { pem, passphrase } => {
            match russh::keys::decode_secret_key(pem, passphrase.as_deref()) {
                Ok(k) => {
                    handle
                        .authenticate_publickey(&t.ssh_user, Arc::new(k))
                        .await
                }
                Err(e) => fail!(format!("Could not load your SSH key: {e}")),
            }
        }
    };
    match authed {
        Ok(true) => {}
        Ok(false) => {
            fail!("SSH authentication rejected by the host (wrong password or key not authorized)")
        }
        Err(e) => fail!(format!("SSH authentication error: {e}")),
    }

    // 3) Open a session channel with a PTY + shell.
    let channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => fail!(format!("Could not open SSH channel: {e}")),
    };
    let chan_id = channel.id();
    if let Err(e) = channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
    {
        fail!(format!("PTY request failed: {e}"));
    }
    if let Err(e) = channel.request_shell(false).await {
        fail!(format!("Shell request failed: {e}"));
    }

    // 4) Bridge. Writes (stdin/resize) go via the Handle so the only borrow of
    // `channel` in the select is `wait()` — avoids a double-borrow.
    let mut channel = channel;
    let mut seq = 0i32;
    loop {
        tokio::select! {
            from_browser = socket.recv() => match from_browser {
                Some(Ok(Message::Binary(b))) => {
                    if handle.data(chan_id, CryptoVec::from_slice(&b)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(_txt))) => {
                    // Live PTY resize is a follow-up; the PTY is fixed at the size
                    // requested below. xterm still renders; wide TUIs may wrap.
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
            from_host = channel.wait() => match from_host {
                Some(ChannelMsg::Data { data }) => {
                    record(state, session_id, &mut seq, &data).await;
                    if socket.send(Message::Binary(data.to_vec().into())).await.is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    record(state, session_id, &mut seq, &data).await;
                    if socket.send(Message::Binary(data.to_vec().into())).await.is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
        }
    }
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    ("closed", None)
}
