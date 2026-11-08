//! Cowboy Widgets document rules on the Controller (docs/widgets.md §5.3,
//! contract `contracts/widget-v1.ts`). The Controller fails closed: an unknown
//! node type, a shape error or an exceeded bound refuses the write with a path
//! the agent can repair. The Web mirrors these rules with its own validator.
use serde_json::{Map, Value, json};
use std::collections::HashSet;

pub const SCHEMA: &str = "cowboy.widget/1";
pub const DOCUMENT_BYTES: usize = 24 * 1024;
const NODES: usize = 400;
const NODE_DEPTH: usize = 12;
const JSON_DEPTH: usize = 32;
const ID_CHARS: usize = 64;
const TITLE_CHARS: usize = 120;
const SUMMARY_CHARS: usize = 200;
const LABEL_CHARS: usize = 200;
const TEXT_CHARS: usize = 8 * 1024;
const FALLBACK_CHARS: usize = 2 * 1024;
const TABLE_ROWS: usize = 200;
const TABLE_COLUMNS: usize = 12;
const CELLS: usize = 2000;
const ITEMS: usize = 1000;
const IMAGES: usize = 24;
const LIST_ITEMS: usize = 200;
const KV_ITEMS: usize = 100;
const STEPS: usize = 50;
const OPTIONS: usize = 50;
const CARD_ACTIONS: usize = 6;
const GALLERY: usize = 24;
pub const PATCH_OPS: usize = 64;
const DESCRIPTION_CHARS: usize = 2048;

const TONES: &[&str] = &["neutral", "info", "success", "warning", "error", "running"];

/// One refusal, addressed by the JSON path the agent wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub path: String,
    pub message: String,
}

impl Invalid {
    fn at(path: &str, message: &str) -> Self {
        Self {
            path: path.to_owned(),
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"path": self.path, "message": self.message})
    }
}

type Checked = Result<(), Invalid>;

#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Unicode format characters (`Cf`) that can make a label read differently
/// from what it says: bidirectional controls, zero-width and invisible ones.
fn format_char(c: char) -> bool {
    matches!(u32::from(c),
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF
        | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

fn chars(value: &str) -> usize {
    value.chars().count()
}

fn text(value: Option<&Value>, path: &str, max: usize, optional: bool) -> Checked {
    match value {
        None if optional => Ok(()),
        Some(Value::String(s)) if chars(s) <= max => Ok(()),
        Some(Value::String(_)) => Err(Invalid::at(path, &format!("longer than {max} characters"))),
        _ => Err(Invalid::at(path, "expected text")),
    }
}

fn line(value: Option<&Value>, path: &str, max: usize, optional: bool) -> Checked {
    text(value, path, max, optional)?;
    if let Some(Value::String(s)) = value
        && (s.trim().is_empty()
            || s.chars()
                .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}' || format_char(c)))
    {
        return Err(Invalid::at(path, "must be one line of printable text"));
    }
    Ok(())
}

fn array<'v>(value: Option<&'v Value>, path: &str, max: usize) -> Result<&'v Vec<Value>, Invalid> {
    match value {
        Some(Value::Array(items)) if items.len() <= max => Ok(items),
        Some(Value::Array(_)) => Err(Invalid::at(path, &format!("more than {max} entries"))),
        _ => Err(Invalid::at(path, "expected a list")),
    }
}

fn object<'v>(value: &'v Value, path: &str) -> Result<&'v Map<String, Value>, Invalid> {
    value
        .as_object()
        .ok_or_else(|| Invalid::at(path, "expected an object"))
}

fn tone(value: Option<&Value>, path: &str) -> Checked {
    match value {
        None => Ok(()),
        Some(Value::String(s)) if TONES.contains(&s.as_str()) => Ok(()),
        _ => Err(Invalid::at(path, "unknown tone")),
    }
}

fn one_of(value: Option<&Value>, path: &str, allowed: &[&str], optional: bool) -> Checked {
    match value {
        None if optional => Ok(()),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(()),
        _ => Err(Invalid::at(
            path,
            &format!("expected one of {}", allowed.join(", ")),
        )),
    }
}

