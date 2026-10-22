//! Bounded structural JSON selection shared by observation domains.
use super::*;
const PAGE_BYTES: usize = 32 * 1024;

/// The selection's JSON pointer. Agents often omit its leading slash
/// (`"plugins"`, `"0"`); that spelling has only one meaning, so accept it.
fn pointer(args: &Value) -> Result<String, String> {
    let path = match args.get("path") {
        None | Some(Value::Null) => "",
        Some(Value::String(path)) => path.as_str(),
        _ => return Err("invalid_argument".into()),
    };
    let path = if path.is_empty() || path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    if valid_pointer(&path) {
        Ok(path)
    } else {
        Err("invalid_argument".into())
    }
}

pub(super) fn select(value: &Value, args: &Value) -> Result<Value, String> {
    let path = &pointer(args)?;
    let selected = value.pointer(path).ok_or("not_found")?;
    let describe = if args.get("describe").is_none() {
        path.is_empty()
    } else {
        optional_bool(args, "describe")?
    };
    if let Some(text) = selected.as_str().filter(|_| !describe) {
        let offset =
            usize::try_from(arg_u64(args, "offset", 0, u64::MAX)?).map_err(|_| "invalid_cursor")?;
        let chars = arg_u64(args, "chars", 1000, 4000)? as usize;
        if chars == 0 {
            return Err("invalid_argument".into());
        }
        let total = text.chars().count();
        if offset > total {
            return Err("invalid_cursor".into());
        }
        let data = text.chars().skip(offset).take(chars).collect::<String>();
        let end = offset + data.chars().count();
        return Ok(
            json!({"path":path,"kind":"string","data":data,"range":{"offset":offset,"next":(end < total).then_some(end),"total":total}}),
        );
    }
    let mut query = args.clone();
    query["describe"] = json!(describe);
    query["path"] = json!(path);
    page(Some(value), &query)
}

pub(super) fn page(data: Option<&Value>, args: &Value) -> Result<Value, String> {
    let path = &pointer(args)?;
    let limit = page_limit(args, 50, 200)?;
    let offset =
        usize::try_from(arg_u64(args, "offset", 0, u64::MAX)?).map_err(|_| "invalid_argument")?;
    let describe = optional_bool(args, "describe")?;
    let value = match data {
        Some(value) => value.pointer(path).ok_or("not_found")?,
        None if path.is_empty() => &Value::Null,
        None => return Err("not_found".into()),
    };
    let (kind, total) = match value {
        Value::Array(items) => ("array", items.len()),
        Value::Object(items) => ("object", items.len()),
        _ => ("value", 1),
    };
    if offset > total || (kind == "value" && offset != 0) {
        return Err("invalid_cursor".into());
    }
    let mut budget = PAGE_BYTES;
    let data = match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .skip(offset)
                .take(limit)
                .map(|value| copy_value(value, describe, &mut budget))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(items) => {
            let mut entries: Vec<_> = items.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                entries
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|(key, value)| {
                        let mut counter = ByteBudget(budget);
                        serde_json::to_writer(&mut counter, key)
                            .map_err(|_| "selection_too_large")?;
                        budget = counter.0;
                        copy_value(value, describe, &mut budget).map(|value| (key.clone(), value))
                    })
                    .collect::<Result<_, _>>()?,
            )
        }
        _ => copy_value(value, describe, &mut budget)?,
    };
    // Account for object keys and framing too, including malicious long keys.
    serde_json::to_writer(ByteBudget(PAGE_BYTES), &data).map_err(|_| "selection_too_large")?;
    let end = offset.saturating_add(limit).min(total);
    Ok(
        json!({"path":path,"kind":kind,"describe":describe,"data":data,
        "range":{"offset":offset,"next":(end < total).then_some(end),"total":total}}),
    )
}

fn copy_value(value: &Value, describe: bool, budget: &mut usize) -> Result<Value, String> {
    let description;
    let value = if describe {
        description = describe_value(value);
        &description
    } else {
        value
    };
    // Check before cloning a large descendant; describe reveals its shape.
    let mut counter = ByteBudget(*budget);
    serde_json::to_writer(&mut counter, value).map_err(|_| "selection_too_large".to_owned())?;
    *budget = counter.0;
    Ok(value.clone())
}

pub(super) fn valid_pointer(path: &str) -> bool {
    if path.len() > 512 || (!path.is_empty() && !path.starts_with('/')) {
        return false;
    }
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
            return false;
        }
    }
    true
}

fn describe_value(value: &Value) -> Value {
    match value {
        Value::Object(items) => json!({"type":"object","length":items.len()}),
        Value::Array(items) => json!({"type":"array","length":items.len()}),
        Value::String(text) => json!({"type":"string","bytes":text.len()}),
        Value::Number(_) => json!({"type":"number"}),
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Null => json!({"type":"null"}),
    }
}

struct ByteBudget(usize);
impl std::io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("page budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
