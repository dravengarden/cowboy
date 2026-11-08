//! `cowboy-browser-control-v1`: newline-delimited JSON over the instance's
//! control socket. Client lines are commands carrying an `id`; launcher lines
//! are replies (`{"id", "ok", ...}`) or events (`{"event", ...}`). Framing
//! and shapes live here so the init only decides what to do; see
//! `plugins/machine-browser/PROTOCOL.md`.

use serde_json::{Map, Value, json};

pub const PROTOCOL: &str = "cowboy-browser-control-v1";
/// Longest line accepted or produced; matches the Machine's bound.
pub const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_ATTACHMENT_ID_BYTES: usize = 128;
const MAX_GRACEFUL_MS: u64 = 600_000;

/// The `--probe` output.
#[must_use]
pub fn probe_line() -> String {
    json!({ "protocol": PROTOCOL, "version": env!("CARGO_PKG_VERSION") }).to_string()
}

/// Failure class the Machine maps to a Controller reason: `sandbox` for
/// namespace, mount and `AppArmor` transition failures, `host` for missing
/// host prerequisites, `internal` for everything else (including malformed
/// commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Sandbox,
    Host,
    Internal,
}

impl FailureKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Host => "host",
            Self::Internal => "internal",
        }
    }

    /// Exit status of a launcher that failed before or after readiness.
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Internal => 1,
            Self::Host => 2,
            Self::Sandbox => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Navigate {
        url: String,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Attach {
        attachment_id: String,
    },
    Detach {
        attachment_id: String,
    },
    Snapshot {
        generation: u64,
    },
    Status,
    Stop {
        graceful_ms: u64,
        snapshot_generation: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: Value,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Request(Request),
    /// A line that is not a valid command; `id` is echoed when it had one.
    Invalid {
        id: Option<Value>,
        message: String,
    },
}

/// Decode one client line.
#[must_use]
pub fn parse_request(line: &[u8]) -> Parsed {
    let value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(error) => {
            return Parsed::Invalid {
                id: None,
                message: format!("invalid JSON: {error}"),
            };
        }
    };
    let Some(object) = value.as_object() else {
        return Parsed::Invalid {
            id: None,
            message: "command must be a JSON object".to_owned(),
        };
    };
    let id = object.get("id").cloned();
    let Some(id) = id.filter(|id| id.is_u64() || id.as_str().is_some_and(|id| id.len() <= 128))
    else {
        return Parsed::Invalid {
            id: None,
            message: "command needs an integer or short string id".to_owned(),
        };
    };
    match decode(object) {
        Ok(command) => Parsed::Request(Request { id, command }),
        Err(message) => Parsed::Invalid {
            id: Some(id),
            message,
        },
    }
}

fn decode(object: &Map<String, Value>) -> Result<Command, String> {
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "command name missing".to_owned())?;
    let string = |key: &str, limit: usize| -> Result<String, String> {
        let value = object
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{key} must be a string"))?;
        if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
            return Err(format!(
                "{key} is empty, too long or contains control characters"
            ));
        }
        Ok(value.to_owned())
    };
    let integer = |key: &str| -> Result<u64, String> {
        object
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{key} must be a non-negative integer"))
    };
    match command {
        "navigate" => {
            let url = string("url", MAX_URL_BYTES)?;
            validate_url(&url)?;
            Ok(Command::Navigate { url })
        }
        "resize" => {
            let width = u32::try_from(integer("width")?).map_err(|_| "width is too large")?;
            let height = u32::try_from(integer("height")?).map_err(|_| "height is too large")?;
            if !crate::args::geometry_allowed(width, height) {
                return Err(format!(
                    "geometry {width}x{height} is outside {}x{}..{}x{}",
                    crate::args::MIN_WIDTH,
                    crate::args::MIN_HEIGHT,
                    crate::args::MAX_WIDTH,
                    crate::args::MAX_HEIGHT
                ));
            }
            Ok(Command::Resize { width, height })
        }
        "attach" => Ok(Command::Attach {
            attachment_id: string("attachment_id", MAX_ATTACHMENT_ID_BYTES)?,
        }),
        "detach" => Ok(Command::Detach {
            attachment_id: string("attachment_id", MAX_ATTACHMENT_ID_BYTES)?,
        }),
        "snapshot" => Ok(Command::Snapshot {
            generation: integer("generation")?,
        }),
        "status" => Ok(Command::Status),
        "stop" => {
            let graceful_ms = integer("graceful_ms")?.min(MAX_GRACEFUL_MS);
            let snapshot_generation = match object.get("snapshot_generation") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_u64()
                        .ok_or("snapshot_generation must be a non-negative integer")?,
                ),
            };
            Ok(Command::Stop {
                graceful_ms,
                snapshot_generation,
            })
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

