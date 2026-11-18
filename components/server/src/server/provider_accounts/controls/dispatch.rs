//! The account gate of one dispatch: admission, account state and uncertain
//! effects. One runtime per account (the daemon's unit limit and the launch
//! gate) bounds an account to one turn, so nothing is reserved here
//! (`docs/antigravity-real-accounts-design.md`, "Revision 5 simplification").
use super::*;

/// The account gate of every dispatch of `session` (user prompts and
/// supervised follow-ups alike). `Err` is the session's visible wait.
pub(in crate::server) async fn before_dispatch(
    state: &Arc<AppState>,
    session: &str,
    explicit: bool,
) -> Result<(), String> {
    let governed = state
        .hub
        .session_info(session)
        .is_some_and(|info| governed(&info.meta));
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
                let control = self
                    .cache
                    .control(meta.provider_account_id.as_str())
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