fn scalar(value: &Value, path: &str) -> Checked {
    match value {
        Value::Null | Value::Bool(_) => Ok(()),
        Value::Number(n)
            if n.as_f64()
                .is_some_and(|f| f.is_finite() && f.abs() <= 9_007_199_254_740_992.0) =>
        {
            Ok(())
        }
        Value::String(s) if chars(s) <= LABEL_CHARS * 4 => Ok(()),
        _ => Err(Invalid::at(path, "expected a short scalar")),
    }
}

fn reference(value: Option<&Value>, path: &str) -> Checked {
    let Some(value) = value else { return Ok(()) };
    let map = object(value, path)?;
    match map.get("kind").and_then(Value::as_str) {
        Some("url") => match map.get("url").and_then(Value::as_str) {
            Some(url) if url.starts_with("https://") && url.len() <= 2048 => Ok(()),
            _ => Err(Invalid::at(
                &format!("{path}.url"),
                "only https: links are allowed",
            )),
        },
        Some("session" | "file" | "document" | "call" | "artifact") => Ok(()),
        _ => Err(Invalid::at(
            &format!("{path}.kind"),
            "unknown reference kind",
        )),
    }
}

#[derive(Default)]
struct Budget {
    nodes: usize,
    cells: usize,
    items: usize,
    images: usize,
    ids: HashSet<String>,
}

fn cell(value: &Value, path: &str) -> Checked {
    match value {
        Value::Object(map) => {
            scalar(map.get("v").unwrap_or(&Value::Null), &format!("{path}.v"))?;
            tone(map.get("tone"), &format!("{path}.tone"))?;
            reference(map.get("to"), &format!("{path}.to"))
        }
        other => scalar(other, path),
    }
}

fn list_items(value: Option<&Value>, path: &str, depth: usize) -> Result<usize, Invalid> {
    if depth > NODE_DEPTH {
        return Err(Invalid::at(path, "nested too deeply"));
    }
    let mut count = 0;
    for (index, item) in array(value, path, LIST_ITEMS)?.iter().enumerate() {
        let at = format!("{path}[{index}]");
        let map = object(item, &at)?;
        text(map.get("text"), &format!("{at}.text"), TEXT_CHARS, false)?;
        tone(map.get("tone"), &format!("{at}.tone"))?;
        reference(map.get("to"), &format!("{at}.to"))?;
        count += 1;
        if map.contains_key("children") {
            count += list_items(map.get("children"), &format!("{at}.children"), depth + 1)?;
        }
    }
    Ok(count)
}

fn image_source(value: Option<&Value>, path: &str) -> Checked {
    let map = object(value.ok_or_else(|| Invalid::at(path, "required"))?, path)?;
    one_of(
        map.get("kind"),
        &format!("{path}.kind"),
        &["artifact", "event", "data"],
        false,
    )
}

fn table(node: &Map<String, Value>, path: &str, budget: &mut Budget) -> Checked {
    let field = |name: &str| format!("{path}.{name}");
    let columns = array(node.get("columns"), &field("columns"), TABLE_COLUMNS)?;
    for (index, column) in columns.iter().enumerate() {
        let at = format!("{path}.columns[{index}]");
        match column {
            Value::String(_) => text(Some(column), &at, LABEL_CHARS, false)?,
            _ => {
                let map = object(column, &at)?;
                text(map.get("label"), &format!("{at}.label"), LABEL_CHARS, false)?;
                one_of(
                    map.get("align"),
                    &format!("{at}.align"),
                    &["start", "center", "end"],
                    true,
                )?;
                one_of(
                    map.get("format"),
                    &format!("{at}.format"),
                    &[
                        "text", "number", "percent", "bytes", "duration", "date", "code", "status",
                    ],
                    true,
                )?;
                one_of(
                    map.get("width"),
                    &format!("{at}.width"),
                    &["auto", "min", "max"],
                    true,
                )?;
            }
        }
    }
    let rows = array(node.get("rows"), &field("rows"), TABLE_ROWS)?;
    for (r, row) in rows.iter().enumerate() {
        let at = format!("{path}.rows[{r}]");
        let cells = array(Some(row), &at, TABLE_COLUMNS)?;
        if cells.len() != columns.len() {
            return Err(Invalid::at(&at, "row width differs from the columns"));
        }
        for (c, value) in cells.iter().enumerate() {
            cell(value, &format!("{at}[{c}]"))?;
        }
    }
    one_of(
        node.get("mobile"),
        &field("mobile"),
        &["auto", "cards", "fit"],
        true,
    )?;
    if node.get("sort").is_some_and(|sort| !sort.is_boolean()) {
        return Err(Invalid::at(&field("sort"), "expected a boolean"));
    }
    budget.cells += rows.len() * columns.len();
    Ok(())
}

