use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};
use tokio::sync::{Mutex, OnceCell};

use crate::code_review::{DiffDocument, DiffScope};

const DEFAULT_PAGE_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_BYTES: usize = 48 * 1024 * 1024;
const DEFAULT_MAX_ENTRIES: usize = 12;
const DEFAULT_TTL: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffSnapshotKey {
    pub session_id: String,
    pub cwd: String,
    pub path: String,
    pub context: usize,
    pub show_whitespace: bool,
    pub scope: DiffScope,
}

#[derive(Debug)]
pub struct DiffSnapshot {
    pub path: String,
    pub revision: String,
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub limited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPage {
    pub path: String,
    pub revision: String,
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub next_cursor: Option<String>,
    pub limited: bool,
}

type SnapshotResult = Result<Arc<DiffSnapshot>, String>;

struct Entry {
    key: DiffSnapshotKey,
    cell: Arc<OnceCell<SnapshotResult>>,
    touched_at: Instant,
}

struct CacheInner {
    entries: Vec<Entry>,
}

pub struct DiffSnapshotCache {
    inner: Mutex<CacheInner>,
    page_bytes: usize,
    max_bytes: usize,
    max_entries: usize,
    ttl: Duration,
}

impl Default for DiffSnapshotCache {
    fn default() -> Self {
        Self::new(
            DEFAULT_PAGE_BYTES,
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_ENTRIES,
            DEFAULT_TTL,
        )
    }
}

impl DiffSnapshotCache {
    fn new(page_bytes: usize, max_bytes: usize, max_entries: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                entries: Vec::new(),
            }),
            page_bytes,
            max_bytes,
            max_entries,
            ttl,
        }
    }

    pub async fn first_page<F, Fut>(
        &self,
        key: DiffSnapshotKey,
        generate: F,
    ) -> Result<DiffPage, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<DiffDocument, String>>,
    {
        let cell = {
            let mut inner = self.inner.lock().await;
            self.prune(&mut inner, Instant::now());
            if let Some(entry) = inner.entries.iter_mut().find(|entry| entry.key == key) {
                entry.touched_at = Instant::now();
                Arc::clone(&entry.cell)
            } else {
                let cell = Arc::new(OnceCell::new());
                inner.entries.push(Entry {
                    key,
                    cell: Arc::clone(&cell),
                    touched_at: Instant::now(),
                });
                cell
            }
        };
        let snapshot = cell
            .get_or_init(|| async {
                generate().await.map(|document| {
                    let revision = format!("{:x}", Sha256::digest(document.text.as_bytes()));
                    Arc::new(DiffSnapshot {
                        path: document.path,
                        revision,
                        text: document.text,
                        added: document.added,
                        removed: document.removed,
                        limited: document.truncated,
                    })
                })
            })
            .await
            .clone()?;
        {
            let mut inner = self.inner.lock().await;
            self.prune(&mut inner, Instant::now());
        }
        Ok(self.page(&snapshot, 0))
    }

    pub async fn next_page(&self, session_id: &str, cursor: &str) -> Result<DiffPage, String> {
        let (revision, offset) = parse_cursor(cursor)?;
        let snapshot = {
            let mut inner = self.inner.lock().await;
            self.prune(&mut inner, Instant::now());
            let Some(entry) = inner.entries.iter_mut().find(|entry| {
                entry.key.session_id == session_id
                    && entry
                        .cell
                        .get()
                        .and_then(|result| result.as_ref().ok())
                        .is_some_and(|snapshot| snapshot.revision == revision)
            }) else {
                return Err("diff snapshot expired".to_owned());
            };
            entry.touched_at = Instant::now();
            entry
                .cell
                .get()
                .and_then(|result| result.as_ref().ok())
                .cloned()
                .ok_or_else(|| "diff snapshot unavailable".to_owned())?
        };
        if offset == 0 || offset >= snapshot.text.len() {
            return Err("invalid diff cursor".to_owned());
        }
        Ok(self.page(&snapshot, offset))
    }

    fn page(&self, snapshot: &DiffSnapshot, offset: usize) -> DiffPage {
        let desired = offset
            .saturating_add(self.page_bytes)
            .min(snapshot.text.len());
        let mut end = desired;
        while end > offset && !snapshot.text.is_char_boundary(end) {
            end -= 1;
        }
        if end < snapshot.text.len()
            && let Some(newline) = snapshot.text[offset..end].rfind('\n')
        {
            end = offset + newline + 1;
        }
        if end == offset {
            end = desired;
            while end < snapshot.text.len() && !snapshot.text.is_char_boundary(end) {
                end += 1;
            }
        }
        let next_cursor =
            (end < snapshot.text.len()).then(|| format!("{}:{end}", snapshot.revision));
        DiffPage {
            path: snapshot.path.clone(),
            revision: snapshot.revision.clone(),
            text: snapshot.text[offset..end].to_owned(),
            added: snapshot.added,
            removed: snapshot.removed,
            next_cursor,
            limited: snapshot.limited,
        }
    }

    fn prune(&self, inner: &mut CacheInner, now: Instant) {
        inner
            .entries
            .retain(|entry| now.duration_since(entry.touched_at) <= self.ttl);
        while inner.entries.len() > self.max_entries || cache_bytes(inner) > self.max_bytes {
            let Some((oldest, _)) = inner
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.touched_at)
            else {
                break;
            };
            inner.entries.remove(oldest);
        }
    }
}

