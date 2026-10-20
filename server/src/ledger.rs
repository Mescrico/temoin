//! Append-only, hash-chained receipt log (`ledger.jsonl`).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub index: u64,
    pub received_at: String,
    pub device: String,
    pub session: String,
    pub seq: u32,
    pub size: u64,
    pub sha256: String,
    pub prev: String,
    pub chain: String,
}

impl Entry {
    pub fn compute_chain(&self) -> String {
        let line = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.prev, self.index, self.received_at, self.device, self.session, self.seq, self.size, self.sha256
        );
        hex::encode(Sha256::digest(line.as_bytes()))
    }
}

pub struct Ledger {
    path: PathBuf,
    file: File,
    pub head: String,
    pub next_index: u64,
    pub total_bytes: u64,
    pub sessions: u64,
    pub last_received_at: Option<String>,
    pub device_bytes: HashMap<String, u64>,
    pub device_objects: HashMap<String, u64>,
    pub device_last: HashMap<String, String>,
}

impl Ledger {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let mut ledger = Ledger {
            path: path.to_owned(),
            file: OpenOptions::new().create(true).append(true).open(path)?,
            head: GENESIS.to_string(),
            next_index: 0,
            total_bytes: 0,
            sessions: 0,
            last_received_at: None,
            device_bytes: HashMap::new(),
            device_objects: HashMap::new(),
            device_last: HashMap::new(),
        };
        let reader = BufReader::new(File::open(path)?);
        for (n, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: Entry = match serde_json::from_str(&line) {
                Ok(e) => e,
                // A torn last line (power cut mid-write) is tolerated; anything else is not.
                Err(e) => {
                    tracing::warn!("ledger line {} unreadable: {e}", n + 1);
                    continue;
                }
            };
            ledger.account(&entry);
        }
        Ok(ledger)
    }

    fn account(&mut self, e: &Entry) {
        self.head = e.chain.clone();
        self.next_index = e.index + 1;
        self.total_bytes += e.size;
        if e.seq == 0 {
            self.sessions += 1;
        }
        self.last_received_at = Some(e.received_at.clone());
        *self.device_bytes.entry(e.device.clone()).or_default() += e.size;
        *self.device_objects.entry(e.device.clone()).or_default() += 1;
        self.device_last.insert(e.device.clone(), e.received_at.clone());
    }

    pub fn append(
        &mut self,
        received_at: String,
        device: &str,
        session: &str,
        seq: u32,
        size: u64,
        sha256: &str,
    ) -> anyhow::Result<Entry> {
        let mut entry = Entry {
            index: self.next_index,
            received_at,
            device: device.to_string(),
            session: session.to_string(),
            seq,
            size,
            sha256: sha256.to_string(),
            prev: self.head.clone(),
            chain: String::new(),
        };
        entry.chain = entry.compute_chain();
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        self.account(&entry);
        Ok(entry)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// All readable entries, without chain verification (the server's own file).
pub fn read_entries(path: &Path) -> anyhow::Result<Vec<Entry>> {
    let reader = BufReader::new(File::open(path)?);
    Ok(reader.lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect())
}

/// Verifies the chain of a ledger file. Returns the entries on success.
pub fn verify_file(path: &Path) -> anyhow::Result<Vec<Entry>> {
    let reader = BufReader::new(File::open(path)?);
    let mut prev = GENESIS.to_string();
    let mut out = Vec::new();
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let e: Entry = serde_json::from_str(&line)
            .map_err(|err| anyhow::anyhow!("line {}: {err}", n + 1))?;
        if e.prev != prev {
            anyhow::bail!("line {}: chain broken (prev mismatch)", n + 1);
        }
        if e.index != out.len() as u64 {
            anyhow::bail!("line {}: index gap", n + 1);
        }
        if e.compute_chain() != e.chain {
            anyhow::bail!("line {}: entry altered (chain mismatch)", n + 1);
        }
        prev = e.chain.clone();
        out.push(e);
    }
    Ok(out)
}
