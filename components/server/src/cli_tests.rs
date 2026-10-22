use clap::Parser as _;

use crate::cli::{Cli, Command, mask_secret};

#[test]
#[cfg(unix)]
fn managed_calls_have_provider_aliases_and_bounded_noninteractive_waits() {
    for provider in ["codex", "claude"] {
        assert!(
            Cli::try_parse_from(["cowboy", provider, "--request-file", "-", "--check"]).is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "cowboy",
                provider,
                "--request-file",
                "-",
                "--wait-ms",
                "60001"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "cowboy",
                provider,
                "--request-file",
                "-",
                "--parent-session-id",
                "victim"
            ])
            .is_err()
        );
    }
    assert!(
        Cli::try_parse_from([
            "cowboy",
            "call",
            "start",
            "--provider",
            "claude-code",
            "--request-file",
            "-"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "cowboy",
            "call",
            "start",
            "--provider",
            "auto",
            "--request-file",
            "-"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "cowboy",
            "call",
            "start",
            "--provider",
            "/bin/sh",
            "--request-file",
            "-"
        ])
        .is_err()
    );
    // A wait longer than one gateway exchange is sliced by the client,
    // still within one hour.
    assert!(
        Cli::try_parse_from(["cowboy", "call", "wait", "call-1", "--wait-ms", "900000"]).is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "cowboy",
            "call",
            "wait",
            "call-1",
            "--timeout-ms",
            "3600001"
        ])
        .is_err()
    );
}

#[test]
#[cfg(feature = "full")]
fn catalog_inspection_and_host_preflight_are_distinct_commands() {
    assert!(
        Cli::try_parse_from([
            "cowboy",
            "serve",
            "--check-plugin-catalog",
            "--check-plugin-hosts",
        ])
        .is_err()
    );
}

#[tokio::test]
#[cfg(feature = "full")]
async fn catalog_inspection_does_not_validate_host_policy_or_create_service_state() {
    let root = std::env::temp_dir().join(format!("cowboy-cli-catalog-{}", uuid::Uuid::new_v4()));
    for (catalog_only, hosts) in [(true, false), (false, true), (false, false)] {
        let mut args = crate::cli::ServeArgs::test_plugin_check(&root);
        args.check_plugin_hosts = hosts;
        args.check_plugin_catalog = catalog_only;
        args.plugin_catalog_dir = Some(root.join("missing-catalog"));
        args.plugin_host_config = Some(root.join("missing-policy.json"));
        args.auth_config = Some(root.join("missing-auth.json"));
        args.machine_components_manifest = Some(root.join("missing-components.json"));
        args.product_auth_enabled = true;
        args.database_url = Some(format!("sqlite://{}", root.join("state.db").display()));
        let result = crate::server::serve(args).await;
        assert_eq!(
            result.is_ok(),
            catalog_only,
            "unexpected check result: {result:?}"
        );
        assert!(!root.exists());
    }
}

