use super::*;

fn codes(report: &Report) -> Vec<&'static str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn every_typed_key_matches_a_declared_field_of_its_kind() {
    fn assert_key<T: FromValue>(key: &Key<T>) {
        let field = key
            .scope
            .fields()
            .iter()
            .find(|field| field.key == key.path)
            .unwrap_or_else(|| panic!("{} is not declared", key.path));
        assert!(
            T::from_value(&field.default).is_some(),
            "{} has a key type that does not match its kind",
            key.path
        );
        // Reading through the typed key never panics.
        let _ = Config::defaults(key.scope).get(key);
    }
    assert_key(&schema::PLUGIN_GENERATION_RETENTION_INTERVAL);
    assert_key(&schema::DEVICE_MAX_SESSIONS);
    assert_key(&schema::DEVICE_DRAINING);
}

#[test]
fn declarations_are_unique_documented_and_defaults_are_in_range() {
    for scope in [Scope::Service, Scope::Device] {
        let mut seen = std::collections::BTreeSet::new();
        for field in scope.fields() {
            assert!(seen.insert(field.key), "duplicate key {}", field.key);
            assert!(
                field.key.contains('.'),
                "{} must live in a section",
                field.key
            );
            assert!(
                !field.doc.trim().is_empty(),
                "{} needs documentation",
                field.key
            );
            // The generated template round-trips each default through the parser.
            let (table, name) = field.key.rsplit_once('.').unwrap();
            let text = format!(
                "schema = 1\n[{table}]\n{name} = {}\n",
                field.default.to_toml()
            );
            let report = check_text(scope, &text, None);
            assert!(report.is_valid(), "{}: {}", field.key, report.render_text());
            assert_eq!(report.config.unwrap().source(field.key), Some(Source::File));
        }
    }
}

#[test]
fn checked_in_examples_match_the_schema() {
    for scope in [Scope::Service, Scope::Device] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config")
            .join(scope.file_name().replace(".toml", ".example.toml"));
        let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            checked_in,
            example(scope),
            "{} is stale; regenerate it with `cowboy config init --scope {} --print`",
            path.display(),
            scope.as_str()
        );
        assert!(check_text(scope, &checked_in, None).is_valid());
    }
}

#[test]
fn missing_file_yields_defaults_and_valid_values_override_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = Scope::Device.path_in(directory.path());
    let report = check_file(Scope::Device, &path);
    let config = report.config.unwrap();
    assert_eq!(config.get(&schema::DEVICE_MAX_SESSIONS), 8);
    assert_eq!(
        config.source("capacity.max_sessions"),
        Some(Source::Default)
    );

    let report = check_text(
        Scope::Device,
        "schema = 1\n[capacity]\nmax_sessions = 28\n",
        None,
    );
    let config = report.config.unwrap();
    assert_eq!(config.get(&schema::DEVICE_MAX_SESSIONS), 28);
    assert!(!config.get(&schema::DEVICE_DRAINING));
}

#[test]
fn diagnostics_locate_the_problem_and_say_how_to_fix_it() {
    let text = "schema = 1\n[capacity]\nmax_sesions = 4\ndraining = \"yes\"\n[capacity.extra]\n";
    let report = check_text(Scope::Device, text, None);
    assert!(!report.is_valid());
    assert_eq!(
        codes(&report),
        vec!["unknown-key", "wrong-type", "unknown-key"]
    );
    let typo = &report.diagnostics[0];
    assert_eq!((typo.line, typo.column), (Some(3), Some(1)));
    assert_eq!(typo.path, "capacity.max_sesions");
    assert_eq!(
        typo.hint.as_deref(),
        Some("did you mean `capacity.max_sessions`?")
    );
    let wrong = &report.diagnostics[1];
    assert_eq!(wrong.path, "capacity.draining");
    assert_eq!(wrong.found.as_deref(), Some("\"yes\""));
    assert_eq!(wrong.expected.as_deref(), Some("boolean (true or false)"));

    let json = report.to_json();
    assert_eq!(json["valid"], false);
    assert_eq!(json["diagnostics"][0]["code"], "unknown-key");
}

