//! Validate the CLI saved with an incarnation, never the current Machine CLI.
use super::*;

pub(super) fn validate_agent_cli(contract: &LaunchContract) -> Result<()> {
    if let Some(cli) = &contract.agent_cli {
        let directory = wire::agent_cli_directory(cli).map_err(anyhow::Error::msg)?;
        ensure!(
            contract.managed.is_none(),
            "managed execution cannot pin an agent CLI"
        );
        ensure!(
            contract
                .environment
                .get("PATH")
                .and_then(|path| path.split(':').next())
                == Some(directory),
            "agent CLI differs from target search path"
        );
    }
    Ok(())
}

pub(super) fn annotate(initialization: &mut Value, contract: &LaunchContract) -> Result<()> {
    // The executor cannot assert the keeper-owned CLI identity, even on legacy launches.
    if let Some(object) = initialization.as_object_mut() {
        object.remove(wire::AGENT_CLI_INITIALIZATION_KEY);
    }
    if let Some(cli) = &contract.agent_cli {
        initialization
            .as_object_mut()
            .context("executor initialization is not an object")?
            .insert(
                wire::AGENT_CLI_INITIALIZATION_KEY.to_owned(),
                serde_json::to_value(cli)?,
            );
    }
    Ok(())
}

pub(super) async fn verify(identity: &wire::Executor) -> Result<()> {
    let mut file = std::fs::File::open(&identity.command)?;
    let expected = identity.sha256.clone();
    let digest = tokio::task::spawn_blocking(move || {
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).map(|_| format!("{:x}", hash.finalize()))
    })
    .await??;
    ensure!(digest == expected, "execution component digest differs");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> LaunchContract {
        let cli = wire::Executor {
            command: "/nix/store/fixture/bin/cowboy".into(),
            sha256: "a".repeat(64),
            version: "1.0.0".into(),
        };
        LaunchContract {
            schema: 1,
            session_id: "fixture".into(),
            binding: crate::execution_environment::fixture().decode().unwrap(),
            executor: cli.clone(),
            agent_cli: Some(cli),
            capability: "a".repeat(64),
            environment: BTreeMap::from([(
                "PATH".into(),
                "/nix/store/fixture/bin:/usr/bin".into(),
            )]),
            call_context: None,
            managed: None,
        }
    }

    use std::collections::BTreeMap;

    #[test]
    fn recorded_agent_cli_requires_the_default_search_path() {
        let mut contract = contract();
        validate_agent_cli(&contract).unwrap();
        contract
            .environment
            .insert("PATH".into(), "/usr/bin:/nix/store/fixture/bin".into());
        assert!(validate_agent_cli(&contract).is_err());
        contract.environment.clear();
        assert!(validate_agent_cli(&contract).is_err());
        contract.agent_cli = None;
        validate_agent_cli(&contract).unwrap();
        let serialized = serde_json::to_value(&contract).unwrap();
        assert!(serialized.get("agent_cli").is_none());
        assert!(
            serde_json::from_value::<LaunchContract>(serialized)
                .unwrap()
                .agent_cli
                .is_none()
        );
    }

    #[test]
    fn cli_annotation_replaces_executor_claim_and_erases_unpinned_claim() {
        let mut contract = contract();
        let mut initialization =
            json!({"cowboyAgentCli":{"command":"/forged/cowboy"},"capabilities":{"read":true}});
        annotate(&mut initialization, &contract).unwrap();
        assert_eq!(
            initialization[wire::AGENT_CLI_INITIALIZATION_KEY],
            serde_json::to_value(contract.agent_cli.as_ref().unwrap()).unwrap()
        );
        assert_eq!(initialization["capabilities"], json!({"read":true}));
        contract.agent_cli = None;
        annotate(&mut initialization, &contract).unwrap();
        assert!(
            initialization
                .get(wire::AGENT_CLI_INITIALIZATION_KEY)
                .is_none()
        );
        assert_eq!(initialization["capabilities"], json!({"read":true}));
    }

    #[test]
    fn agent_cli_prepend_preserves_target_path_and_rejects_ambiguous_identity() {
        let mut contract = contract();
        contract
            .environment
            .insert("PATH".into(), "/usr/bin".into());
        let cli = contract.agent_cli.as_mut().unwrap();
        wire::prepend_agent_cli(cli, &mut contract.environment).unwrap();
        assert_eq!(
            contract.environment["PATH"],
            "/nix/store/fixture/bin:/usr/bin"
        );
        for command in [
            "cowboy",
            "/tmp/a:b/cowboy",
            "/tmp/../bin/cowboy",
            "/bin/other",
        ] {
            cli.command = command.into();
            assert!(wire::agent_cli_directory(cli).is_err());
        }
    }

    #[test]
    fn managed_execution_rejects_an_agent_cli_pin() {
        let mut contract = contract();
        contract.managed = Some(wire::ManagedExecution {
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
            round_path: "/private/round.json".into(),
        });
        assert!(validate_agent_cli(&contract).is_err());
        contract.agent_cli = None;
        validate_agent_cli(&contract).unwrap();
    }

    #[tokio::test]
    async fn agent_cli_digest_is_checked_against_file_contents() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cowboy");
        std::fs::write(&path, b"first").unwrap();
        let mut cli = contract().agent_cli.unwrap();
        cli.command = path.to_str().unwrap().into();
        cli.sha256 = format!("{:x}", Sha256::digest(b"first"));
        verify(&cli).await.unwrap();
        std::fs::write(&path, b"changed").unwrap();
        assert!(verify(&cli).await.is_err());
    }
}
