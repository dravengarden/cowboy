//! Current binding checks under the same locks as cancellation and revocation.
use super::{NativeCompletion, Receipt, Registry, Result, StorageBackend, Store};

enum BindingUse<'a> {
    Latest,
    Insert(&'a crate::core::SessionMeta),
    /// Session id and the exact release persisted on its row. A compatible
    /// later release may differ from the login's own (see native_generation).
    Existing(&'a str, &'a str, &'a str),
    TransitionFinish(&'a str, &'a str),
    Transition(&'a str, &'a super::super::transitions::Intent),
    TransitionDispatch(&'a str, &'a super::super::transitions::Intent),
    TransitionCommit(
        &'a str,
        &'a super::super::transitions::Intent,
        &'a super::super::transitions::BindingCommit,
    ),
    TransitionStop(
        &'a str,
        &'a super::super::transitions::Intent,
        &'a super::super::revocation::stops::Evidence,
    ),
}

impl Store {
    /// Call only after the exact Broker readmission ACK. The callback checks
    /// the live runtime connection; failure rolls back the finished flag.
    #[allow(dead_code)]
    pub(crate) async fn finish_native_session_transition(
        &self,
        owner: &str,
        operation: &str,
        admitted: &crate::provider_account::PreparedBinding,
        validate: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let intent = self
            .native_session_transition(owner, operation)
            .await?
            .ok_or_else(|| anyhow::anyhow!("transition_not_found"))?;
        let stopped = self
            .native_session_transition_stop(owner, operation)
            .await?
            .and_then(|value| value.tree_exit)
            .ok_or_else(|| anyhow::anyhow!("transition_stop_missing"))?;
        anyhow::ensure!(
            admitted.operation == operation
                && admitted.target == intent.target.profile
                && admitted.stopped == stopped,
            "transition_admission_scope_mismatch"
        );
        anyhow::ensure!(
            self.native_session_transition_commit(owner, operation)
                .await?
                .is_some(),
            "transition_commit_missing"
        );
        if self
            .native_session_transition_finished(owner, operation)
            .await?
        {
            return Ok(());
        }
        self.native_binding_action(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            BindingUse::TransitionFinish(operation, &intent.request.session_id),
            || std::future::ready(validate()),
        )
        .await
    }
    /// Atomic metadata commit only; the Machine fence remains in place.
    /// Caller supplies behavior from the exact signed target package.
    #[cfg(test)]
    pub(crate) async fn commit_native_session_transition(
        &self,
        owner: &str,
        operation: &str,
        commit: &super::super::transitions::BindingCommit,
    ) -> Result<()> {
        self.commit_native_session_transition_checked(owner, operation, commit, || Ok(()))
            .await
    }

    /// The Controller rechecks its live connection and trusted package here,
    /// after asynchronous admission reads and before the transaction commits.
    #[allow(dead_code, reason = "public migration remains gated")]
    pub(crate) async fn commit_native_session_transition_checked(
        &self,
        owner: &str,
        operation: &str,
        commit: &super::super::transitions::BindingCommit,
        validate: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        if let Some(old) = self
            .native_session_transition_commit(owner, operation)
            .await?
        {
            anyhow::ensure!(old == *commit, "transition_commit_conflict");
            return Ok(());
        }
        let intent = self
            .native_session_transition(owner, operation)
            .await?
            .ok_or_else(|| anyhow::anyhow!("transition_not_found"))?;
        self.native_binding_action(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            BindingUse::TransitionCommit(operation, &intent, commit),
            || std::future::ready(validate()),
        )
        .await
    }
    /// Queue one idempotent migration stop under current account, target-login
    /// and original-session admission. The callback checks the live runtime.
    #[allow(dead_code, reason = "public migration remains gated")]
    pub(crate) async fn dispatch_native_session_transition_stop<T>(
        &self,
        owner: &str,
        operation: &str,
        enqueue: impl FnOnce(&super::super::transitions::Intent) -> Result<T>,
    ) -> Result<T> {
        let intent = self
            .native_session_transition(owner, operation)
            .await?
            .ok_or_else(|| anyhow::anyhow!("transition_not_found"))?;
        self.native_binding_action(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            BindingUse::TransitionDispatch(operation, &intent),
            || std::future::ready(enqueue(&intent)),
        )
        .await
    }
    /// Call only after acknowledged quiescence and a fresh Machine tree query.
    /// The predicate must recheck the authenticated query connection under lock.
    #[allow(dead_code, reason = "public migration remains gated")]
    pub(crate) async fn record_native_session_transition_stop(
        &self,
        owner: &str,
        operation: &str,
        evidence: &super::super::revocation::stops::Evidence,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<()> {
        evidence.require_transition_stop(operation)?;
        if let Some(old) = self
            .native_session_transition_stop(owner, operation)
            .await?
        {
            anyhow::ensure!(old == *evidence, "transition_stop_conflict");
            return Ok(());
        }
        let intent = self
            .native_session_transition(owner, operation)
            .await?
            .ok_or_else(|| anyhow::anyhow!("transition_not_found"))?;
        anyhow::ensure!(
            evidence.account_id == intent.request.account_id
                && evidence.session_id == intent.request.session_id
                && evidence.machine_id == intent.request.machine_id
                && evidence.recipient_fingerprint == intent.target.recipient_fingerprint,
            "transition_stop_scope_mismatch"
        );
        self.native_binding_action(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            BindingUse::TransitionStop(operation, &intent, evidence),
            || async move {
                anyhow::ensure!(connection_is_current(), "machine_connection_changed");
                Ok(())
            },
        )
        .await
    }
    /// Persist a migration proposal while the target login is freshly admitted.
    /// Exact replay is historical; it must not redispatch any process operation.
    #[allow(dead_code, reason = "public migration remains gated")]
    pub(crate) async fn plan_native_session_transition(
        &self,
        owner: &str,
        operation: &str,
        intent: &super::super::transitions::Intent,
    ) -> Result<()> {
        anyhow::ensure!(
            crate::managed_calls::valid_id(operation),
            "invalid_transition_operation"
        );
        intent.validate()?;
        // The transaction repeats this lookup to serialize concurrent claims.
        if let Some(old) = self.native_session_transition(owner, operation).await? {
            anyhow::ensure!(old == *intent, "operation_reused");
            return Ok(());
        }
        self.native_binding_action(
            owner,
            &intent.request.target_login_operation,
            &intent.target,
            BindingUse::Transition(operation, intent),
            || std::future::ready(Ok(())),
        )
        .await
    }
    /// Commit first, then publish under a fresh admission check. If revocation
    /// wins between the two transactions, its plan already contains the row.
    /// This registers metadata only; it never starts a worker.
    #[cfg(test)]
    pub(crate) async fn register_native_session(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        session: crate::core::SessionMeta,
        hub: &crate::core::Hub,
    ) -> Result<()> {
        self.register_native_session_with_projection(
            owner,
            operation,
            completion,
            session,
            |session| {
                anyhow::ensure!(
                    hub.project_persisted_session(session),
                    "session_projection_conflict"
                );
                Ok(())
            },
        )
        .await
    }

    pub(crate) async fn register_native_session_with_projection(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        session: crate::core::SessionMeta,
        project: impl FnOnce(crate::core::SessionMeta) -> Result<()>,
    ) -> Result<()> {
        self.insert_native_session(owner, operation, completion, &session)
            .await?;
        self.with_native_login_binding(owner, operation, completion, || project(session))
            .await
    }

    /// Execute one synchronous action against a current native binding. This
    /// is not a worker lease: later revocation needs confirmed runtime cleanup.
    /// The callback must not await or reenter Store; no authorization object is
    /// returned for later reuse. Callers also check their live connection and
    /// the action's own permission before enqueuing any Machine command.
    pub(crate) async fn with_native_login_binding<T>(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.native_binding_action(owner, operation, completion, BindingUse::Latest, || {
            std::future::ready(action())
        })
        .await
    }

    /// Only for bounded transport writes or synchronous projection of an
    /// already committed binding. Hold revocation admission through the action;
    /// neither a queued frame nor a projection proves remote completion.
    pub(crate) async fn with_native_session_binding<
        T,
        F: std::future::Future<Output = Result<T>>,
    >(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        session: &str,
        action: impl FnOnce() -> F,
    ) -> Result<T> {
        self.native_binding_action(
            owner,
            operation,
            completion,
            BindingUse::Existing(
                session,
                &completion.profile.provider_version,
                &completion.profile.generation_digest,
            ),
            action,
        )
        .await
    }

    /// Admit runtime traffic for a session pinned to `version`/`digest`, which
    /// registration accepted as the login's release or a compatible later one.
    pub(crate) async fn with_native_session_release_binding<
        T,
        F: std::future::Future<Output = Result<T>>,
    >(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        session: &crate::core::SessionMeta,
        action: impl FnOnce() -> F,
    ) -> Result<T> {
        self.native_binding_action(
            owner,
            operation,
            completion,
            BindingUse::Existing(
                &session.id,
                &session.provider_version,
                &session.provider_generation_digest,
            ),
            action,
        )
        .await
    }

    /// Persist before projecting into Hub or starting a worker. Revocation
    /// captures the row under the same owner lock, including after a crash.
    #[allow(dead_code, reason = "named session admission remains rollout gated")]
    pub(crate) async fn insert_native_session(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        session: &crate::core::SessionMeta,
    ) -> Result<()> {
        let profile = &completion.profile;
        anyhow::ensure!(
            session.owner_user_id.as_deref() == Some(owner)
                && session.provider_account_id == profile.account_id
                && session.machine_id == completion.machine_id
                && session.provider == profile.provider_id
                // The release may differ from the one used to sign in when its
                // authentication contract matches; registration resolves that
                // and the Machine verifies it before launch.
                && !session.provider_version.is_empty()
                && !session.provider_generation_digest.is_empty()
                && session.provider_auth_generation == Some(profile.generation)
                && session.status == crate::core::Status::Starting,
            "native_session_scope_mismatch"
        );
        self.native_binding_action(
            owner,
            operation,
            completion,
            BindingUse::Insert(session),
            || std::future::ready(Ok(())),
        )
        .await
    }

    #[allow(clippy::too_many_lines)] // Keep both backends in one ordered admission transaction.
    async fn native_binding_action<T, F: std::future::Future<Output = Result<T>>>(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        usage: BindingUse<'_>,
        action: impl FnOnce() -> F,
    ) -> Result<T> {
        let profile = &completion.profile;
        anyhow::ensure!(
            profile.provider_id == "claude-code" && !profile.account_id.is_default(),
            "native_account_required"
        );
        let fingerprint = profile
            .identity
            .fingerprint(&profile.provider_id)
            .map_err(anyhow::Error::msg)?;
        macro_rules! admit { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            if let BindingUse::TransitionCommit(operation, _, commit) = usage {
                let old: Option<String> = sqlx::query_scalar("SELECT value FROM provider_session_transition_commits WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
                if let Some(old) = old {
                    anyhow::ensure!(serde_json::from_str::<super::super::transitions::BindingCommit>(&old)? == *commit, "transition_commit_conflict");
                    let result = action().await?;
                    tx.commit().await?;
                    return Ok(result);
                }
            }
            if let BindingUse::Transition(operation, intent) = usage {
                let old: Option<String> = sqlx::query_scalar("SELECT value FROM provider_session_transitions WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
                if let Some(old) = old {
                    anyhow::ensure!(serde_json::from_str::<super::super::transitions::Intent>(&old)? == *intent, "operation_reused");
                    let result = action().await?;
                    tx.commit().await?;
                    return Ok(result);
                }
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(!receipt.cancel_requested && receipt.dispatch_rejection.is_none() && receipt.native_logout.is_none(), "login_not_admitted");
            anyhow::ensure!(receipt.native_completion.as_ref() == Some(completion)
                && receipt.native_generation == Some(profile.generation), "native_completion_scope_mismatch");
            let revision = receipt.native_binding_revision.ok_or_else(|| anyhow::anyhow!("native_identity_not_bound"))?;
            let generation: i64 = sqlx::query_scalar("SELECT generation FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                .bind(owner).bind(profile.account_id.as_str()).bind(&completion.machine_id).fetch_one(&mut *tx).await?;
            let latest = u64::try_from(generation)?;
            anyhow::ensure!(profile.generation > 0 && if matches!(usage, BindingUse::Existing(..) | BindingUse::TransitionFinish(..)) {
                profile.generation <= latest
            } else { profile.generation == latest }, "native_generation_superseded");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&completion.machine_id).execute(&mut *tx).await?;
            anyhow::ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let recipient: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&completion.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(recipient.as_ref().is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint), "machine_enrollment_changed");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            anyhow::ensure!(registry.accounts.iter().any(|account|
                account.admits_binding(revision, registry.revision) &&
                account.id == profile.account_id && account.provider_id == profile.provider_id && account.available()
                && account.identity_fingerprint.as_ref() == Some(&fingerprint)), "native_binding_changed");
            let existing = match usage {
                BindingUse::Existing(id, version, digest) => Some((id, version, digest)),
                BindingUse::TransitionFinish(_, id) => Some((id, profile.provider_version.as_str(), profile.generation_digest.as_str())),
                _ => None,
            };
            if let Some((id, version, digest)) = existing {
                // Lock the exact persisted binding, including against concurrent
                // deletion or edits. A Hub snapshot alone cannot authorize reuse.
                let row = sqlx::query("UPDATE sessions SET id=id WHERE id=$1 AND owner_user_id=$2 AND provider_account_id=$3 AND machine_id=$4 AND provider=$5 AND provider_version=$6 AND provider_generation_digest=$7 AND provider_auth_generation=$8")
                    .bind(id).bind(owner).bind(profile.account_id.as_str()).bind(&completion.machine_id)
                    .bind(&profile.provider_id).bind(version).bind(digest)
                    .bind(i64::try_from(profile.generation)?).execute(&mut *tx).await?;
                anyhow::ensure!(row.rows_affected() == 1, "native_session_binding_changed");
            }
            if let BindingUse::TransitionFinish(transition, id) = usage {
                let changed = sqlx::query("UPDATE provider_session_transitions SET finished=TRUE WHERE owner_user_id=$1 AND operation_id=$2 AND session_id=$3 AND EXISTS (SELECT 1 FROM provider_session_transition_commits c WHERE c.owner_user_id=$1 AND c.operation_id=$2)")
                    .bind(owner).bind(transition).bind(id).execute(&mut *tx).await?;
                anyhow::ensure!(changed.rows_affected() == 1, "transition_commit_missing");
            }
            if let BindingUse::Transition(operation, intent) | BindingUse::TransitionStop(operation, intent, _) | BindingUse::TransitionDispatch(operation, intent) | BindingUse::TransitionCommit(operation, intent, _) = usage {
                let r = &intent.request;
                let row = sqlx::query("UPDATE sessions SET id=id WHERE id=$1 AND owner_user_id=$2 AND provider_account_id=$3 AND machine_id=$4 AND provider=$5 AND provider_version=$6 AND provider_generation_digest=$7 AND provider_auth_generation=$8")
                    .bind(&r.session_id).bind(owner).bind(r.account_id.as_str()).bind(&r.machine_id)
                    .bind(&r.provider).bind(&r.from_version).bind(&r.from_digest)
                    .bind(i64::try_from(r.from_generation)?).execute(&mut *tx).await?;
                anyhow::ensure!(row.rows_affected() == 1, "native_session_binding_changed");
                if matches!(usage, BindingUse::TransitionStop(..) | BindingUse::TransitionDispatch(..) | BindingUse::TransitionCommit(..)) {
                    let current: (String, bool) = sqlx::query_as("SELECT value,finished FROM provider_session_transitions WHERE owner_user_id=$1 AND operation_id=$2")
                        .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
                    anyhow::ensure!(!current.1 && serde_json::from_str::<super::super::transitions::Intent>(&current.0)? == *intent, "transition_changed");
                    if let BindingUse::TransitionCommit(_, _, commit) = usage {
                        let stopped: Option<String> = sqlx::query_scalar("SELECT value FROM provider_session_transition_stops WHERE owner_user_id=$1 AND operation_id=$2")
                            .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
                        let evidence: super::super::revocation::stops::Evidence = serde_json::from_str(&stopped.ok_or_else(|| anyhow::anyhow!("transition_stop_unconfirmed"))?)?;
                        evidence.require_transition_stop(operation)?;
                        anyhow::ensure!(evidence.account_id == r.account_id && evidence.session_id == r.session_id
                            && evidence.machine_id == r.machine_id && evidence.recipient_fingerprint == completion.recipient_fingerprint, "transition_stop_scope_mismatch");
                        sqlx::query("UPDATE sessions SET provider_version=$1,provider_generation_digest=$2,provider_auth_generation=$3,provider_behavior=$4,agent_session_id=NULL,status='exited' WHERE id=$5")
                            .bind(&profile.provider_version).bind(&profile.generation_digest).bind(i64::try_from(profile.generation)?)
                            .bind(serde_json::to_value(&commit.behavior)?).bind(&r.session_id).execute(&mut *tx).await?;
                        sqlx::query("INSERT INTO provider_session_transition_commits(owner_user_id,operation_id,value) VALUES ($1,$2,$3)")
                            .bind(owner).bind(operation).bind(serde_json::to_string(commit)?).execute(&mut *tx).await?;
                    }
                    if let BindingUse::TransitionStop(_, _, evidence) = usage {
                    let old: Option<String> = sqlx::query_scalar("SELECT value FROM provider_session_transition_stops WHERE owner_user_id=$1 AND operation_id=$2")
                        .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
                    if let Some(old) = old {
                        anyhow::ensure!(serde_json::from_str::<super::super::revocation::stops::Evidence>(&old)? == *evidence, "transition_stop_conflict");
                    } else {
                        sqlx::query("INSERT INTO provider_session_transition_stops(owner_user_id,operation_id,value) VALUES ($1,$2,$3)")
                            .bind(owner).bind(operation).bind(serde_json::to_string(evidence)?).execute(&mut *tx).await?;
                    }
                    }
                } else {
                    sqlx::query("INSERT INTO provider_session_transitions(owner_user_id,operation_id,session_id,value) VALUES ($1,$2,$3,$4)")
                        .bind(owner).bind(operation).bind(&r.session_id).bind(serde_json::to_string(intent)?)
                        .execute(&mut *tx).await?;
                }
            }
            if let BindingUse::Insert(m) = usage {
                sqlx::query("INSERT INTO sessions(id,provider,provider_version,provider_generation_digest,provider_auth_generation,provider_behavior,machine_id,workspace_id,workspace_name,workspace_source_path,cwd,title,origin,status,next_seq,system,owner_user_id,execution_binding,provider_account_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,0,$15,$16,$17,$18)")
                    .bind(&m.id).bind(&m.provider).bind(&m.provider_version).bind(&m.provider_generation_digest)
                    .bind(i64::try_from(profile.generation)?)
                    .bind(m.provider_behavior.as_ref().map(serde_json::to_value).transpose()?)
                    .bind(&m.machine_id).bind(m.workspace_id.as_deref()).bind(m.workspace_name.as_deref())
                    .bind(m.workspace_source_path.as_deref()).bind(&m.cwd).bind(crate::store::strip_nul_str(&m.title))
                    .bind(crate::store::origin_to_str(m.origin)).bind(crate::store::status_to_str(m.status))
                    .bind(m.system).bind(m.owner_user_id.as_deref())
                    .bind(m.execution_binding.as_ref().map(crate::execution_environment::ExecutionBinding::record))
                    .bind(m.provider_account_id.as_str()).execute(&mut *tx).await?;
            }
            let result = action().await?;
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => admit!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => admit!(db, "revoked_at_ms"),
        })
    }
}