fn cache_bytes(inner: &CacheInner) -> usize {
    inner
        .entries
        .iter()
        .filter_map(|entry| entry.cell.get())
        .filter_map(|result| result.as_ref().ok())
        .map(|snapshot| snapshot.text.len())
        .sum()
}

fn parse_cursor(cursor: &str) -> Result<(&str, usize), String> {
    let (revision, offset) = cursor
        .rsplit_once(':')
        .ok_or_else(|| "invalid diff cursor".to_owned())?;
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid diff cursor".to_owned());
    }
    let offset = offset
        .parse::<usize>()
        .map_err(|_| "invalid diff cursor".to_owned())?;
    Ok((revision, offset))
}

#[cfg(test)]
mod tests {
    use super::{DiffSnapshotCache, DiffSnapshotKey};
    use crate::code_review::{DiffDocument, DiffScope};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn key(path: &str) -> DiffSnapshotKey {
        DiffSnapshotKey {
            session_id: "session".to_owned(),
            cwd: "/work".to_owned(),
            path: path.to_owned(),
            context: 6,
            show_whitespace: true,
            scope: DiffScope::Unstaged,
        }
    }

    fn document(path: &str, lines: usize) -> DiffDocument {
        DiffDocument {
            path: path.to_owned(),
            text: (0..lines).map(|index| format!("+line {index}\n")).collect(),
            added: lines,
            removed: 0,
            truncated: false,
        }
    }

    #[tokio::test]
    async fn concurrent_first_pages_share_one_immutable_snapshot() {
        let cache = Arc::new(DiffSnapshotCache::new(32, 1024, 4, Duration::from_secs(60)));
        let generated = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let cache = Arc::clone(&cache);
            let generated = Arc::clone(&generated);
            tasks.push(tokio::spawn(async move {
                cache
                    .first_page(key("a.rs"), || async move {
                        generated.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        Ok(document("a.rs", 20))
                    })
                    .await
                    .unwrap()
            }));
        }
        let first = tasks.remove(0).await.unwrap();
        let second = tasks.remove(0).await.unwrap();
        assert_eq!(generated.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        assert!(first.next_cursor.is_some());
    }

    #[tokio::test]
    async fn cursor_pages_reassemble_the_snapshot_without_overlap() {
        let cache = DiffSnapshotCache::new(24, 1024, 4, Duration::from_secs(60));
        let expected = document("a.rs", 20);
        let first = cache
            .first_page(key("a.rs"), || async { Ok(document("a.rs", 20)) })
            .await
            .unwrap();
        let mut text = first.text;
        let mut cursor = first.next_cursor;
        assert!(
            cache
                .next_page("other-session", cursor.as_deref().unwrap())
                .await
                .is_err()
        );
        while let Some(next) = cursor {
            let page = cache.next_page("session", &next).await.unwrap();
            text.push_str(&page.text);
            cursor = page.next_cursor;
        }
        assert_eq!(text, expected.text);
    }

    #[tokio::test]
    async fn cursors_resume_at_line_boundaries_and_expire_cleanly() {
        let cache = DiffSnapshotCache::new(24, 1024, 4, Duration::ZERO);
        let first = cache
            .first_page(key("a.rs"), || async { Ok(document("a.rs", 20)) })
            .await
            .unwrap();
        assert!(first.text.ends_with('\n'));
        assert!(
            cache
                .next_page("session", first.next_cursor.as_deref().unwrap())
                .await
                .is_err()
        );
    }
}