fn choice(node: &Map<String, Value>, path: &str, budget: &mut Budget) -> Checked {
    let field = |name: &str| format!("{path}.{name}");
    if !node.get("id").and_then(Value::as_str).is_some_and(valid_id) {
        return Err(Invalid::at(&field("id"), "required on a choice"));
    }
    line(node.get("label"), &field("label"), LABEL_CHARS, false)?;
    let options = array(node.get("options"), &field("options"), OPTIONS)?;
    let mut values = HashSet::new();
    let mut labels = HashSet::new();
    for (index, option) in options.iter().enumerate() {
        let at = format!("{path}.options[{index}]");
        let map = object(option, &at)?;
        let value = map
            .get("value")
            .and_then(Value::as_str)
            .filter(|v| valid_id(v));
        if !value.is_some_and(|v| values.insert(v.to_owned())) {
            return Err(Invalid::at(
                &format!("{at}.value"),
                "expected a unique identifier",
            ));
        }
        line(map.get("label"), &format!("{at}.label"), LABEL_CHARS, false)?;
        if !labels.insert(map["label"].as_str().unwrap_or_default().to_owned()) {
            return Err(Invalid::at(
                &format!("{at}.label"),
                "duplicate option label",
            ));
        }
    }
    budget.items += options.len();
    Ok(())
}

fn button(node: &Map<String, Value>, path: &str) -> Checked {
    let field = |name: &str| format!("{path}.{name}");
    if !node.get("id").and_then(Value::as_str).is_some_and(valid_id) {
        return Err(Invalid::at(&field("id"), "required on a button"));
    }
    line(node.get("label"), &field("label"), LABEL_CHARS, false)?;
    let action = object(node.get("action").unwrap_or(&Value::Null), &field("action"))?;
    one_of(
        action.get("kind"),
        &field("action.kind"),
        &["prompt", "open", "copy"],
        false,
    )?;
    one_of(
        node.get("variant"),
        &field("variant"),
        &["primary", "secondary", "text", "destructive"],
        true,
    )
}

