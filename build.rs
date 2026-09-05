//! Discover first-party host plugins at compile time.
//!
//! Adding `host.json` under `examples/authentication/<id>/` or `plugins/<id>/`
//! stages that plugin on boot. Core Rust does not list plugin ids. Machine-host
//! builds omit those trees; this script then emits an empty inventory.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct BundledHost {
    id: String,
    files: Vec<(String, PathBuf)>,
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=examples/authentication");
    println!("cargo:rerun-if-changed=plugins");

    let mut hosts = Vec::new();
    collect_hosts(&manifest.join("examples/authentication"), &mut hosts);
    collect_hosts(&manifest.join("plugins"), &mut hosts);
    hosts.sort_by(|left, right| left.id.cmp(&right.id));
    if let Some(duplicate) = hosts.windows(2).find(|pair| pair[0].id == pair[1].id) {
        panic!("duplicate first-party host plugin {}", duplicate[0].id);
    }
    if env::var("CARGO_FEATURE_FULL").is_ok() {
        let ids: Vec<&str> = hosts.iter().map(|host| host.id.as_str()).collect();
        for required in ["password", "passkey"] {
            assert!(
                ids.contains(&required),
                "first-party host plugin {required} is missing from the controller source"
            );
        }
    }

    write_first_party_plugins(&manifest);
    write_first_party_sources(&manifest);

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("bundled_hosts.rs");
    let mut rust = String::from("&[\n");
    for host in &hosts {
        rust.push_str("    BundledHost {\n");
        rust.push_str(&format!("        id: {:?},\n", host.id));
        rust.push_str("        files: &[\n");
        for (relative, path) in &host.files {
            let suffix = format!(
                "/{}",
                path.strip_prefix(&manifest)
                    .unwrap_or(path)
                    .display()
                    .to_string()
                    .replace('\\', "/")
            );
            rust.push_str(&format!(
                "            BundledHostFile {{ path: {relative:?}, content: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {suffix:?})) }},\n"
            ));
        }
        rust.push_str("        ],\n    },\n");
    }
    rust.push_str("]\n");
    fs::write(&out, rust).unwrap_or_else(|error| panic!("writing {}: {error}", out.display()));
}

fn write_first_party_sources(manifest: &Path) {
    let mut sources: BTreeMap<String, (Option<PathBuf>, Option<PathBuf>)> = BTreeMap::new();
    collect_payload_sources(&manifest.join("plugins"), &mut sources);
    collect_payload_sources(&manifest.join("examples/authentication"), &mut sources);

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    write_source_array(
        &out_dir.join("first_party_provider_sources.rs"),
        manifest,
        &sources,
        true,
    );
    write_source_array(
        &out_dir.join("first_party_host_sources.rs"),
        manifest,
        &sources,
        false,
    );
}

fn collect_payload_sources(
    root: &Path,
    sources: &mut BTreeMap<String, (Option<PathBuf>, Option<PathBuf>)>,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_plugin_id(id) {
            continue;
        }
        let provider = path.join("provider.json");
        let host = path.join("host.json");
        if !provider.is_file() && !host.is_file() {
            continue;
        }
        let row = sources.entry(id.to_owned()).or_default();
        if provider.is_file() {
            assert!(
                row.0.replace(provider).is_none(),
                "duplicate first-party provider source {id}"
            );
        }
        if host.is_file() {
            assert!(
                row.1.replace(host).is_none(),
                "duplicate first-party host source {id}"
            );
        }
    }
}

fn write_source_array(
    out: &Path,
    manifest: &Path,
    sources: &BTreeMap<String, (Option<PathBuf>, Option<PathBuf>)>,
    provider: bool,
) {
    let mut rust = String::from("&[\n");
    for (id, (provider_path, host_path)) in sources {
        let path = if provider { provider_path } else { host_path };
        let Some(path) = path else {
            continue;
        };
        let suffix = format!(
            "/{}",
            path.strip_prefix(manifest)
                .unwrap_or(path)
                .display()
                .to_string()
                .replace('\\', "/")
        );
        rust.push_str(&format!(
            "    ({id:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {suffix:?}))),\n"
        ));
    }
    rust.push_str("]\n");
    fs::write(out, rust).unwrap_or_else(|error| panic!("writing {}: {error}", out.display()));
}

fn write_first_party_plugins(manifest: &Path) {
    let mut paths = Vec::new();
    collect_plugin_json(manifest.join("plugins"), &mut paths);
    paths.sort();
    if env::var("CARGO_FEATURE_FULL").is_ok() {
        assert!(
            paths.len() >= 7,
            "first-party plugin.json inventory is incomplete"
        );
    }
    let mut rust = String::from("&[\n");
    for path in &paths {
        let suffix = format!(
            "/{}",
            path.strip_prefix(manifest)
                .unwrap_or(path)
                .display()
                .to_string()
                .replace('\\', "/")
        );
        rust.push_str(&format!(
            "    include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {suffix:?})),\n"
        ));
    }
    rust.push_str("]\n");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("first_party_plugins.rs");
    fs::write(&out, rust).unwrap_or_else(|error| panic!("writing {}: {error}", out.display()));
}

fn collect_plugin_json(root: PathBuf, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_plugin_id(id) {
            continue;
        }
        let plugin_json = path.join("plugin.json");
        if plugin_json.is_file() {
            paths.push(plugin_json);
        }
    }
}

fn collect_hosts(root: &Path, hosts: &mut Vec<BundledHost>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_plugin_id(id) {
            continue;
        }
        let host_json = path.join("host.json");
        if !host_json.is_file() {
            continue;
        }
        let mut files = vec![("host.json".to_owned(), host_json)];
        reject_host_ui_files(&path.join("ui"));
        let collector_root = path.join("collector");
        if collector_root.is_dir() {
            collect_host_files(&collector_root, "collector", &["js", "json"], &mut files);
        }
        hosts.push(BundledHost {
            id: id.to_owned(),
            files,
        });
    }
}

fn reject_host_ui_files(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("reading {}: {error}", root.display()));
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("reading {}: {error}", entry.path().display()));
        if file_type.is_dir() {
            reject_host_ui_files(&entry.path());
        } else {
            panic!(
                "plugin host UI code is forbidden; move presentation into a Cowboy-owned renderer: {}",
                entry.path().display()
            );
        }
    }
}

fn collect_host_files(
    root: &Path,
    prefix: &str,
    extensions: &[&str],
    files: &mut Vec<(String, PathBuf)>,
) {
    let mut entries: Vec<_> = fs::read_dir(root)
        .unwrap_or_else(|error| panic!("reading {}: {error}", root.display()))
        .map(|entry| entry.unwrap_or_else(|error| panic!("reading {}: {error}", root.display())))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .unwrap_or_else(|_| panic!("plugin host path below {} is not UTF-8", root.display()));
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("reading {}: {error}", entry.path().display()));
        if !file_type.is_file() && !file_type.is_dir() {
            continue;
        }
        assert!(
            is_host_path_segment(&name),
            "unsafe plugin host path segment {name:?} below {}",
            root.display()
        );
        let relative = format!("{prefix}/{name}");
        if file_type.is_dir() {
            collect_host_files(&entry.path(), &relative, extensions, files);
            continue;
        }
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            files.push((relative, entry.path()));
        }
    }
}

fn is_host_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_plugin_id(plugin_id: &str) -> bool {
    (1..=64).contains(&plugin_id.len())
        && plugin_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !plugin_id.starts_with('-')
        && !plugin_id.ends_with('-')
        && !plugin_id.contains("--")
}
