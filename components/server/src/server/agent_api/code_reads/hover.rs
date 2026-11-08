//! Pull-only hover over an already-open buffer. The caller pins its locator
//! to exact content; native revision checks and enclosing snapshots reject
//! edits, replacement and late policy/route changes without opening anything.
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::super::code_reads::{language, session};
use super::super::super::{
    AppState, BufferSnapshot, CodeHoverBlock, SourceState, ZedAdapterResponse,
};
use crate::machine_control::SessionReadScope;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Locator {
    path: String,
    row: u32,
    column: u32,
    sha256: String,
}

impl Locator {
    pub(super) fn parse(args: &Value) -> Result<Self, String> {
        let value: Self = serde_json::from_value(args.clone()).map_err(|_| {
            "invalid_argument: hover requires path, row, column and sha256".to_owned()
        })?;
        let path = std::path::Path::new(&value.path);
        if value.path.is_empty()
            || value.path.len() > 4096
            || value.path.contains(['\0', '\\'])
            || value.path.chars().any(char::is_control)
            || !path
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_)))
            || path.components().collect::<std::path::PathBuf>().to_str() != Some(&value.path)
            || value.row > 4 * 1024 * 1024
            || value.column > 4 * 1024 * 1024
            || value.sha256.len() != 64
            || !value
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid_argument: hover needs a relative file, bounded UTF-16 position and lowercase SHA-256".into());
        }
        Ok(value)
    }
}

pub(super) async fn read(state: &AppState, scope: &SessionReadScope, locator: Locator) -> Value {
    let trust = state
        .hub
        .observe_language_server_trust(scope.session().session_id())
        .filter(|trust| trust.scope() == scope.session());
    let observation = observed_hover(
        &locator,
        || {
            trust
                .as_ref()
                .is_some_and(|trust| trust.current(&state.hub))
                && session::current(&state.hub, &state.machine_control, scope)
        },
        |query| language::query_open_buffer(state, scope, &locator.path, query),
    );
    // Bound this optional read, without waiting for analysis or retaining a
    // lease. Cancelling the observer never authorizes a retry of an effect.
    tokio::time::timeout(std::time::Duration::from_secs(5), observation)
        .await
        .unwrap_or_else(|_| json!({"path":locator.path,"state":"unavailable"}))
}

fn snapshot(reply: ZedAdapterResponse, hash: &str) -> Result<Box<BufferSnapshot>, &'static str> {
    match reply {
        ZedAdapterResponse::BufferLanguage {
            buffer_snapshot: Some(snapshot),
            source_state,
            ..
        } if snapshot.bounded() => {
            if source_state == SourceState::Changed || !snapshot.matches_content(hash) {
                Err("stale")
            } else if source_state != SourceState::Matching {
                Err("unavailable")
            } else {
                Ok(snapshot)
            }
        }
        _ => Err("unavailable"),
    }
}

async fn observed_hover<F, Fut>(
    locator: &Locator,
    current: impl Fn() -> bool,
    mut query: F,
) -> Value
where
    F: FnMut(language::Query) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<ZedAdapterResponse>>,
{
    let refused = |state| json!({"path":locator.path,"state":state});
    if !current() {
        return refused("restricted");
    }
    let before = query(language::Query::Language).await;
    if !current() {
        return refused("restricted");
    }
    let before = match before {
        Ok(value) => match snapshot(value, &locator.sha256) {
            Ok(snapshot) => snapshot,
            Err(reason) => return refused(reason),
        },
        Err(error) => {
            return refused(if error.to_string().contains("not open") {
                "not_open"
            } else {
                "unavailable"
            });
        }
    };
    let hover = query(language::Query::Hover {
        row: locator.row,
        column: locator.column,
    })
    .await;
    if !current() {
        return refused("restricted");
    }
    let Ok(ZedAdapterResponse::BufferHover { contents, .. }) = hover else {
        return refused("unavailable");
    };
    let after = query(language::Query::Language).await;
    if !current() {
        return refused("restricted");
    }
    let after = match after {
        Ok(value) => match snapshot(value, &locator.sha256) {
            Ok(snapshot) => snapshot,
            Err(reason) => return refused(reason),
        },
        Err(_) => return refused("unavailable"),
    };
    if before != after {
        return refused("stale");
    }
    let (contents, truncated) = bounded_contents(contents);
    let reply = bounded_reply(
        json!({"path":locator.path,"state":"complete","precision":"language_server",
        "bufferSnapshot":after,"contents":contents,"truncated":truncated}),
    );
    if current() {
        reply
    } else {
        refused("restricted")
    }
}

fn bounded_reply(reply: Value) -> Value {
    // JSON escaping, language labels, long paths and the version vector count
    // too. Keep headroom below the MCP result ceiling, not just a text bound.
    if serde_json::to_vec(&reply).is_ok_and(|bytes| bytes.len() <= 12 * 1024) {
        reply
    } else {
        json!({"path":reply["path"],"state":"unavailable"})
    }
}

fn bounded_contents(contents: Vec<CodeHoverBlock>) -> (Vec<Value>, bool) {
    let mut budget = 2000;
    let mut truncated = contents.len() > 8;
    let mut result = Vec::new();
    for block in contents.into_iter().take(8) {
        let text: String = block.text.chars().take(budget).collect();
        truncated |= text.len() < block.text.len();
        budget -= text.chars().count();
        let language = block.language.map(|value| {
            let limited: String = value.chars().take(64).collect();
            truncated |= limited.len() < value.len();
            limited
        });
        result.push(json!({"text":text,"language":language,"markdown":block.markdown}));
    }
    (result, truncated)
}

#[cfg(test)]
mod tests;