fn kv(node: &Map<String, Value>, path: &str, budget: &mut Budget) -> Checked {
    let field = |name: &str| format!("{path}.{name}");
    let items = array(node.get("items"), &field("items"), KV_ITEMS)?;
    for (index, item) in items.iter().enumerate() {
        let at = format!("{path}.items[{index}]");
        let map = object(item, &at)?;
        text(map.get("k"), &format!("{at}.k"), LABEL_CHARS, false)?;
        scalar(map.get("v").unwrap_or(&Value::Null), &format!("{at}.v"))?;
        tone(map.get("tone"), &format!("{at}.tone"))?;
        reference(map.get("to"), &format!("{at}.to"))?;
    }
    budget.items += items.len();
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn leaf(node: &Map<String, Value>, kind: &str, path: &str, budget: &mut Budget) -> Checked {
    let field = |name: &str| format!("{path}.{name}");
    match kind {
        "divider" => Ok(()),
        "text" => {
            text(node.get("text"), &field("text"), TEXT_CHARS, false)?;
            one_of(
                node.get("variant"),
                &field("variant"),
                &["title", "subtitle", "body", "caption"],
                true,
            )?;
            tone(node.get("tone"), &field("tone"))
        }
        "kv" => kv(node, path, budget),
        "list" => {
            let count = list_items(node.get("items"), &field("items"), 1)?;
            if count > LIST_ITEMS {
                return Err(Invalid::at(
                    &field("items"),
                    "more than 200 items including nested ones",
                ));
            }
            budget.items += count;
            Ok(())
        }
        "table" => table(node, path, budget),
        "link" => {
            line(node.get("label"), &field("label"), LABEL_CHARS, false)?;
            reference(
                Some(
                    node.get("to")
                        .ok_or_else(|| Invalid::at(&field("to"), "required"))?,
                ),
                &field("to"),
            )
        }
        "file" => text(node.get("path"), &field("path"), 1024, false),
        "status" => {
            line(node.get("label"), &field("label"), LABEL_CHARS, false)?;
            one_of(node.get("tone"), &field("tone"), TONES, false)
        }
        "progress" => {
            if let Some(value) = node.get("value")
                && !value.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v))
            {
                return Err(Invalid::at(
                    &field("value"),
                    "expected a number from 0 to 1",
                ));
            }
            one_of(
                node.get("variant"),
                &field("variant"),
                &["linear", "circular"],
                true,
            )?;
            tone(node.get("tone"), &field("tone"))
        }
        "steps" => {
            let steps = array(node.get("steps"), &field("steps"), STEPS)?;
            for (index, step) in steps.iter().enumerate() {
                let at = format!("{path}.steps[{index}]");
                let map = object(step, &at)?;
                text(map.get("label"), &format!("{at}.label"), LABEL_CHARS, false)?;
                one_of(
                    map.get("state"),
                    &format!("{at}.state"),
                    &["pending", "active", "done", "failed", "skipped"],
                    false,
                )?;
            }
            budget.items += steps.len();
            Ok(())
        }
        "alert" => {
            one_of(
                node.get("severity"),
                &field("severity"),
                &["info", "success", "warning", "error"],
                false,
            )?;
            text(node.get("text"), &field("text"), TEXT_CHARS, false)
        }
        "image" => {
            text(node.get("alt"), &field("alt"), LABEL_CHARS, false)?;
            image_source(node.get("src"), &field("src"))?;
            budget.images += 1;
            Ok(())
        }
        "gallery" => {
            let images = array(node.get("images"), &field("images"), GALLERY)?;
            for (index, image) in images.iter().enumerate() {
                let at = format!("{path}.images[{index}]");
                let map = object(image, &at)?;
                text(map.get("alt"), &format!("{at}.alt"), LABEL_CHARS, false)?;
                image_source(map.get("src"), &format!("{at}.src"))?;
            }
            budget.images += images.len();
            Ok(())
        }
        "button" => button(node, path),
        "choice" => choice(node, path, budget),
        _ => Err(Invalid::at(&field("type"), "unknown node type")),
    }
}

