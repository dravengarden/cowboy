//! Keep each incarnation's immutable CLI reachable independently of host upgrades.
use super::*;

fn cli_closure(cli: &keeper::Executor) -> Result<&Path, Refusal> {
    keeper::agent_cli_directory(cli).map_err(|_| Refusal::PreparationFailed)?;
    let command = Path::new(&cli.command);
    let bin = command.parent().ok_or(Refusal::PreparationFailed)?;
    let closure = bin.parent().ok_or(Refusal::PreparationFailed)?;
    if bin.file_name().and_then(|name| name.to_str()) != Some("bin")
        || closure.parent() != Some(Path::new("/nix/store"))
    {
        return Err(Refusal::PreparationFailed);
    }
    Ok(closure)
}

pub(super) async fn retain(
    retention: Option<&Retention>,
    directory: &Path,
    cli: Option<&keeper::Executor>,
) -> Result<(), Refusal> {
    let cli = cli.map(cli_closure).transpose()?;
    let Some(retention) = retention else {
        return if cli.is_none() {
            Ok(())
        } else {
            Err(Refusal::PreparationFailed)
        };
    };
    let roots = [
        ("executor-root", Some(retention.closure.as_path())),
        ("agent-cli-root", cli),
    ];
    for (name, closure) in roots {
        let Some(closure) = closure else { continue };
        let status = tokio::time::timeout(
            Duration::from_secs(20),
            tokio::process::Command::new(&retention.command)
                .arg("--add-root")
                .arg(directory.join(name))
                .arg("--indirect")
                .arg("--realise")
                .arg(closure)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await;
        if !matches!(status, Ok(Ok(status)) if status.success()) {
            return Err(Refusal::PreparationFailed);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(command: &str) -> keeper::Executor {
        keeper::Executor {
            command: command.into(),
            sha256: "a".repeat(64),
            version: "1.0.0".into(),
        }
    }

    #[test]
    fn cli_retention_requires_a_direct_store_binary() {
        assert_eq!(
            cli_closure(&cli("/nix/store/old-cli/bin/cowboy")).unwrap(),
            Path::new("/nix/store/old-cli")
        );
        for command in [
            "/tmp/bin/cowboy",
            "/nix/store/old-cli/other/cowboy",
            "/nix/store/old-cli/../bin/cowboy",
        ] {
            assert!(cli_closure(&cli(command)).is_err());
        }
    }

    #[tokio::test]
    async fn cli_retention_cannot_silently_skip_a_missing_store_command() {
        let root = tempfile::tempdir().unwrap();
        retain(None, root.path(), None).await.unwrap();
        assert!(
            retain(
                None,
                root.path(),
                Some(&cli("/nix/store/old-cli/bin/cowboy"))
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn retention_uses_the_saved_cli_and_propagates_failure() {
        let root = tempfile::tempdir().unwrap();
        let command = root.path().join("store");
        std::fs::write(
            &command,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$0.args\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let retention = Retention {
            command: command.clone(),
            closure: "/nix/store/executor".into(),
        };
        retain(
            Some(&retention),
            root.path(),
            Some(&cli("/nix/store/old-cli/bin/cowboy")),
        )
        .await
        .unwrap();
        let args = std::fs::read_to_string(command.with_extension("args")).unwrap();
        assert_eq!(
            args,
            format!(
                "--add-root\n{}/executor-root\n--indirect\n--realise\n/nix/store/executor\n--add-root\n{}/agent-cli-root\n--indirect\n--realise\n/nix/store/old-cli\n",
                root.path().display(),
                root.path().display()
            )
        );
        std::fs::write(&command, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(
            retain(
                Some(&retention),
                root.path(),
                Some(&cli("/nix/store/old-cli/bin/cowboy"))
            )
            .await
            .is_err()
        );
    }
}
