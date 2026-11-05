//! Syntax structure without a language server (`outline@1`, `symbols@1`).
//!
//! Tree-sitter grammars and their upstream tag queries are compiled into
//! Cowboy: no project code runs, no warm-up is needed, and restricted
//! worktrees and Machines without the language engine (macOS) are covered.
//! Answers are `heuristic`: definitions come from syntax, not from type
//! information, and a parse of a broken file can omit or misplace symbols.

use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tree_sitter_tags::{TagsConfiguration, TagsContext};

/// Larger sources are refused rather than parsed partially.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_SYMBOLS: usize = 2_000;
const MAX_DEPTH: usize = 16;
pub const MAX_WORKSPACE_LIMIT: usize = 200;
const WORKSPACE_DEADLINE: Duration = Duration::from_secs(4);
const WORKSPACE_MAX_FILES: usize = 5_000;
const WORKSPACE_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Zero-based row and UTF-16 column, as the language engine reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub row: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureSymbol {
    pub name: String,
    /// LSP `SymbolKind`.
    pub kind: i32,
    pub start: Position,
    pub end: Position,
    pub selection_start: Position,
    pub selection_end: Position,
    pub children: Vec<StructureSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbol {
    pub path: String,
    pub name: String,
    pub kind: i32,
    pub selection_start: Position,
    /// The source line of the definition, trimmed and bounded.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbols {
    pub symbols: Vec<WorkspaceSymbol>,
    /// Set when a bound stopped the scan; `symbols` is then incomplete.
    pub stopped: Option<super::content_search::StopReason>,
    pub files_parsed: usize,
}

struct Grammar {
    extensions: &'static [&'static str],
    config: TagsConfiguration,
}

fn grammar(
    extensions: &'static [&'static str],
    language: tree_sitter::Language,
    tags: &str,
    locals: &str,
) -> Grammar {
    Grammar {
        extensions,
        config: TagsConfiguration::new(language, tags, locals)
            .expect("pinned grammar and tag query agree"),
    }
}

static GRAMMARS: LazyLock<Vec<Grammar>> = LazyLock::new(|| {
    let typescript = [
        tree_sitter_javascript::TAGS_QUERY,
        tree_sitter_typescript::TAGS_QUERY,
    ]
    .concat();
    vec![
        grammar(
            &["rs"],
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::TAGS_QUERY,
            "",
        ),
        grammar(
            &["py", "pyi"],
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::TAGS_QUERY,
            "",
        ),
        grammar(
            &["go"],
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::TAGS_QUERY,
            "",
        ),
        grammar(
            &["js", "mjs", "cjs", "jsx"],
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        grammar(
            &["ts", "mts", "cts"],
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            &typescript,
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        grammar(
            &["tsx"],
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            &typescript,
            tree_sitter_typescript::LOCALS_QUERY,
        ),
    ]
});

fn config_for(path: &str) -> Option<&'static TagsConfiguration> {
    let extension = Path::new(path).extension()?.to_str()?;
    GRAMMARS
        .iter()
        .find(|grammar| grammar.extensions.contains(&extension))
        .map(|grammar| &grammar.config)
}

/// Whether a path has a structure grammar.
#[must_use]
pub fn supported(path: &str) -> bool {
    config_for(path).is_some()
}

fn kind(syntax: &str) -> i32 {
    match syntax {
        "module" => 2,
        "class" => 5,
        "method" => 6,
        "interface" => 11,
        "function" | "macro" => 12,
        "constant" => 14,
        _ => 13,
    }
}

struct Flat {
    name: String,
    kind: i32,
    range: std::ops::Range<usize>,
    start: Position,
    end: Position,
    selection_start: Position,
    selection_end: Position,
}

fn position(source: &[u8], line_starts: &[usize], offset: usize) -> Position {
    let row = line_starts
        .partition_point(|start| *start <= offset)
        .saturating_sub(1);
    let line = &source[line_starts[row]..offset];
    let column = String::from_utf8_lossy(line).encode_utf16().count();
    Position {
        row: u32::try_from(row).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    }
}

fn definitions(path: &str, source: &[u8]) -> Result<Vec<Flat>, String> {
    let config = config_for(path).ok_or("no structure grammar for this file")?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err("file is too large for structure".to_owned());
    }
    let mut line_starts = vec![0];
    line_starts.extend(
        source
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(index, _)| index + 1),
    );
    let mut context = TagsContext::new();
    let (tags, _) = context
        .generate_tags(config, source, None)
        .map_err(|error| format!("parsing failed: {error}"))?;
    let mut flat = Vec::new();
    for tag in tags {
        let Ok(tag) = tag else { continue };
        if !tag.is_definition {
            continue;
        }
        let Ok(name) = std::str::from_utf8(&source[tag.name_range.clone()]) else {
            continue;
        };
        flat.push(Flat {
            name: name.to_owned(),
            kind: kind(config.syntax_type_name(tag.syntax_type_id)),
            start: position(source, &line_starts, tag.range.start),
            end: position(source, &line_starts, tag.range.end),
            selection_start: position(source, &line_starts, tag.name_range.start),
            selection_end: position(source, &line_starts, tag.name_range.end),
            range: tag.range,
        });
        if flat.len() >= MAX_SYMBOLS {
            break;
        }
    }
    flat.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(b.range.end.cmp(&a.range.end))
    });
    Ok(flat)
}