fn node(value: &Value, path: &str, depth: usize, budget: &mut Budget) -> Checked {
    let map = object(value, path)?;
    if depth > NODE_DEPTH {
        return Err(Invalid::at(path, "nested more than 12 nodes deep"));
    }
    budget.nodes += 1;
    if budget.nodes > NODES {
        return Err(Invalid::at(path, "more than 400 nodes"));
    }
    if let Some(id) = map.get("id") {
        match id.as_str() {
            Some(id) if valid_id(id) && budget.ids.insert(id.to_owned()) => {}
            _ => {
                return Err(Invalid::at(
                    &format!("{path}.id"),
                    "invalid or duplicate id",
                ));
            }
        }
    }
    text(
        map.get("fallback"),
        &format!("{path}.fallback"),
        FALLBACK_CHARS,
        true,
    )?;
    let kind = map.get("type").and_then(Value::as_str).unwrap_or_default();
    match kind {
        "stack" | "section" | "card" => {
            if kind == "section" {
                line(
                    map.get("title"),
                    &format!("{path}.title"),
                    TITLE_CHARS,
                    false,
                )?;
            }
            if kind == "stack" {
                one_of(
                    map.get("direction"),
                    &format!("{path}.direction"),
                    &["column", "row"],
                    true,
                )?;
            }
            if kind == "card" {
                line(
                    map.get("title"),
                    &format!("{path}.title"),
                    TITLE_CHARS,
                    true,
                )?;
                if let Some(actions) = map.get("actions") {
                    let actions = array(Some(actions), &format!("{path}.actions"), CARD_ACTIONS)?;
                    if !actions.is_empty() && !map.contains_key("id") {
                        return Err(Invalid::at(
                            &format!("{path}.id"),
                            "required on a card with actions",
                        ));
                    }
                    for (index, action) in actions.iter().enumerate() {
                        let at = format!("{path}.actions[{index}]");
                        if action.get("type").and_then(Value::as_str) != Some("button") {
                            return Err(Invalid::at(&at, "card actions must be buttons"));
                        }
                        node(action, &at, depth + 1, budget)?;
                    }
                }
            }
            for (index, child) in array(map.get("children"), &format!("{path}.children"), NODES)?
                .iter()
                .enumerate()
            {
                node(
                    child,
                    &format!("{path}.children[{index}]"),
                    depth + 1,
                    budget,
                )?;
            }
            Ok(())
        }
        _ => leaf(map, kind, path, budget),
    }
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// Every rule a stored document satisfies.
///
/// # Errors
/// The first violated rule, addressed by path.
pub fn validate(document: &Value) -> Result<(), Invalid> {
    let map = object(document, "document")?;
    if map.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(Invalid::at("schema", "expected cowboy.widget/1"));
    }
    if serde_json::to_vec(document).map_or(usize::MAX, |bytes| bytes.len()) > DOCUMENT_BYTES {
        return Err(Invalid::at("document", "larger than 24 KiB"));
    }
    if json_depth(document) > JSON_DEPTH {
        return Err(Invalid::at("document", "nested more than 32 values deep"));
    }
    line(map.get("title"), "title", TITLE_CHARS, false)?;
    line(map.get("summary"), "summary", SUMMARY_CHARS, true)?;
    let mut budget = Budget::default();
    node(
        map.get("root").unwrap_or(&Value::Null),
        "root",
        1,
        &mut budget,
    )?;
    if budget.cells > CELLS {
        return Err(Invalid::at("document", "more than 2000 table cells"));
    }
    if budget.items > ITEMS {
        return Err(Invalid::at("document", "more than 1000 items"));
    }
    if budget.images > IMAGES {
        return Err(Invalid::at("document", "more than 24 images"));
    }
    Ok(())
}

/// The JSON Pointer of the node whose `id` is `id`.
fn node_pointer(value: &Value, id: &str, here: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_str) == Some(id) && map.contains_key("type") {
                return Some(here.to_owned());
            }
            map.iter().find_map(|(key, child)| {
                node_pointer(
                    child,
                    id,
                    &format!("{here}/{}", key.replace('~', "~0").replace('/', "~1")),
                )
            })
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, child)| node_pointer(child, id, &format!("{here}/{index}"))),
        _ => None,
    }
}

fn split_last(pointer: &str) -> Option<(&str, String)> {
    let (parent, last) = pointer.rsplit_once('/')?;
    Some((parent, last.replace("~1", "/").replace("~0", "~")))
}