#[tokio::test]
#[cfg(feature = "full")]
async fn plugin_preflight_exits_before_service_identity_or_other_startup_effects() {
    crate::install_controller_hooks();
    let root = std::env::temp_dir().join(format!(
        "cowboy-cli-preflight-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut args = crate::cli::ServeArgs::test_plugin_check(&root);
    args.plugin_catalog_dir = Some(root.join("external"));
    // Normal startup must read this file, so success also proves the
    // diagnostic returns before unrelated component setup.
    args.machine_components_manifest = Some(root.join("missing-components.json"));
    let cli = Cli {
        command: Command::Serve(Box::new(args)),
    };
    Box::pin(cli.run()).await.unwrap();
    assert!(
        !root.exists(),
        "read-only preflight created Service/Catalog state"
    );
}

#[test]
#[cfg(feature = "full")]
fn serve_accepts_backend_neutral_database_url() {
    let cli =
        Cli::try_parse_from(["cowboy", "serve", "--database-url", "sqlite::memory:"]).unwrap();
    let Command::Serve(args) = cli.command else {
        panic!("expected serve command");
    };
    assert_eq!(args.database_url(), Some("sqlite::memory:"));
}

#[test]
#[cfg(feature = "full")]
fn serve_keeps_legacy_postgres_url_compatible() {
    let cli =
        Cli::try_parse_from(["cowboy", "serve", "--postgres-url", "postgresql:///cowboy"]).unwrap();
    let Command::Serve(args) = cli.command else {
        panic!("expected serve command");
    };
    assert_eq!(args.database_url(), Some("postgresql:///cowboy"));
}

#[test]
#[cfg(feature = "full")]
fn store_copy_requires_explicit_source_and_destination() {
    let cli = Cli::try_parse_from([
        "cowboy",
        "store-copy",
        "--source",
        "postgresql:///cowboy",
        "--destination",
        "sqlite:///tmp/cowboy.sqlite3",
    ])
    .unwrap();
    let Command::StoreCopy(args) = cli.command else {
        panic!("expected store-copy command");
    };
    assert_eq!(args.source, "postgresql:///cowboy");
    assert_eq!(args.destination, "sqlite:///tmp/cowboy.sqlite3");
}

#[test]
#[cfg(feature = "full")]
fn colocated_execution_is_denied_until_a_machine_is_named() {
    let serve = |extra: &[&str]| {
        let mut command_line = vec!["cowboy", "serve"];
        command_line.extend_from_slice(extra);
        let Command::Serve(args) = Cli::try_parse_from(command_line).unwrap().command else {
            panic!("serve");
        };
        args.colocated_machines
    };
    // Empty by default: no Machine may be read on the Controller's own
    // filesystem merely because it says so.
    assert!(serve(&[]).is_empty());
    assert_eq!(serve(&["--colocated-machine", "hawk"]), vec!["hawk"]);
    // Repeated and comma-separated forms both name several Machines.
    assert_eq!(
        serve(&[
            "--colocated-machine",
            "hawk",
            "--colocated-machine",
            "spare"
        ]),
        vec!["hawk", "spare"]
    );
    assert_eq!(
        serve(&["--colocated-machine", "hawk,spare"]),
        vec!["hawk", "spare"]
    );
}

#[test]
#[cfg(feature = "full")]
fn navigation_policy_defaults_closed_and_requires_explicit_candidate() {
    let Command::Serve(args) = Cli::try_parse_from(["cowboy", "serve"]).unwrap().command else {
        panic!("serve");
    };
    assert_eq!(
        args.code_navigation_admission,
        crate::cli::CodeNavigationAdmission::Closed
    );
    let Command::Serve(args) = Cli::try_parse_from([
        "cowboy",
        "serve",
        "--code-navigation-admission",
        "candidate",
    ])
    .unwrap()
    .command
    else {
        panic!("serve");
    };
    assert_eq!(
        args.code_navigation_admission,
        crate::cli::CodeNavigationAdmission::Candidate
    );
    for value in ["true", "enabled", "21"] {
        assert!(
            Cli::try_parse_from(["cowboy", "serve", "--code-navigation-admission", value]).is_err()
        );
    }
}

#[test]
#[cfg(feature = "full")]
fn serve_product_auth_defaults_on() {
    let cli = Cli::try_parse_from(["cowboy", "serve"]).unwrap();
    let Command::Serve(args) = cli.command else {
        panic!("expected serve command");
    };
    assert!(args.product_auth_enabled);

    let cli = Cli::try_parse_from(["cowboy", "serve", "--product-auth-enabled", "true"]).unwrap();
    let Command::Serve(args) = cli.command else {
        panic!("expected serve command");
    };
    assert!(args.product_auth_enabled);
}

#[test]
#[cfg(feature = "full")]
fn serve_acp_keeps_hidden_legacy_token_compatibility() {
    let cli = Cli::try_parse_from([
        "cowboy",
        "serve-acp",
        "--provider",
        "codex",
        "--token",
        "cow_testtoken",
    ])
    .unwrap();
    let Command::ServeAcp(args) = cli.command else {
        panic!("expected serve-acp command");
    };
    assert_eq!(args.token.as_deref(), Some("cow_testtoken"));
}

#[test]
#[cfg(feature = "full")]
fn serve_acp_and_login_default_to_browser_authorized_devices() {
    let cli = Cli::try_parse_from(["cowboy", "serve-acp", "--provider", "codex"]).unwrap();
    let Command::ServeAcp(args) = cli.command else {
        panic!("expected serve-acp command");
    };
    assert!(args.token.is_none());
    assert!(args.auth_state_dir.is_none());
    assert!(args.runtime_machine.is_none());
    let remote = Cli::try_parse_from([
        "cowboy",
        "serve-acp",
        "--runtime-machine",
        "ovh",
        "--machine",
        "hawk",
        "--workspace",
        "cowboy",
    ])
    .unwrap();
    let Command::ServeAcp(remote) = remote.command else {
        panic!("expected serve-acp");
    };
    assert_eq!(remote.runtime_machine.as_deref(), Some("ovh"));
    assert_eq!(remote.machine, "hawk");
    assert!(Cli::try_parse_from(["cowboy", "serve-acp", "--runtime-machine", "ovh"]).is_err());

    let cli = Cli::try_parse_from([
        "cowboy",
        "login",
        "https://cowboy.example",
        "--device-name",
        "Zed on Hawk",
        "--cardea-profile",
        "cowboy-hawk",
    ])
    .unwrap();
    let Command::Login(args) = cli.command else {
        panic!("expected login command");
    };
    assert_eq!(args.origin, "https://cowboy.example");
    assert_eq!(args.device_name.as_deref(), Some("Zed on Hawk"));
    assert_eq!(args.cardea_profile.as_deref(), Some("cowboy-hawk"));
}

#[test]
fn register_is_origin_and_optional_machine_id() {
    let cli = Cli::try_parse_from([
        "cowboy",
        "register",
        "https://cowboy.example",
        "--token-file",
        "/tmp/cowboy-enroll.token",
    ])
    .unwrap();
    let Command::Register(args) = cli.command else {
        panic!("expected register command");
    };
    assert_eq!(args.origin, "https://cowboy.example");
    assert_eq!(args.machine_id, None);
    assert!(args.workspaces.is_empty());
    assert!(!args.background);
    assert_eq!(
        args.token_file.as_deref(),
        Some(std::path::Path::new("/tmp/cowboy-enroll.token"))
    );

    let cli = Cli::try_parse_from([
        "cowboy",
        "register",
        "https://cowboy.example",
        "macbook-air",
    ])
    .unwrap();
    let Command::Register(args) = cli.command else {
        panic!("expected register command");
    };
    assert_eq!(args.machine_id.as_deref(), Some("macbook-air"));

    let cli = Cli::try_parse_from([
        "cowboy",
        "register",
        "https://cowboy.example",
        "--background",
    ])
    .unwrap();
    let Command::Register(args) = cli.command else {
        panic!("expected register command");
    };
    assert!(args.background);
}

#[test]
fn identity_is_a_top_level_command() {
    let cli = Cli::try_parse_from(["cowboy", "identity"]).unwrap();
    assert!(matches!(cli.command, Command::Identity(_)));
}

#[test]
fn enrollment_token_mask_reveals_only_the_last_four_characters() {
    let token = "7XSdk_AvMYumg66vkgC6ZkVZ_Ak572CgSC_A9jc6zKA";
    let masked = mask_secret(token);
    assert_eq!(masked.chars().count(), token.chars().count());
    assert!(masked.ends_with("6zKA"));
    assert!(
        masked[..masked.len() - 4]
            .chars()
            .all(|character| character == '*')
    );
    assert_eq!(mask_secret("abc"), "abc");
}
