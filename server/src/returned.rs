//! Recordings handed back to the person who filmed them.
//!
//! Nothing is ever deleted. Once a hand-back link has been retrieved, the objects it
//! covered move from `objects/` to `returned/` (same layout) and stop counting against
//! the device quota, so the person can keep filming. Every move is appended to
//! `returned.jsonl`; the ledger is untouched and `sr verify` looks in both folders.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Moved {
    pub index: u64,
    pub device: String,
    pub session: String,
    pub seq: u32,
    pub size: u64,
    pub moved_at: String,
    pub share: String,
}

pub struct Returned {
    data_dir: PathBuf,
    log: PathBuf,
    set: HashSet<(String, u32)>,
    device_bytes: HashMap<String, u64>,
    device_objects: HashMap<String, u64>,
}

pub fn object_rel(session: &str, seq: u32) -> PathBuf {
    PathBuf::from(&session[..2]).join(session).join(format!("{seq:06}.age"))
}

impl Returned {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let log = data_dir.join("returned.jsonl");
        // Always present, so `tar … returned.jsonl` on the operator side never fails.
        OpenOptions::new().create(true).append(true).open(&log)?;
        let mut r = Returned {
            data_dir: data_dir.to_owned(),
            log: log.clone(),
            set: HashSet::new(),
            device_bytes: HashMap::new(),
            device_objects: HashMap::new(),
        };
        for line in BufReader::new(std::fs::File::open(&log)?).lines() {
            let line = line?;
            if let Ok(m) = serde_json::from_str::<Moved>(&line) {
                r.account(&m);
            }
        }
        Ok(r)
    }

    fn account(&mut self, m: &Moved) {
        if self.set.insert((m.session.clone(), m.seq)) {
            *self.device_bytes.entry(m.device.clone()).or_default() += m.size;
            *self.device_objects.entry(m.device.clone()).or_default() += 1;
        }
    }

    pub fn contains(&self, session: &str, seq: u32) -> bool {
        self.set.contains(&(session.to_string(), seq))
    }

    pub fn device_bytes(&self, device: &str) -> u64 {
        self.device_bytes.get(device).copied().unwrap_or(0)
    }

    pub fn device_objects(&self, device: &str) -> u64 {
        self.device_objects.get(device).copied().unwrap_or(0)
    }

    pub fn path_of(&self, session: &str, seq: u32) -> PathBuf {
        self.data_dir.join("returned").join(object_rel(session, seq))
    }

    /// Moves every object of `device` with ledger index <= `upto` to `returned/`.
    /// Idempotent: already moved objects are skipped. Returns (objects, bytes) moved.
    pub fn move_device(
        &mut self,
        entries: &[crate::ledger::Entry],
        device: &str,
        upto: u64,
        share: &str,
        now: &str,
    ) -> anyhow::Result<(u64, u64)> {
        let mut log = OpenOptions::new().append(true).open(&self.log)?;
        let (mut n, mut bytes) = (0, 0);
        for e in entries.iter().filter(|e| e.device == device && e.index <= upto) {
            if self.contains(&e.session, e.seq) || e.session.len() < 2 {
                continue;
            }
            let src = self.data_dir.join("objects").join(object_rel(&e.session, e.seq));
            let dst = self.path_of(&e.session, e.seq);
            if !dst.exists() {
                if !src.exists() {
                    tracing::warn!(session = %e.session, seq = e.seq, "object to hand back is missing");
                    continue;
                }
                std::fs::create_dir_all(dst.parent().unwrap())?;
                // Same dataset: a rename, never a copy + delete.
                std::fs::rename(&src, &dst)?;
                if let Some(dir) = src.parent() {
                    let _ = std::fs::remove_dir(dir); // only if now empty
                }
            }
            let m = Moved {
                index: e.index,
                device: device.to_string(),
                session: e.session.clone(),
                seq: e.seq,
                size: e.size,
                moved_at: now.to_string(),
                share: share.to_string(),
            };
            let mut line = serde_json::to_vec(&m)?;
            line.push(b'\n');
            log.write_all(&line)?;
            self.account(&m);
            n += 1;
            bytes += e.size;
        }
        log.sync_data()?;
        Ok((n, bytes))
    }
}