/// Applies `ops` in order to a copy of `document`; all or nothing.
///
/// # Errors
/// The first op that does not apply; the document is unchanged.
pub fn apply_patch(document: &Value, ops: &[Value]) -> Result<Value, Invalid> {
    if ops.is_empty() || ops.len() > PATCH_OPS {
        return Err(Invalid::at("ops", "expected 1 to 64 operations"));
    }
    let mut next = document.clone();
    for (index, op) in ops.iter().enumerate() {
        let at = format!("ops[{index}]");
        let map = object(op, &at)?;
        let raw = map
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| Invalid::at(&format!("{at}.path"), "required"))?;
        let pointer = match raw.strip_prefix('#') {
            Some(rest) => {
                let (id, inner) = rest
                    .split_once('/')
                    .map_or((rest, ""), |(id, inner)| (id, inner));
                let base = node_pointer(&next, id, "")
                    .ok_or_else(|| Invalid::at(&format!("{at}.path"), "no node has this id"))?;
                if inner.is_empty() {
                    base
                } else {
                    format!("{base}/{inner}")
                }
            }
            None if raw.starts_with('/') => raw.to_owned(),
            None => {
                return Err(Invalid::at(
                    &format!("{at}.path"),
                    "expected a JSON Pointer or #id",
                ));
            }
        };
        let missing = || Invalid::at(&format!("{at}.path"), "path does not exist");
        match map.get("op").and_then(Value::as_str) {
            Some("set") => {
                let value = map
                    .get("value")
                    .cloned()
                    .ok_or_else(|| Invalid::at(&format!("{at}.value"), "required"))?;
                *next.pointer_mut(&pointer).ok_or_else(missing)? = value;
            }
            Some("append") => {
                let value = map
                    .get("value")
                    .cloned()
                    .ok_or_else(|| Invalid::at(&format!("{at}.value"), "required"))?;
                next.pointer_mut(&pointer)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| Invalid::at(&format!("{at}.path"), "not a list"))?
                    .push(value);
            }
            Some("remove") => {
                let (parent, last) = split_last(&pointer).ok_or_else(missing)?;
                match next.pointer_mut(parent).ok_or_else(missing)? {
                    Value::Object(map) => {
                        map.remove(&last).ok_or_else(missing)?;
                    }
                    Value::Array(items) => {
                        let index: usize = last.parse().map_err(|_| missing())?;
                        if index >= items.len() {
                            return Err(missing());
                        }
                        items.remove(index);
                    }
                    _ => return Err(missing()),
                }
            }
            _ => {
                return Err(Invalid::at(
                    &format!("{at}.op"),
                    "expected set, append or remove",
                ));
            }
        }
    }
    validate(&next)?;
    Ok(next)
}

