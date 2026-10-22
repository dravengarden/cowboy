//! Bounded, Controller-local bindings for existing file-page continuations.
//!
//! Browser tokens are never native adapter cursors or authorization. Resolve a
//! token against the current context and exact requested path before any I/O;
//! the outer code-reader boundary still rechecks the context after the response.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};

use crate::code_review::{FILE_PAGE_BYTES, FileDocument, MAX_FILE_BYTES, parse_file_cursor};
use crate::core::CodeReadScope;
use crate::server::{
    Body, CodeFileResponse, HeaderMap, IntoResponse as _, Response, StatusCode, header,
};

const MAX_ENTRIES: usize = 512;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_CONTROL: &str = "private, max-age=0, must-revalidate";

#[derive(Clone, Debug)]
struct NativeCursor {
    wire: String,
    revision: String,
    offset: usize,
}

impl NativeCursor {
    fn parse(wire: &str) -> Result<Self, CursorError> {
        if wire.len() > 86 {
            return Err(CursorError::Invalid);
        }
        let (revision, offset) = parse_file_cursor(wire).map_err(|_| CursorError::Invalid)?;
        if offset == 0 || offset >= MAX_FILE_BYTES || wire != format!("{revision}:{offset}") {
            return Err(CursorError::Invalid);
        }
        Ok(Self {
            wire: wire.to_owned(),
            revision: revision.to_owned(),
            offset,
        })
    }
}

#[derive(Clone, Debug)]
pub(in crate::server) struct Continuation {
    owner: CodeReadScope,
    path: String,
    native: NativeCursor,
}

impl Continuation {
    pub(in crate::server) fn native_cursor(&self) -> &str {
        &self.native.wire
    }
}

struct Entry {
    public: String,
    continuation: Continuation,
    touched: Instant,
    string_bytes: usize,
}

pub(in crate::server) struct PageCursors {
    entries: Mutex<VecDeque<Entry>>,
    max_entries: usize,
    max_string_bytes: usize,
    ttl: Duration,
}

impl Default for PageCursors {
    fn default() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            max_entries: MAX_ENTRIES,
            max_string_bytes: MAX_STRING_BYTES,
            ttl: TTL,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::server) enum CursorError {
    Invalid,
    Expired,
    Changed,
    InvalidPage,
    Capacity,
}

impl axum::response::IntoResponse for CursorError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid file cursor"),
            Self::Expired => (StatusCode::GONE, "file snapshot expired"),
            Self::Changed => (StatusCode::CONFLICT, "file snapshot changed"),
            Self::InvalidPage => (StatusCode::BAD_GATEWAY, "invalid file page"),
            Self::Capacity => (
                StatusCode::SERVICE_UNAVAILABLE,
                "file cursor capacity exceeded",
            ),
        };
        (status, [(header::CACHE_CONTROL, "no-store")], message).into_response()
    }
}

impl PageCursors {
    pub(in crate::server) fn resolve(
        &self,
        owner: &CodeReadScope,
        path: &str,
        public: Option<&str>,
    ) -> Result<Option<Continuation>, CursorError> {
        let Some(public) = public else {
            return Ok(None);
        };
        NativeCursor::parse(public)?;
        let now = Instant::now();
        let mut entries = self.entries.lock();
        self.prune(&mut entries, now);
        let index = entries
            .iter()
            .position(|entry| {
                entry.public == public
                    && entry.continuation.owner == *owner
                    && entry.continuation.path == path
            })
            .ok_or(CursorError::Expired)?;
        let mut entry = entries.remove(index).expect("located cursor entry");
        entry.touched = now;
        let continuation = entry.continuation.clone();
        entries.push_back(entry);
        Ok(Some(continuation))
    }

