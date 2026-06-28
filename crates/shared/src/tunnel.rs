//! Reverse-tunnel wire protocol (agent <-> hub), used by the shell/exec feature.
//!
//! The agent holds one outbound WebSocket to the hub. Over it the hub multiplexes
//! many independent byte streams (one per console session). Each WS *binary*
//! message is one frame:
//!
//! ```text
//! [u8 type][u32 stream_id LE][payload...]
//! ```
//!
//! The agent is forward-only: it dials `127.0.0.1:<port>` on an `Open` and pipes
//! bytes. It never spawns a process — see docs/exec-design.md (Tier 1). We hand-roll
//! the codec (no serde/JSON) so the byte hot-path stays allocation-light.

/// Frame type tags (byte 0).
const T_OPEN: u8 = 1;
const T_OPEN_OK: u8 = 2;
const T_OPEN_ERR: u8 = 3;
const T_DATA: u8 = 4;
const T_CLOSE: u8 = 5;

const HDR: usize = 5; // type(1) + stream_id(4)

/// One multiplexed tunnel frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelFrame {
    /// hub -> agent: open a TCP connection to `127.0.0.1:port` for `stream`.
    Open { stream: u32, port: u16 },
    /// agent -> hub: the connection for `stream` is established.
    OpenOk { stream: u32 },
    /// agent -> hub: the connection for `stream` failed; `msg` is the reason.
    OpenErr { stream: u32, msg: String },
    /// both ways: payload bytes for `stream`.
    Data { stream: u32, bytes: Vec<u8> },
    /// both ways: close `stream` (EOF / teardown).
    Close { stream: u32 },
}

impl TunnelFrame {
    /// The stream id this frame addresses.
    pub fn stream(&self) -> u32 {
        match self {
            TunnelFrame::Open { stream, .. }
            | TunnelFrame::OpenOk { stream }
            | TunnelFrame::OpenErr { stream, .. }
            | TunnelFrame::Data { stream, .. }
            | TunnelFrame::Close { stream } => *stream,
        }
    }

    /// Encode to a single binary WS message.
    pub fn encode(&self) -> Vec<u8> {
        let stream = self.stream();
        let (tag, extra): (u8, &[u8]) = match self {
            TunnelFrame::Open { .. } => (T_OPEN, &[]),
            TunnelFrame::OpenOk { .. } => (T_OPEN_OK, &[]),
            TunnelFrame::OpenErr { msg, .. } => (T_OPEN_ERR, msg.as_bytes()),
            TunnelFrame::Data { bytes, .. } => (T_DATA, bytes),
            TunnelFrame::Close { .. } => (T_CLOSE, &[]),
        };
        let mut out = Vec::with_capacity(HDR + 2 + extra.len());
        out.push(tag);
        out.extend_from_slice(&stream.to_le_bytes());
        if let TunnelFrame::Open { port, .. } = self {
            out.extend_from_slice(&port.to_le_bytes());
        } else {
            out.extend_from_slice(extra);
        }
        out
    }

    /// Decode a binary WS message. Returns `None` on a malformed/short frame.
    pub fn decode(buf: &[u8]) -> Option<TunnelFrame> {
        if buf.len() < HDR {
            return None;
        }
        let tag = buf[0];
        let stream = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
        let rest = &buf[HDR..];
        match tag {
            T_OPEN => {
                if rest.len() < 2 {
                    return None;
                }
                let port = u16::from_le_bytes([rest[0], rest[1]]);
                Some(TunnelFrame::Open { stream, port })
            }
            T_OPEN_OK => Some(TunnelFrame::OpenOk { stream }),
            T_OPEN_ERR => Some(TunnelFrame::OpenErr {
                stream,
                msg: String::from_utf8_lossy(rest).into_owned(),
            }),
            T_DATA => Some(TunnelFrame::Data {
                stream,
                bytes: rest.to_vec(),
            }),
            T_CLOSE => Some(TunnelFrame::Close { stream }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(f: TunnelFrame) {
        assert_eq!(TunnelFrame::decode(&f.encode()), Some(f));
    }

    #[test]
    fn frames_roundtrip() {
        roundtrip(TunnelFrame::Open {
            stream: 7,
            port: 22,
        });
        roundtrip(TunnelFrame::OpenOk { stream: 7 });
        roundtrip(TunnelFrame::OpenErr {
            stream: 7,
            msg: "connection refused".into(),
        });
        roundtrip(TunnelFrame::Data {
            stream: 9,
            bytes: vec![0, 1, 2, 255, 254],
        });
        roundtrip(TunnelFrame::Close { stream: 9 });
    }

    #[test]
    fn short_and_bad_frames_rejected() {
        assert_eq!(TunnelFrame::decode(&[]), None);
        assert_eq!(TunnelFrame::decode(&[1, 0, 0]), None); // short header
        assert_eq!(TunnelFrame::decode(&[T_OPEN, 1, 0, 0, 0]), None); // open w/o port
        assert_eq!(TunnelFrame::decode(&[99, 0, 0, 0, 0]), None); // unknown tag
    }

    #[test]
    fn data_payload_can_be_empty() {
        roundtrip(TunnelFrame::Data {
            stream: 1,
            bytes: vec![],
        });
    }
}
