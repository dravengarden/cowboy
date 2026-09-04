//! Discover first-party host plugins at compile time.
//!
//! Adding `host.json` under `examples/authentication/<id>/` or `plugins/<id>/`
//! stages that plugin on boot. Core Rust does not list plugin ids. Machine-host
//! builds omit those trees; this script then emits an empty inventory.

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
        let ui_root = path.join("ui");
        if ui_root.is_dir() {
            let mut ui_files: Vec<_> = fs::read_dir(&ui_root)
                .unwrap_or_else(|error| panic!("reading {}: {error}", ui_root.display()))
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect();
            ui_files.sort();
            for file in ui_files {
                let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !(name.ends_with(".js") || name.ends_with(".css") || name.ends_with(".json")) {
                    continue;
                }
                files.push((format!("ui/{name}"), file));
            }
        }
        hosts.push(BundledHost {
            id: id.to_owned(),
            files,
        });
    }
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
