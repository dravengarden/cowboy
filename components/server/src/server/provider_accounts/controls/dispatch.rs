//! The account gate of one dispatch: retired legacy cards, admission,
//! account state and uncertain effects. Sessions of one account run
//! concurrently and nothing is reserved here
//! (`docs/antigravity-real-accounts-design.md`, "Revision 5 simplification"
//! and "Revision 6: concurrent sessions per account").
use super::*;

/// The account gate of every dispatch of `session` (user prompts and
/// supervised follow-ups alike). `Err` is the session's visible wait.
pub(in crate::server) async fn before_dispatch(
    state: &Arc<AppState>,
    session: &str,
    explicit: bool,
) -> Result<(), String> {
    let info = state.hub.session_info(session);
    // A retired legacy card has no credential left anywhere to resume on.
    if info.as_ref().is_some_and(|info| {
        info.meta.provider_account_id.is_default()
            && super::super::retirement::retired(state, &info.meta.provider)
    }) {
        return Err(wait(super::super::retirement::SESSION_RETIRED));
    }
    let governed = info.is_some_and(|info| governed(&info.meta));
    if !governed {
        return Ok(());
    }
    of(state)
        .ok_or_else(|| wait("account controls need the durable store"))?
        .before_dispatch(session, explicit)
        .await
}

/// Hand one prompt of `session` to its runtime through the account gate
/// (supervised follow-ups). `send` runs only after the gate admitted it.
pub(in crate::server) async fn handoff<T>(
    state: &Arc<AppState>,
    session: &str,
    send: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    before_dispatch(state, session, false).await?;
    send()
}

impl Controls {
    /// The account gate of one dispatch, read from the store: admission,
    /// account state (an explicit owner send after a rate limit ended
    /// resumes the account) and uncertain effects.
    pub(in crate::server) async fn before_dispatch(
        &self,
        session: &str,
        explicit: bool,
    ) -> Result<(), String> {
        let Some((meta, _)) = self.env.session(session) else {
            return Ok(());
        };
        if !governed(&meta) {
            return Ok(());
        }
        if meta.provider_account_id.is_default() {
            return Err(wait("a named Antigravity account is required"));
        }
        self.refresh().await.map_err(|reason| wait(&reason))?;
        match self.gate(&meta, explicit) {
            Gate::Park(reason) => Err(wait(&reason)),
            Gate::Proceed {
                clears_rate_limit: true,
            } => {
                // Only the expired rate limit that authorized this send is
                // cleared: a newer state (a block) read here is refused, and
                // one written after this read fails the revision compare in
                // the store transaction.
                let control = self
                    .cache
                    .control(meta.provider_account_id.as_str())
                    .filter(|control| {
                        control.state == AccountState::RateLimited
                            && control.until_ms.is_some_and(|until| until <= now_ms())
                    })
                    .ok_or_else(|| wait("the account's state changed; send again"))?;
                self.enter(
                    &control,
                    AccountState::Ok,
                    "owner_send_after_rate_limit",
                    None,
                    Some(control.revision),
                    false,
                )
                .await
                .map(drop)
                .map_err(|_| wait("the account's state changed; send again"))
            }
            Gate::Proceed { .. } => Ok(()),
        }
    }
}
