//! Reverse tunnel to the hub for the shell/exec feature (Tier 1 — docs/exec-design.md).
//!
//! Runs only when `ALLOW_SHELL=1`. The agent holds one outbound WebSocket to the
//! hub and, on the hub's request, connects to a **loopback** port (the host's local
//! sshd) and pipes bytes. It is strictly forward-only: it never spawns a process and
//! never dials anything but `127.0.0.1`. So the capability this grants is "forward to
//! a local port", not "exec" — a compromised hub still needs SSH credentials.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use shared::tunnel::TunnelFrame;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

/// Per-connection state shared with the spawned stream tasks.
struct Conn {
    /// Messages queued to the hub (the WS writer drains this). Carries our binary
    /// tunnel frames and the occasional Pong reply.
    out: mpsc::UnboundedSender<Message>,
    /// Per-stream senders feeding bytes to each local TCP write half.
    writers: Mutex<HashMap<u32, mpsc::UnboundedSender<Vec<u8>>>>,
}

impl Conn {
    fn send_frame(&self, f: TunnelFrame) -> bool {
        self.out.send(Message::Binary(f.encode())).is_ok()
    }
}

/// True when `ALLOW_SHELL` is set to an affirmative value.
pub fn enabled() -> bool {
    matches!(
        std::env::var("ALLOW_SHELL").ok().as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Maintain the reverse tunnel forever, reconnecting with backoff. Spawn as a task.
pub async fn run(hub_url: String, api_key: String, hostname: String) {
    let scheme = if hub_url.starts_with("https://") {
        "wss"
    } else {
        "ws"
    };
    let base = hub_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let url = format!(
        "{scheme}://{base}/pub/tunnel?hostname={}",
        urlencode(&hostname)
    );

    let mut backoff = Duration::from_secs(2);
    loop {
        match serve_once(&url, &api_key).await {
            Ok(()) => backoff = Duration::from_secs(2), // clean close — reconnect promptly
            Err(e) => tracing::warn!(error = %e, "shell tunnel disconnected; retrying"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn serve_once(url: &str, api_key: &str) -> anyhow::Result<()> {
    let mut req = url.into_client_request()?;
    req.headers_mut()
        .insert(shared::API_KEY_HEADER, HeaderValue::from_str(api_key)?);
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    tracing::info!("shell tunnel connected");
    let (mut ws_w, mut ws_r) = ws.split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let conn = Arc::new(Conn {
        out: out_tx,
        writers: Mutex::new(HashMap::new()),
    });

    // Writer task: forward queued messages to the socket.
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_w.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = ws_r.next().await {
        match msg {
            Ok(Message::Binary(b)) => {
                if let Some(frame) = TunnelFrame::decode(&b) {
                    handle(&conn, frame).await;
                }
            }
            Ok(Message::Ping(p)) => {
                let _ = conn.out.send(Message::Pong(p));
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }
    writer.abort();
    Ok(())
}

async fn handle(conn: &Arc<Conn>, frame: TunnelFrame) {
    match frame {
        TunnelFrame::Open { stream, port } => {
            let conn = conn.clone();
            tokio::spawn(async move { open_stream(conn, stream, port).await });
        }
        TunnelFrame::Data { stream, bytes } => {
            if let Some(w) = conn.writers.lock().await.get(&stream) {
                let _ = w.send(bytes);
            }
        }
        TunnelFrame::Close { stream } => {
            // Dropping the writer sender closes the local TCP write half.
            conn.writers.lock().await.remove(&stream);
        }
        // The hub never sends these to the agent.
        TunnelFrame::OpenOk { .. } | TunnelFrame::OpenErr { .. } => {}
    }
}

/// Connect to `127.0.0.1:port` and bridge it to `stream`. Loopback only — never an
/// arbitrary host — so this can't be turned into an SSRF/exec primitive.
async fn open_stream(conn: Arc<Conn>, stream: u32, port: u16) {
    let tcp = match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(t) => t,
        Err(e) => {
            conn.send_frame(TunnelFrame::OpenErr {
                stream,
                msg: e.to_string(),
            });
            return;
        }
    };
    conn.send_frame(TunnelFrame::OpenOk { stream });

    let (mut rd, mut wr) = tcp.into_split();
    let (w_tx, mut w_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    conn.writers.lock().await.insert(stream, w_tx);

    // hub -> local TCP
    let writer = tokio::spawn(async move {
        while let Some(b) = w_rx.recv().await {
            if wr.write_all(&b).await.is_err() {
                break;
            }
        }
        let _ = wr.shutdown().await;
    });

    // local TCP -> hub
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match rd.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if !conn.send_frame(TunnelFrame::Data {
                    stream,
                    bytes: buf[..n].to_vec(),
                }) {
                    break;
                }
            }
        }
    }
    conn.writers.lock().await.remove(&stream);
    conn.send_frame(TunnelFrame::Close { stream });
    writer.abort();
}

/// Minimal percent-encoding for the hostname query value (alnum/-._~ pass through).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