/// Only web URLs reach the browser, and only ones the X toolkit and Chromium
/// can take verbatim.
pub fn validate_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL_BYTES {
        return Err("url is too long".to_owned());
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "url must be http:// or https://".to_owned())?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.starts_with('-') {
        return Err("url has no host".to_owned());
    }
    if url
        .bytes()
        .any(|byte| byte <= b' ' || byte >= 0x7f || byte == b'"' || byte == b'\\')
    {
        return Err("url contains whitespace, control or non-ASCII bytes".to_owned());
    }
    Ok(())
}

/// Accumulates socket bytes and yields complete lines.
#[derive(Debug, Default)]
pub struct LineBuffer {
    buffer: Vec<u8>,
}

impl LineBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `bytes` and return every complete line (without the newline).
    ///
    /// # Errors
    /// Returns when a line exceeds [`MAX_LINE_BYTES`]; the connection must be
    /// dropped because framing is lost.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.buffer.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=position).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if !line.iter().all(u8::is_ascii_whitespace) {
                lines.push(line);
            }
        }
        if self.buffer.len() > MAX_LINE_BYTES {
            return Err("control line exceeds the bound".to_owned());
        }
        Ok(lines)
    }
}

fn line(value: &Value) -> String {
    let mut text = value.to_string();
    text.push('\n');
    text
}

/// `{"id":..,"ok":true,...}`.
#[must_use]
pub fn reply_ok(id: &Value, fields: Map<String, Value>) -> String {
    let mut object = Map::new();
    object.insert("id".to_owned(), id.clone());
    object.insert("ok".to_owned(), Value::Bool(true));
    object.extend(fields);
    line(&Value::Object(object))
}

/// `{"id":..,"ok":false,"error":..,"kind":..}`.
#[must_use]
pub fn reply_error(id: &Value, kind: FailureKind, message: &str) -> String {
    line(&json!({ "id": id, "ok": false, "error": message, "kind": kind.as_str() }))
}

#[must_use]
pub fn event_error(kind: FailureKind, message: &str) -> String {
    line(&json!({ "event": "error", "kind": kind.as_str(), "message": message }))
}

#[must_use]
pub fn event_exited(process: &str, code: Option<i32>) -> String {
    line(&json!({ "event": "exited", "process": process, "code": code }))
}

#[must_use]
pub fn event_snapshot_done(generation: u64, bytes: u64, entries: u64) -> String {
    line(
        &json!({ "event": "snapshot_done", "generation": generation, "bytes": bytes, "entries": entries }),
    )
}