#[test]
fn ranges_units_schema_and_syntax_are_enforced() {
    let out_of_range = check_text(
        Scope::Device,
        "schema = 1\n[capacity]\nmax_sessions = 0\n",
        None,
    );
    assert_eq!(codes(&out_of_range), vec!["out-of-range"]);
    assert_eq!(
        out_of_range.diagnostics[0].expected.as_deref(),
        Some("integer from 1 to 1024")
    );

    let unit = check_text(
        Scope::Service,
        "schema = 1\n[plugins]\ngeneration_retention_interval = \"6 hours\"\n",
        None,
    );
    assert_eq!(codes(&unit), vec!["invalid-value"]);
    let short = check_text(
        Scope::Service,
        "schema = 1\n[plugins]\ngeneration_retention_interval = \"1m\"\n",
        None,
    );
    assert_eq!(codes(&short), vec!["out-of-range"]);

    assert_eq!(
        codes(&check_text(Scope::Device, "[capacity]\n", None)),
        vec!["missing-schema"]
    );
    assert_eq!(
        codes(&check_text(Scope::Device, "schema = 2\n", None)),
        vec!["unsupported-schema"]
    );
    let syntax = check_text(Scope::Device, "schema = 1\n[capacity\n", None);
    assert_eq!(codes(&syntax), vec!["syntax"]);
    assert_eq!(syntax.diagnostics[0].line, Some(2));

    // A Service setting is not a Device setting, and vice versa.
    assert_eq!(
        codes(&check_text(
            Scope::Device,
            "schema = 1\n[plugins]\ngeneration_retention_interval = \"6h\"\n",
            None
        )),
        vec!["unknown-key"]
    );
}

#[test]
fn units_parse_and_format_round_trip() {
    assert_eq!(parse_duration("90s"), Ok(Duration::from_secs(90)));
    assert_eq!(parse_duration("6h"), Ok(Duration::from_secs(21_600)));
    assert!(parse_duration("6").is_err());
    assert!(parse_duration("6w").is_err());
    assert_eq!(format_duration(Duration::from_secs(21_600)), "6h");
    assert_eq!(format_duration(Duration::from_secs(90)), "90s");
    assert_eq!(parse_bytes("1.5GiB"), Ok(3 << 29));
    assert_eq!(parse_bytes("512MiB"), Ok(512 << 20));
    assert!(parse_bytes("1.5GB").is_err());
    assert!(parse_bytes("1.2345GiB").is_err());
    assert_eq!(format_bytes(3 << 29), "1536MiB");
}

#[test]
fn reload_applies_live_fields_and_holds_restart_fields() {
    let handle = Handle::new(Config::defaults(Scope::Device));
    let next = check_text(
        Scope::Device,
        "schema = 1\n[capacity]\nmax_sessions = 28\n",
        None,
    );
    let (live, restart) = handle.reload(&next).unwrap();
    assert!(live.is_empty());
    assert_eq!(restart.len(), 1);
    assert_eq!(
        handle.get(&schema::DEVICE_MAX_SESSIONS),
        8,
        "restart field keeps its startup value"
    );

    let service = Handle::new(Config::defaults(Scope::Service));
    let next = check_text(
        Scope::Service,
        "schema = 1\n[plugins]\ngeneration_retention_interval = \"1h\"\n",
        None,
    );
    let (live, _) = service.reload(&next).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(
        service.get(&schema::PLUGIN_GENERATION_RETENTION_INTERVAL),
        Duration::from_secs(3600)
    );
    // An invalid edit changes nothing.
    let broken = check_text(Scope::Service, "schema = 1\n[plugins]\nnope = 1\n", None);
    assert!(service.reload(&broken).is_none());
    assert_eq!(
        service.get(&schema::PLUGIN_GENERATION_RETENTION_INTERVAL),
        Duration::from_secs(3600)
    );
}

#[test]
fn install_validates_keeps_history_and_rejects_unsafe_files() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let path = Scope::Device.path_in(directory.path());
    assert!(
        install(
            Scope::Device,
            &path,
            "schema = 1\n[capacity]\nmax_sessions = 0\n"
        )
        .is_err()
    );
    assert!(
        !path.exists(),
        "an invalid candidate never reaches the live path"
    );

    install(
        Scope::Device,
        &path,
        "schema = 1\n[capacity]\nmax_sessions = 4\n",
    )
    .unwrap();
    install(
        Scope::Device,
        &path,
        "schema = 1\n[capacity]\nmax_sessions = 6\n",
    )
    .unwrap();
    assert_eq!(
        check_file(Scope::Device, &path)
            .config
            .unwrap()
            .get(&schema::DEVICE_MAX_SESSIONS),
        6
    );
    let history = std::fs::read_dir(path.parent().unwrap().join("history"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(history, vec!["schema = 1\n[capacity]\nmax_sessions = 4\n"]);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let link = directory.path().join("link.toml");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert_eq!(
        codes(&check_file(Scope::Device, &link)),
        vec!["not-a-regular-file"]
    );
    assert!(load(Scope::Device, &link).is_err());
}
