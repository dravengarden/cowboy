//! Bounded background event collection. Admission remains the caller's job:
//! pass the original pinned directory and a current continuation/root fence.
use super::{Cursor, Journal, Page, linux::Source};
use anyhow::{Context as _, Result, ensure};
use cowboy::managed_calls::host::LiveGrant;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

const TICK: Duration = Duration::from_millis(200);
const RECONCILE: Duration = Duration::from_secs(5);

enum Request {
    Read(Option<Cursor>, oneshot::Sender<Result<Page>>),
    Wake,
}

struct Cancellation {
    requests: mpsc::SyncSender<Request>,
    stop: Arc<AtomicBool>,
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.try_send(Request::Wake);
    }
}

/// Capacity belongs to the resident owner, not to individual requests. A slot
/// remains occupied until native work and descriptors are released, including
/// a blocked scan. Dropping an async waiter cannot release that reservation.
pub struct Capacity(Arc<Semaphore>);

impl Default for Capacity {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(2)))
    }
}

pub struct Worker {
    grant: Arc<LiveGrant>,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
    requests: mpsc::SyncSender<Request>,
    stop: Arc<AtomicBool>,
    owner: Option<oneshot::Sender<()>>,
    finished: Arc<AtomicBool>,
    ended: Option<oneshot::Receiver<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.owner.take();
    }
}

impl Worker {
    pub fn start(
        root: File,
        path: PathBuf,
        grant: Arc<LiveGrant>,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
        capacity: &Capacity,
    ) -> Result<Self> {
        ensure!(grant.current() && current(), "observer continuation ended");
        let runtime = tokio::runtime::Handle::try_current()
            .context("observer requires the resident runtime")?;
        let permit = Arc::clone(&capacity.0)
            .try_acquire_owned()
            .context("observer root capacity exhausted")?;
        let (requests, receiver) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let (owner, dropped) = oneshot::channel();
        let (finished, ended) = oneshot::channel();
        let cleaned = Arc::new(AtomicBool::new(false));
        let thread_cleaned = Arc::clone(&cleaned);
        let thread_stop = Arc::clone(&stop);
        let thread_grant = Arc::clone(&grant);
        let thread_current = Arc::clone(&current);
        std::thread::Builder::new()
            .name("code-observer".into())
            .spawn(move || {
                let _permit: OwnedSemaphorePermit = permit;
                run(
                    root,
                    path,
                    thread_grant,
                    thread_current,
                    thread_stop,
                    receiver,
                );
                // Drop native descriptors and release capacity before confirming.
                drop(_permit);
                thread_cleaned.store(true, Ordering::Release);
                let _ = finished.send(());
            })
            .context("starting observer thread")?;
        // Construct outside the future: runtime shutdown can cancel it before
        // its first poll. That must still stop the native collection loop.
        let cancellation = Cancellation {
            requests: requests.clone(),
            stop: Arc::clone(&stop),
        };
        let watched_grant = Arc::clone(&grant);
        runtime.spawn(async move {
            let _cancellation = cancellation;
            tokio::select! {
                () = watched_grant.invalidated() => {},
                _ = dropped => {},
            }
        });
        Ok(Self {
            grant,
            current,
            requests,
            stop,
            owner: Some(owner),
            finished: cleaned,
            ended: Some(ended),
        })
    }

    pub async fn read(&self, cursor: Option<Cursor>) -> Result<Page> {
        ensure!(self.is_current(), "observer continuation ended");
        ensure!(
            cursor.as_ref().is_none_or(|cursor| {
                cursor.instance.len() == 32
                    && cursor.instance.bytes().all(|b| b.is_ascii_hexdigit())
                    && !cursor.position.is_empty()
                    && cursor.position.len() <= 20
                    && cursor.position.bytes().all(|b| b.is_ascii_digit())
            }),
            "invalid observer cursor"
        );
        let (send, receive) = oneshot::channel();
        self.requests
            .try_send(Request::Read(cursor, send))
            .map_err(|_| anyhow::anyhow!("observer request unavailable"))?;
        let page = tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .context("observer request timed out")?
            .context("observer stopped")??;
        ensure!(self.is_current(), "observer continuation ended");
        Ok(page)
    }

    pub fn is_current(&self) -> bool {
        !self.finished()
            && !self.stop.load(Ordering::Acquire)
            && self.grant.current()
            && (self.current)()
    }

    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.try_send(Request::Wake);
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Unlike dropping an async task, success confirms native collection and
    /// descriptor cleanup finished; it is not merely cancellation acceptance.
    pub async fn close(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.try_send(Request::Wake);
        self.owner.take();
        self.ended
            .take()
            .expect("one worker completion")
            .await
            .context("observer thread did not confirm exit")
    }
}

fn run(
    root: File,
    path: PathBuf,
    grant: Arc<LiveGrant>,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
    stop: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Request>,
) {
    let Ok(mut journal) = Journal::new() else {
        return;
    };
    let mut source: Option<Source> = None;
    let mut reconcile_at = Instant::now();
    let mut reconcile_requested = true;
    loop {
        if stop.load(Ordering::Acquire) || !grant.current() || !current() {
            stop.store(true, Ordering::Release);
            return;
        }
        if source.is_none() && reconcile_requested && Instant::now() >= reconcile_at {
            reconcile_requested = false;
            source = root
                .try_clone()
                .ok()
                .and_then(|root| Source::new(root, path.clone(), &mut journal).ok());
            reconcile_at = Instant::now() + RECONCILE;
        }
        if let Some(active) = source.as_mut()
            && active.drain(&mut journal).is_err()
        {
            source = None;
            reconcile_requested = true;
        }
        match receiver.recv_timeout(TICK) {
            Ok(Request::Read(cursor, response)) => {
                // Drain after the request arrives, never return the previous
                // tick as the requested event frontier.
                if let Some(active) = source.as_mut()
                    && active.drain(&mut journal).is_err()
                {
                    source = None;
                }
                // Failed scans retry only for new demand, at a bounded rate.
                // An unsupported idle root must not rescan indefinitely.
                if source.is_none() {
                    reconcile_requested = true;
                }
                if stop.load(Ordering::Acquire) || !grant.current() || !current() {
                    stop.store(true, Ordering::Release);
                    return;
                }
                let _ = response.send(journal.read(cursor.as_ref()));
            }
            Ok(Request::Wake) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests;