    pub(in crate::server) fn project(
        &self,
        owner: &CodeReadScope,
        path: &str,
        previous: Option<&Continuation>,
        mut page: FileDocument,
    ) -> Result<FileDocument, CursorError> {
        let offset = match previous {
            Some(previous) => {
                if previous.owner != *owner || previous.path != path {
                    return Err(CursorError::Expired);
                }
                if previous.native.revision != page.revision {
                    return Err(CursorError::Changed);
                }
                previous.native.offset
            }
            None => 0,
        };
        let next_offset = offset
            .checked_add(page.text.len())
            .ok_or(CursorError::InvalidPage)?;
        let limited = page.size > MAX_FILE_BYTES as u64;
        let complete = if limited {
            (MAX_FILE_BYTES - 3..=MAX_FILE_BYTES).contains(&next_offset)
        } else {
            next_offset as u64 == page.size
        };
        if page.text.len() > FILE_PAGE_BYTES
            || next_offset as u64 > page.size
            || page.revision.len() != 64
            || !page.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            || page.limited != limited
            || (page.next_cursor.is_none() && !complete)
            || page.truncated != (page.next_cursor.is_some() || limited)
        {
            return Err(CursorError::InvalidPage);
        }
        if let Some(wire) = page.next_cursor.take() {
            let native = NativeCursor::parse(&wire).map_err(|_| CursorError::InvalidPage)?;
            if native.revision != page.revision
                || native.offset != next_offset
                || page.text.is_empty()
                || native.offset as u64 >= page.size
            {
                return Err(CursorError::InvalidPage);
            }
            page.next_cursor = Some(self.issue(owner, path, native)?);
        }
        Ok(page)
    }

    fn issue(
        &self,
        owner: &CodeReadScope,
        path: &str,
        native: NativeCursor,
    ) -> Result<String, CursorError> {
        // Bound variable identity bytes as well as the fixed number of entries.
        let string_bytes =
            owner.string_bytes() + path.len() + native.wire.len() + native.revision.len() + 86;
        if self.max_entries == 0 || string_bytes > self.max_string_bytes {
            return Err(CursorError::Capacity);
        }
        let now = Instant::now();
        let mut entries = self.entries.lock();
        self.prune(&mut entries, now);
        if let Some(index) = entries.iter().position(|entry| {
            entry.continuation.owner == *owner
                && entry.continuation.path == path
                && entry.continuation.native.wire == native.wire
        }) {
            let mut entry = entries.remove(index).expect("located cursor entry");
            entry.touched = now;
            let public = entry.public.clone();
            entries.push_back(entry);
            return Ok(public);
        }
        let mut retained: usize = entries.iter().map(|entry| entry.string_bytes).sum();
        while entries.len() >= self.max_entries || retained + string_bytes > self.max_string_bytes {
            let entry = entries.pop_front().expect("bounded nonempty cursor cache");
            retained -= entry.string_bytes;
        }
        let public = format!(
            "{:x}:{}",
            Sha256::digest(rand::random::<[u8; 32]>()),
            native.offset
        );
        entries.push_back(Entry {
            public: public.clone(),
            continuation: Continuation {
                owner: owner.clone(),
                path: path.to_owned(),
                native,
            },
            touched: now,
            string_bytes,
        });
        Ok(public)
    }

    fn prune(&self, entries: &mut VecDeque<Entry>, now: Instant) {
        entries.retain(|entry| now.saturating_duration_since(entry.touched) < self.ttl);
    }
}

pub(in crate::server) fn response(headers: &HeaderMap, page: FileDocument) -> Response {
    let bytes = serde_json::to_vec(&CodeFileResponse {
        api_version: 1,
        path: page.path,
        revision: page.revision,
        text: page.text,
        size: page.size,
        truncated: page.truncated,
        next_cursor: page.next_cursor,
        limited: page.limited,
    })
    .expect("file page response serializes");
    // A file revision is not a page identity. Include the exact representation,
    // notably its continuation binding, so expiry/restart cannot 304 an old token.
    let etag = format!("\"file-page-v2-{:x}\"", Sha256::digest(&bytes));
    let not_modified = headers.get_all(header::IF_NONE_MATCH).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value.split(',').map(str::trim).any(|candidate| {
                candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
            })
        })
    });
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(header::CONTENT_TYPE, "application/json")],
            Body::from(bytes),
        )
            .into_response()
    };
    response
        .headers_mut()
        .insert(header::ETAG, etag.parse().expect("SHA256 ETag is valid"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(CACHE_CONTROL),
    );
    response
}

#[cfg(test)]
mod tests;
