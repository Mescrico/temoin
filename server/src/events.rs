//! Server-side journal: one JSON line per request in `logs/events-YYYY-MM-DD.jsonl`
//! (UTC day), plus an in-memory ring of recent events for the admin page.
//!
//! Only request metadata is recorded (time, client IP, route, status, size,
//! device id, error code) — never object contents, which the server cannot read.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ua: Option<String>,
}

struct Current {
    day: String,
    file: Option<File>,
}

pub struct EventLog {
    dir: PathBuf,
    current: Mutex<Current>,
    recent: Mutex<VecDeque<Event>>,
    capacity: usize,
}

impl EventLog {
    pub fn open(dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(EventLog {
            dir,
            current: Mutex::new(Current { day: String::new(), file: None }),
            recent: Mutex::new(VecDeque::new()),
            capacity: 1000,
        })
    }

    pub fn record(&self, e: Event) {
        let mut line = serde_json::to_vec(&e).unwrap_or_default();
        line.push(b'\n');
        let day = e.ts.get(..10).unwrap_or("unknown").to_string();
        {
            let mut cur = self.current.lock().unwrap();
            if cur.day != day || cur.file.is_none() {
                cur.file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.dir.join(format!("events-{day}.jsonl")))
                    .map_err(|err| tracing::error!("cannot open event log: {err}"))
                    .ok();
                cur.day = day;
            }
            if let Some(f) = cur.file.as_mut() {
                if let Err(err) = f.write_all(&line) {
                    tracing::error!("cannot write event log: {err}");
                }
            }
        }
        let mut recent = self.recent.lock().unwrap();
        if recent.len() == self.capacity {
            recent.pop_front();
        }
        recent.push_back(e);
    }

    /// Most recent first.
    pub fn recent(&self, limit: usize) -> Vec<Event> {
        self.recent.lock().unwrap().iter().rev().take(limit).cloned().collect()
    }
}