/// Nest definitions by source containment.
fn nest(flat: Vec<Flat>) -> Vec<StructureSymbol> {
    fn into_symbol(item: Flat) -> StructureSymbol {
        StructureSymbol {
            name: item.name,
            kind: item.kind,
            start: item.start,
            end: item.end,
            selection_start: item.selection_start,
            selection_end: item.selection_end,
            children: Vec::new(),
        }
    }
    fn attach(
        stack: &mut Vec<(std::ops::Range<usize>, StructureSymbol)>,
        roots: &mut Vec<StructureSymbol>,
    ) {
        let (_, symbol) = stack.pop().expect("caller checked");
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(symbol),
            None => roots.push(symbol),
        }
    }
    let mut roots = Vec::new();
    let mut stack: Vec<(std::ops::Range<usize>, StructureSymbol)> = Vec::new();
    for item in flat {
        while stack
            .last()
            .is_some_and(|(range, _)| item.range.start >= range.end)
        {
            attach(&mut stack, &mut roots);
        }
        if stack.len() >= MAX_DEPTH {
            attach(&mut stack, &mut roots);
        }
        let range = item.range.clone();
        stack.push((range, into_symbol(item)));
    }
    while !stack.is_empty() {
        attach(&mut stack, &mut roots);
    }
    roots
}

/// The outline of one source text.
///
/// # Errors
/// No grammar for the path, an oversized source or a failed parse.
pub fn outline_source(path: &str, source: &[u8]) -> Result<Vec<StructureSymbol>, String> {
    definitions(path, source).map(nest)
}

/// The outline of one worktree file; the path must stay inside the root.
///
/// # Errors
/// An invalid or escaping path, a missing file, or any `outline_source` error.
pub fn outline(root: &Path, relative: &str) -> Result<Vec<StructureSymbol>, String> {
    let source = read_inside(root, relative)?;
    outline_source(relative, &source)
}

fn read_inside(root: &Path, relative: &str) -> Result<Vec<u8>, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("invalid path".to_owned());
    }
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let canonical = root
        .join(path)
        .canonicalize()
        .map_err(|_| "file does not exist".to_owned())?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return Err("file is outside the worktree".to_owned());
    }
    let metadata = std::fs::metadata(&canonical).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_SOURCE_BYTES as u64 {
        return Err("file is too large for structure".to_owned());
    }
    std::fs::read(&canonical).map_err(|error| error.to_string())
}

/// Definitions whose name contains `query` (case-insensitive), across the
/// worktree's supported files, with the same walk rules as content search.
///
/// # Errors
/// An empty or oversized query.
pub fn workspace_symbols(
    root: &Path,
    query: &str,
    limit: usize,
) -> Result<WorkspaceSymbols, String> {
    use super::content_search::StopReason;
    if query.is_empty() || query.len() > super::content_search::MAX_QUERY_BYTES {
        return Err("query must be 1 to 256 bytes".to_owned());
    }
    let needle = query.to_lowercase();
    let limit = limit.clamp(1, MAX_WORKSPACE_LIMIT);
    let deadline = Instant::now() + WORKSPACE_DEADLINE;
    let mut result = WorkspaceSymbols {
        symbols: Vec::new(),
        stopped: None,
        files_parsed: 0,
    };
    let mut bytes = 0_u64;
    let walk = ignore::WalkBuilder::new(root)
        .follow_links(false)
        .hidden(true)
        .parents(true)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(relative) = entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(Path::to_str)
            .filter(|relative| supported(relative))
        else {
            continue;
        };
        if Instant::now() >= deadline {
            result.stopped = Some(StopReason::Deadline);
            break;
        }
        if result.files_parsed >= WORKSPACE_MAX_FILES {
            result.stopped = Some(StopReason::Files);
            break;
        }
        let size = entry.metadata().map_or(0, |metadata| metadata.len());
        if size > MAX_SOURCE_BYTES as u64 {
            continue;
        }
        if bytes + size > WORKSPACE_MAX_BYTES {
            result.stopped = Some(StopReason::Bytes);
            break;
        }
        let Ok(source) = std::fs::read(entry.path()) else {
            continue;
        };
        bytes += source.len() as u64;
        result.files_parsed += 1;
        let Ok(flat) = definitions(relative, &source) else {
            continue;
        };
        let text = String::from_utf8_lossy(&source);
        let lines: Vec<&str> = text.lines().collect();
        for item in flat {
            if !item.name.to_lowercase().contains(&needle) {
                continue;
            }
            if result.symbols.len() >= limit {
                result.stopped = Some(StopReason::Limit);
                return Ok(result);
            }
            let line = lines
                .get(item.selection_start.row as usize)
                .copied()
                .unwrap_or_default()
                .trim()
                .chars()
                .take(200)
                .collect();
            result.symbols.push(WorkspaceSymbol {
                path: relative.to_owned(),
                name: item.name,
                kind: item.kind,
                selection_start: item.selection_start,
                text: line,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
