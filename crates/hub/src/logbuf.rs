//! In-memory ring buffer of recent log lines so the UI can show hub logs for
//! debugging without shelling into the pod. A tracing fmt writer tees each
//! formatted line here (bounded) in addition to stdout. The read path is
//! admin-only (see `api::admin_logs`) — logs may reveal operational detail.

use std::collections::VecDeque;
use std::io;
use std::sync::{Mutex, OnceLock};

/// How many recent lines to keep. ~2000 lines is plenty to debug a recent issue
/// and stays tiny in memory; older lines drop off the front.
const CAP: usize = 2000;

fn buffer() -> &'static Mutex<VecDeque<String>> {
    static B: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAP)))
}

/// The most recent `limit` lines, oldest first.
pub fn recent(limit: usize) -> Vec<String> {
    let b = buffer().lock().unwrap_or_else(|e| e.into_inner());
    let skip = b.len().saturating_sub(limit);
    b.iter().skip(skip).cloned().collect()
}

fn push_line(line: &str) {
    let mut b = buffer().lock().unwrap_or_else(|e| e.into_inner());
    if b.len() >= CAP {
        b.pop_front();
    }
    b.push_back(line.to_string());
}

/// `io::Write` that splits incoming bytes into lines and stores the complete ones.
/// tracing's fmt layer writes one formatted event (ending in '\n') per call.
pub struct Writer;
impl io::Write for Writer {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let s = String::from_utf8_lossy(data);
        for line in s.split('\n') {
            let line = line.trim_end_matches('\r');
            if !line.trim().is_empty() {
                push_line(line);
            }
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// MakeWriter handing out a fresh `Writer` per event; tee'd with stdout in `main`.
#[derive(Clone)]
pub struct MakeBuf;
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MakeBuf {
    type Writer = Writer;
    fn make_writer(&'a self) -> Self::Writer {
        Writer
    }
}