fn describe(node: &Value, out: &mut Vec<String>) {
    let text = |key: &str| {
        node.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    if let Some(fallback) = node.get("fallback").and_then(Value::as_str) {
        out.push(fallback.to_owned());
        return;
    }
    match node.get("type").and_then(Value::as_str).unwrap_or_default() {
        "text" | "alert" => out.push(text("text")),
        "status" => out.push(text("label")),
        "link" | "button" => out.push(text("label")),
        "file" => out.push(text("path")),
        "image" => out.push(format!("[image: {}]", text("alt"))),
        "kv" => out.extend(node["items"].as_array().into_iter().flatten().map(|item| {
            format!(
                "{}: {}",
                item["k"].as_str().unwrap_or_default(),
                scalar_text(&item["v"])
            )
        })),
        "steps" => out.extend(node["steps"].as_array().into_iter().flatten().map(|step| {
            let mark = match step["state"].as_str() {
                Some("done") => "[x]",
                Some("active") => "[>]",
                Some("failed") => "[!]",
                _ => "[ ]",
            };
            format!("{mark} {}", step["label"].as_str().unwrap_or_default())
        })),
        "table" => out.push(format!(
            "[table: {} rows]",
            node["rows"].as_array().map_or(0, Vec::len)
        )),
        "choice" => out.push(text("label")),
        _ => {
            for key in ["children", "actions"] {
                for child in node[key].as_array().into_iter().flatten() {
                    describe(child, out);
                }
            }
        }
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Object(map) => scalar_text(map.get("v").unwrap_or(&Value::Null)),
        Value::String(s) => s.clone(),
        Value::Null => "—".to_owned(),
        other => other.to_string(),
    }
}

/// The plain-text projection older clients draw as a notice description.
#[must_use]
pub fn description(document: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(summary) = document.get("summary").and_then(Value::as_str) {
        lines.push(summary.to_owned());
    }
    describe(&document["root"], &mut lines);
    let joined = lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    joined.chars().take(DESCRIPTION_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval() -> Value {
        json!({"schema":"cowboy.widget/1","title":"Deploy","summary":"Build passed","root":{
            "type":"card","id":"release","title":"Release","children":[
                {"type":"kv","items":[{"k":"Commit","v":"1b9f121eb"}]},
                {"type":"steps","id":"plan","steps":[{"label":"Build","state":"done"},{"label":"Activate","state":"pending"}]}
            ],"actions":[{"type":"button","id":"go","label":"Activate it","action":{"kind":"prompt"}}]}})
    }

    #[test]
    fn a_contract_example_validates() {
        validate(&approval()).unwrap();
    }

    #[test]
    fn shape_and_bound_violations_are_addressed() {
        let mut unknown = approval();
        unknown["root"]["children"][0]["type"] = json!("chart");
        assert_eq!(
            validate(&unknown).unwrap_err().path,
            "root.children[0].type"
        );
        let mut ragged = approval();
        ragged["root"] = json!({"type":"table","columns":["a","b"],"rows":[["1"]]});
        assert_eq!(validate(&ragged).unwrap_err().path, "root.rows[0]");
        let mut table = approval();
        table["root"] = json!({"type":"table","sort":true,
            "columns":["a",{"label":"b","width":"min","format":"bytes"}],"rows":[["1",2]]});
        validate(&table).unwrap();
        table["root"]["sort"] = json!("yes");
        assert_eq!(validate(&table).unwrap_err().path, "root.sort");
        table["root"]["sort"] = json!(false);
        table["root"]["columns"][1]["width"] = json!("wide");
        assert_eq!(validate(&table).unwrap_err().path, "root.columns[1].width");
        let mut bidi = approval();
        bidi["title"] = json!("Deploy \u{202E}evil");
        assert_eq!(validate(&bidi).unwrap_err().path, "title");
        let mut prose = approval();
        prose["root"] = json!({"type":"choice","id":"c","label":"Q","options":[{"value":"rm -rf /","label":"A"}]});
        assert!(validate(&prose).is_err());
        let mut http = approval();
        http["root"] =
            json!({"type":"link","label":"x","to":{"kind":"url","url":"http://example.com"}});
        assert!(validate(&http).is_err());
        let mut cells = approval();
        let row: Vec<Value> = (0..12).map(|_| json!(1)).collect();
        cells["root"] = json!({"type":"stack","children":[
            {"type":"table","columns":(0..12).map(|i| format!("c{i}")).collect::<Vec<_>>(),"rows":vec![row.clone(); 100]},
            {"type":"table","columns":(0..12).map(|i| format!("c{i}")).collect::<Vec<_>>(),"rows":vec![row; 100]}]});
        assert_eq!(
            validate(&cells).unwrap_err().message,
            "more than 2000 table cells"
        );
    }

    #[test]
    fn patches_address_nodes_by_id_and_apply_all_or_nothing() {
        let next = apply_patch(&approval(), &[
            json!({"op":"set","path":"#plan/steps/1/state","value":"done"}),
            json!({"op":"set","path":"/summary","value":"Activated"}),
            json!({"op":"append","path":"#plan/steps","value":{"label":"Verify","state":"active"}}),
        ]).unwrap();
        assert_eq!(next["root"]["children"][1]["steps"][1]["state"], "done");
        assert_eq!(next["root"]["children"][1]["steps"][2]["label"], "Verify");
        assert_eq!(next["summary"], "Activated");
        let refused = apply_patch(
            &approval(),
            &[
                json!({"op":"set","path":"/summary","value":"changed"}),
                json!({"op":"set","path":"#missing/x","value":1}),
            ],
        );
        assert_eq!(refused.unwrap_err().path, "ops[1].path");
        let invalid = apply_patch(
            &approval(),
            &[json!({"op":"set","path":"#plan/steps/0/state","value":"maybe"})],
        );
        assert!(
            invalid.is_err(),
            "a patch whose result breaks the contract is refused"
        );
        let removed = apply_patch(
            &approval(),
            &[json!({"op":"remove","path":"#plan/steps/1"})],
        )
        .unwrap();
        assert_eq!(
            removed["root"]["children"][1]["steps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn the_description_is_readable_text() {
        let text = description(&approval());
        assert!(text.contains("Build passed"));
        assert!(text.contains("Commit: 1b9f121eb"));
        assert!(text.contains("[x] Build"));
        assert!(text.contains("Activate it"));
    }
}
