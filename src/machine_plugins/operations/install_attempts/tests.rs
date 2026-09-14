use super::*;
use crate::machine_protocol::plugin_install::{InstallRejection, fixture};

#[test]
fn lookup_does_not_adopt_and_duplicate_identity_never_reopens_execution() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(DIRECTORY);
    let journal = Attempts::open(path.clone()).unwrap();
    let step = fixture();
    assert_eq!(journal.query(&step), InstallLookup::NotFound {});
    assert!(!path.exists());
    let mut pending = journal.begin(&step).unwrap();
    assert!(journal.begin(&step).is_err());
    assert!(journal.ensure_unfenced("victoria").is_err());
    assert!(journal.ensure_unfenced("unrelated").is_ok());
    journal
        .advance(
            &mut pending,
            InstallOutcome::Rejected {
                reason: InstallRejection::Expired,
            },
        )
        .unwrap();
    let bytes = fs::read(path.join(format!("{}.json", step.key().unwrap()))).unwrap();
    drop(journal);
    let reopened = Attempts::open(path.clone()).unwrap();
    assert!(matches!(reopened.query(&step), InstallLookup::Found { .. }));
    assert!(reopened.begin(&step).is_err());
    assert!(reopened.ensure_legacy_allowed().is_err());
    assert!(reopened.ensure_unfenced("victoria").is_ok());
    let mut changed = step.clone();
    changed.expires_at_ms += 1;
    assert_eq!(
        reopened.query(&changed),
        unavailable(InstallUnavailable::IdentityConflict)
    );
    assert_eq!(
        fs::read(path.join(format!("{}.json", step.key().unwrap()))).unwrap(),
        bytes
    );
}

#[test]
fn every_pending_phase_reopens_as_fenced_evidence_without_completion_authority() {
    for phase in [
        InstallPhase::Prepared,
        InstallPhase::Staging,
        InstallPhase::Activating,
        InstallPhase::ProjectingAuthentication,
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        let journal = Attempts::open(path.clone()).unwrap();
        let mut step = fixture();
        step.plugin_kind = cowboy_plugin_sdk::PluginKind::AgentProvider;
        let mut pending = journal.begin(&step).unwrap();
        for next in [
            InstallPhase::Staging,
            InstallPhase::Activating,
            InstallPhase::ProjectingAuthentication,
        ] {
            if pending.receipt().outcome == (InstallOutcome::Pending { phase }) {
                break;
            }
            journal
                .advance(&mut pending, InstallOutcome::Pending { phase: next })
                .unwrap();
        }
        let file = path.join(format!("{}.json", step.key().unwrap()));
        let bytes = fs::read(&file).unwrap();
        drop(journal);
        for _ in 0..2 {
            let reopened = Attempts::open(path.clone()).unwrap();
            assert!(reopened.ensure_unfenced("victoria").is_err());
            assert!(reopened.advance(&mut pending, InstallOutcome::Unknown { phase, reason: crate::machine_protocol::plugin_install::InstallUncertainty::Interrupted }).is_err());
            assert_eq!(fs::read(&file).unwrap(), bytes);
            assert!(reopened.begin(&step).is_err());
        }
    }
}

#[test]
fn failed_intent_and_completion_flushes_poison_all_mutation_paths() {
    for after in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        let journal = Attempts::open(path.clone()).unwrap();
        let step = fixture();
        let mut pending = after.then(|| journal.begin(&step).unwrap());
        if after {
            fs::rename(&path, root.path().join("retained")).unwrap();
        }
        fs::write(&path, b"not a directory").unwrap();
        if let Some(pending) = &mut pending {
            assert!(
                journal
                    .advance(
                        pending,
                        InstallOutcome::Pending {
                            phase: InstallPhase::Staging
                        }
                    )
                    .is_err()
            );
        } else {
            assert!(journal.begin(&step).is_err());
        }
        assert_eq!(
            journal.query(&step),
            unavailable(InstallUnavailable::Storage)
        );
        assert!(journal.ensure_unfenced("unrelated").is_err());
        assert!(journal.ensure_legacy_allowed().is_err());
    }
}

#[test]
fn corrupt_future_oversized_and_symlinked_evidence_fail_closed() {
    for case in ["checksum", "schema", "identity", "oversized", "symlink"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        let journal = Attempts::open(path.clone()).unwrap();
        let step = fixture();
        journal.begin(&step).unwrap();
        drop(journal);
        let file = path.join(format!("{}.json", step.key().unwrap()));
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        match case {
            "checksum" => {
                json["evidence_digest"] = digest(b"wrong").into();
            }
            "schema" => {
                json["schema"] = 99.into();
            }
            "identity" => {
                fs::rename(&file, path.join("other.json")).unwrap();
            }
            "oversized" => {
                fs::write(
                    &file,
                    vec![b' '; usize::try_from(MAX_ATTEMPT_BYTES).unwrap() + 1],
                )
                .unwrap();
            }
            "symlink" => {
                let retained = root.path().join("retained.json");
                fs::rename(&file, &retained).unwrap();
                symlink(retained, &file).unwrap();
            }
            _ => unreachable!(),
        }
        if matches!(case, "checksum" | "schema") {
            fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
        }
        assert!(Attempts::open(path).is_err(), "{case}");
    }
}
