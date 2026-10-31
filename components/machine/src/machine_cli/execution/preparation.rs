//! Target-owned validation of storage aliases; the durable binding is immutable.
use super::*;
use crate::execution_environment::PreparationV1;

pub(super) fn same_source(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right && left.is_dir(),
        _ => false,
    }
}

pub(super) fn accepts_source_alias(intent: &PreparationV1, binding: &BindingV1) -> bool {
    if intent.validate().is_err()
        || !same_source(&intent.source_path, &binding.workspace.source_path)
    {
        return false;
    }
    let mut resolved = intent.clone();
    resolved
        .source_path
        .clone_from(&binding.workspace.source_path);
    resolved.accepts(binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_physical_source_preparation_reuses_the_same_keeper() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let alias = root.path().join("alias");
        std::fs::create_dir(&source).unwrap();
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let executor = keeper::Executor {
            command: "/unused-executor".into(),
            sha256: "a".repeat(64),
            version: "1.0.0".into(),
        };
        let manager = Manager::new(
            Some(format!("svc-{}", "1".repeat(32))),
            "target".into(),
            root.path(),
            Some(Configuration {
                schema: 1,
                host_command: "/unused-keeper".into(),
                executor: executor.clone(),
                retention: None,
            }),
            false,
        )
        .unwrap();
        let mut binding = crate::execution_environment::fixture().decode().unwrap();
        binding.environment.machine_id = "target".into();
        binding.environment.incarnation = random_id();
        binding.workspace.source_path = source.display().to_string();
        let workspaces = vec![MachineWorkspace {
            id: binding.workspace.id.clone(),
            display_name: "Fixture".into(),
            canonical_path: alias.display().to_string(),
        }];
        let directory = manager.root.join("session");
        std::fs::create_dir_all(&directory).unwrap();
        let contract = LaunchContract {
            schema: 1,
            session_id: "session".into(),
            binding: binding.clone(),
            executor,
            capability: "fixture".into(),
            environment: BTreeMap::new(),
            call_context: None,
            managed: None,
        };
        let path = directory.join("contract.json");
        let original = serde_json::to_vec(&contract).unwrap();
        std::fs::write(&path, &original).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let socket = control_socket(&directory, &binding).unwrap();
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let scope = Scope::from_binding(&binding);
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let request: keeper::Request = serde_json::from_str(&line).unwrap();
            assert!(matches!(request.command, keeper::Command::Describe));
            assert_eq!(request.scope, scope);
            let response = keeper::Response::Ready {
                scope,
                initialization: serde_json::json!({}),
                event_cursor: 0,
            };
            stream
                .get_mut()
                .write_all(format!("{}\n", serde_json::to_string(&response).unwrap()).as_bytes())
                .await
                .unwrap();
        });
        assert_eq!(
            manager
                .prepare(
                    "session",
                    &binding.workspace.id,
                    binding.runtime.clone(),
                    None,
                    &workspaces
                )
                .await
                .unwrap(),
            binding
        );
        peer.await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(root.path(), &alias).unwrap();
        assert_eq!(
            manager
                .prepare(
                    "session",
                    &binding.workspace.id,
                    binding.runtime.clone(),
                    None,
                    &workspaces
                )
                .await,
            Err(Refusal::IdentityMismatch)
        );
        std::fs::remove_file(socket.parent().unwrap()).unwrap();
    }

    #[test]
    fn storage_alias_preserves_all_other_preparation_fences() {
        let root = tempfile::tempdir().unwrap();
        let physical = root.path().join("physical");
        let alias = root.path().join("alias");
        std::fs::create_dir(&physical).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let mut binding = crate::execution_environment::fixture().decode().unwrap();
        binding.workspace.source_path = physical.display().to_string();
        let intent = PreparationV1 {
            schema: 1,
            phase: "preparing".into(),
            runtime: binding.runtime.clone(),
            machine_id: binding.environment.machine_id.clone(),
            workspace_id: binding.workspace.id.clone(),
            source_path: alias.display().to_string(),
            executor_digest: binding.environment.executor_digest.clone(),
        };
        assert!(!intent.accepts(&binding));
        assert!(accepts_source_alias(&intent, &binding));
        let mut changed = binding.clone();
        changed.runtime.cwd.push_str("/other");
        assert!(!accepts_source_alias(&intent, &changed));
        changed = binding.clone();
        changed.environment.executor_digest = format!("sha256:{}", "f".repeat(64));
        assert!(!accepts_source_alias(&intent, &changed));
        changed = binding.clone();
        changed.workspace.id.push_str("-other");
        assert!(!accepts_source_alias(&intent, &changed));
        std::fs::remove_file(&alias).unwrap();
        assert!(!accepts_source_alias(&intent, &binding));
        std::os::unix::fs::symlink(root.path(), &alias).unwrap();
        assert!(!accepts_source_alias(&intent, &binding));
    }
}
