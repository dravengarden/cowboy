//! Bounded change history. Gaps invalidate comparisons even for unchanged files.
use anyhow::{Context as _, Result, ensure};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Component, Path};

const MAX_EVENTS: usize = 1024;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_PATHS: usize = 64;
const MAX_PATH_BYTES: usize = 4096;
const PAGE_EVENTS: usize = 64;
// Even worst-case JSON escaping stays below the Machine wire response bound.
const PAGE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub instance: String,
    /// Decimal text preserves u64 positions through JavaScript consumers.
    pub position: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gap {
    InitialScan,
    QueueOverflow,
    Coverage,
    Budget,
    RootChanged,
    Disconnected,
    Exhausted,
    HistoryExpired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub cursor: Cursor,
    /// Changes invalidate these relative paths and descendants. Empty is never
    /// used as a shorthand for a complete or unchanged filesystem.
    pub paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageState {
    Changes,
    Gap,
    Reset,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    /// Event continuity only. Inotify does not observe mmap writes, remote
    /// network-filesystem writes or analyzer input consumption. Even an empty
    /// reconciled page must never establish unchanged content/project freshness.
    pub coverage: Coverage,
    pub state: PageState,
    pub head: Cursor,
    pub through: Cursor,
    pub low_water: String,
    pub scan_at: Option<String>,
    pub reason: Option<Gap>,
    pub events: Vec<Change>,
    pub more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    EventStreamOnly,
}

/// Non-serializable scan reservation. Only the watcher may complete one after
/// scanning the pinned root and draining its actual event source to a barrier.
pub struct Scan {
    instance: String,
    position: u64,
}

struct Entry {
    position: u64,
    paths: Vec<String>,
    bytes: usize,
}

pub struct Journal {
    instance: String,
    position: u64,
    low_water: u64,
    last_gap: u64,
    unresolved: Option<Gap>,
    scan_at: Option<u64>,
    entries: VecDeque<Entry>,
    bytes: usize,
    exhausted: bool,
}

impl Journal {
    pub fn new() -> Result<Self> {
        let mut random = [0; 16];
        rand::rngs::OsRng.try_fill_bytes(&mut random)?;
        Ok(Self {
            instance: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            position: 0,
            low_water: 0,
            last_gap: 0,
            unresolved: Some(Gap::InitialScan),
            scan_at: None,
            entries: VecDeque::new(),
            bytes: 0,
            exhausted: false,
        })
    }

    fn cursor(&self, position: u64) -> Cursor {
        Cursor {
            instance: self.instance.clone(),
            position: position.to_string(),
        }
    }

    pub fn head(&self) -> Cursor {
        self.cursor(self.position)
    }

    fn advance(&mut self) -> Result<u64> {
        let Some(next) = self.position.checked_add(1) else {
            self.exhausted = true;
            self.unresolved = Some(Gap::Exhausted);
            self.scan_at = None;
            anyhow::bail!("observer sequence exhausted");
        };
        self.position = next;
        Ok(next)
    }

    pub fn gap(&mut self, reason: Gap) {
        if let Ok(position) = self.advance() {
            self.last_gap = position;
            self.unresolved = Some(reason);
            self.scan_at = None;
        }
    }

    /// No ignore rules: generated and ignored paths still invalidate analysis.
    pub fn record(&mut self, mut paths: Vec<String>) -> Result<()> {
        let valid = !paths.is_empty()
            && paths.len() <= MAX_PATHS
            && paths.iter().all(|path| {
                !path.is_empty()
                    && path.len() <= MAX_PATH_BYTES
                    && !path.contains(['\0', '\\'])
                    && Path::new(path)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
                    && Path::new(path)
                        .components()
                        .collect::<std::path::PathBuf>()
                        .to_str()
                        == Some(path.as_str())
            });
        if !valid {
            self.gap(Gap::Coverage);
            anyhow::bail!("observer change paths are not bounded relative paths");
        }
        paths.sort_unstable();
        paths.dedup();
        let bytes = paths.iter().map(|p| p.len() + 32).sum::<usize>() + 64;
        if bytes > PAGE_BYTES {
            self.gap(Gap::Budget);
            anyhow::bail!("observer change exceeds history budget");
        }
        let position = self.advance()?;
        while self.entries.len() >= MAX_EVENTS || self.bytes + bytes > MAX_BYTES {
            let old = self
                .entries
                .pop_front()
                .expect("bounded entry fits empty journal");
            self.bytes -= old.bytes;
            self.low_water = old.position;
        }
        self.entries.push_back(Entry {
            position,
            paths,
            bytes,
        });
        self.bytes += bytes;
        Ok(())
    }

    pub fn begin_scan(&self) -> Result<Scan> {
        ensure!(!self.exhausted, "observer sequence exhausted");
        Ok(Scan {
            instance: self.instance.clone(),
            position: self.position,
        })
    }

    /// A caller must perform a full bounded scan plus a kernel event barrier;
    /// this check rejects any event or gap observed during that scan. It is not
    /// itself a watcher barrier, a quiet-period heuristic or freshness evidence.
    pub fn finish_scan(&mut self, scan: Scan) -> Result<()> {
        ensure!(
            !self.exhausted && scan.instance == self.instance && scan.position == self.position,
            "observer changed during reconciliation"
        );
        self.unresolved = None;
        self.scan_at = Some(self.position);
        Ok(())
    }

    pub fn read(&self, cursor: Option<&Cursor>) -> Result<Page> {
        let mut page = Page {
            coverage: Coverage::EventStreamOnly,
            state: PageState::Reset,
            head: self.head(),
            through: self.head(),
            low_water: self.low_water.to_string(),
            scan_at: self.scan_at.map(|n| n.to_string()),
            reason: self.unresolved,
            events: Vec::new(),
            more: false,
        };
        let Some(cursor) = cursor.filter(|c| c.instance == self.instance) else {
            return Ok(page);
        };
        let position: u64 = cursor.position.parse().context("invalid observer cursor")?;
        ensure!(
            position.to_string() == cursor.position && position <= self.position,
            "observer cursor was not issued"
        );
        if self.unresolved.is_some() || position < self.low_water || position < self.last_gap {
            page.state = PageState::Gap;
            page.reason = page.reason.or(Some(Gap::HistoryExpired));
            return Ok(page);
        }
        page.state = PageState::Changes;
        page.through = cursor.clone();
        let mut bytes = 0;
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.position > position)
        {
            if page.events.len() == PAGE_EVENTS || bytes + entry.bytes > PAGE_BYTES {
                page.more = true;
                break;
            }
            bytes += entry.bytes;
            page.through = self.cursor(entry.position);
            page.events.push(Change {
                cursor: page.through.clone(),
                paths: entry.paths.clone(),
            });
        }
        if !page.more {
            page.through = self.head();
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests;