/// A `ready` or `status` body from the report fields.
#[must_use]
pub fn report_line(
    event: Option<&str>,
    id: Option<&Value>,
    mut report: Map<String, Value>,
) -> String {
    let mut object = Map::new();
    if let Some(event) = event {
        object.insert("event".to_owned(), Value::String(event.to_owned()));
    }
    if let Some(id) = id {
        object.insert("id".to_owned(), id.clone());
        object.insert("ok".to_owned(), Value::Bool(true));
    }
    object.append(&mut report);
    line(&Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_decode_with_their_bounds() {
        let parsed =
            parse_request(br#"{"id":4,"command":"navigate","url":"https://example.com/a?b=c"}"#);
        assert_eq!(
            parsed,
            Parsed::Request(Request {
                id: json!(4),
                command: Command::Navigate {
                    url: "https://example.com/a?b=c".to_owned()
                }
            })
        );
        assert!(matches!(
            parse_request(br#"{"id":5,"command":"navigate","url":"file:///etc/passwd"}"#),
            Parsed::Invalid { id: Some(_), .. }
        ));
        assert!(matches!(
            parse_request(br#"{"id":6,"command":"resize","width":100,"height":800}"#),
            Parsed::Invalid { .. }
        ));
        assert_eq!(
            parse_request(br#"{"id":"r-1","command":"resize","width":1024,"height":700}"#),
            Parsed::Request(Request {
                id: json!("r-1"),
                command: Command::Resize {
                    width: 1024,
                    height: 700
                }
            })
        );
        assert_eq!(
            parse_request(br#"{"id":7,"command":"stop","graceful_ms":5000}"#),
            Parsed::Request(Request {
                id: json!(7),
                command: Command::Stop {
                    graceful_ms: 5000,
                    snapshot_generation: None
                }
            })
        );
        assert_eq!(
            parse_request(
                br#"{"id":8,"command":"stop","graceful_ms":99999999,"snapshot_generation":3}"#
            ),
            Parsed::Request(Request {
                id: json!(8),
                command: Command::Stop {
                    graceful_ms: MAX_GRACEFUL_MS,
                    snapshot_generation: Some(3)
                }
            })
        );
        assert_eq!(
            parse_request(br#"{"id":9,"command":"snapshot","generation":12}"#),
            Parsed::Request(Request {
                id: json!(9),
                command: Command::Snapshot { generation: 12 }
            })
        );
        assert_eq!(
            parse_request(br#"{"id":10,"command":"status"}"#),
            Parsed::Request(Request {
                id: json!(10),
                command: Command::Status
            })
        );
        assert!(matches!(
            parse_request(br#"{"id":11,"command":"attach","attachment_id":""}"#),
            Parsed::Invalid { .. }
        ));
        assert!(matches!(
            parse_request(br#"{"command":"status"}"#),
            Parsed::Invalid { id: None, .. }
        ));
        assert!(matches!(
            parse_request(b"[1]"),
            Parsed::Invalid { id: None, .. }
        ));
        assert!(matches!(
            parse_request(b"{"),
            Parsed::Invalid { id: None, .. }
        ));
        assert!(matches!(
            parse_request(br#"{"id":12,"command":"explode"}"#),
            Parsed::Invalid { id: Some(_), .. }
        ));
    }

    #[test]
    fn urls_are_web_only_and_clean() {
        assert!(validate_url("https://claude.ai/oauth?x=1").is_ok());
        assert!(validate_url("http://127.0.0.1:8080/").is_ok());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("https://").is_err());
        assert!(validate_url("https://exa mple.com").is_err());
        assert!(validate_url("https://example.com/\u{e9}").is_err());
        assert!(validate_url(&format!("https://example.com/{}", "a".repeat(2100))).is_err());
    }

    #[test]
    fn framing_splits_lines_and_bounds_them() {
        let mut buffer = LineBuffer::new();
        assert_eq!(
            buffer.push(b"{\"a\":1}\n\n{\"b\"").unwrap(),
            vec![b"{\"a\":1}".to_vec()]
        );
        assert_eq!(
            buffer.push(b":2}\r\n").unwrap(),
            vec![b"{\"b\":2}".to_vec()]
        );
        assert!(buffer.push(&vec![b'x'; MAX_LINE_BYTES + 1]).is_err());
    }

    #[test]
    fn replies_and_events_have_the_documented_shape() {
        let parsed = |line: String| -> Value {
            assert!(line.ends_with('\n'), "{line:?} lacks its newline");
            serde_json::from_str(&line).unwrap()
        };
        let mut fields = Map::new();
        fields.insert("graceful".to_owned(), json!(true));
        assert_eq!(
            parsed(reply_ok(&json!(3), fields)),
            json!({"id": 3, "ok": true, "graceful": true})
        );
        assert_eq!(
            parsed(reply_error(&json!(3), FailureKind::Host, "no")),
            json!({"id": 3, "ok": false, "error": "no", "kind": "host"})
        );
        assert_eq!(
            parsed(event_exited("browser", Some(1))),
            json!({"event": "exited", "process": "browser", "code": 1})
        );
        assert_eq!(
            parsed(event_snapshot_done(2, 10, 3)),
            json!({"event": "snapshot_done", "generation": 2, "bytes": 10, "entries": 3})
        );
        assert_eq!(
            parsed(event_error(FailureKind::Sandbox, "mount denied")),
            json!({"event": "error", "kind": "sandbox", "message": "mount denied"})
        );
        let mut report = Map::new();
        report.insert("uid_map".to_owned(), json!("1000 1000 1"));
        assert_eq!(
            parsed(report_line(Some("ready"), None, report.clone())),
            json!({"event": "ready", "uid_map": "1000 1000 1"})
        );
        assert_eq!(
            parsed(report_line(None, Some(&json!(1)), report)),
            json!({"id": 1, "ok": true, "uid_map": "1000 1000 1"})
        );
        let probe: Value = serde_json::from_str(&probe_line()).unwrap();
        assert_eq!(probe["protocol"], "cowboy-browser-control-v1");
        assert_eq!(probe["version"], env!("CARGO_PKG_VERSION"));
    }
}
