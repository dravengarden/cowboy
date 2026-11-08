//! Sign-in binding: a browser opened for a native login follows that login's
//! authority under the login registry lock.

use super::*;

impl BrowserManager {
    pub(super) fn bind_login(
        &self,
        request_id: &str,
        account_id: &ProviderAccountId,
        instance_id: &str,
        incarnation: u64,
    ) -> Result<(Option<String>, i64), Reason> {
        let logins = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade())
            .ok_or(Reason::LoginMismatch)?;
        let mut logins = logins.lock();
        let login = logins.get_mut(request_id).ok_or(Reason::LoginMismatch)?;
        if login.account.as_ref() != Some(account_id)
            || login.provider != "claude-code"
            || login
                .browser
                .as_ref()
                .is_some_and(|(bound, _)| bound != instance_id)
        {
            return Err(Reason::LoginMismatch);
        }
        login.browser = Some((instance_id.to_owned(), incarnation));
        Ok((login.verification_url.clone(), login.expires_at_ms))
    }

    pub(super) fn unbind_login(&self, purpose: &Purpose, instance_id: &str) {
        let Purpose::SignIn { login_request_id } = purpose else {
            return;
        };
        let Some(logins) = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade())
        else {
            return;
        };
        if let Some(login) = logins.lock().get_mut(login_request_id)
            && login
                .browser
                .as_ref()
                .is_some_and(|(bound, _)| bound == instance_id)
        {
            login.browser = None;
        }
    }

    /// `Some(reason)` when a sign-in instance's login is no longer the active
    /// login it was bound to.
    pub(super) fn login_binding_broken(&self, instance: &Arc<Instance>) -> Option<Reason> {
        let request = instance.login_request()?;
        let logins = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade());
        let Some(logins) = logins else {
            return Some(Reason::LoginEnded);
        };
        let logins = logins.lock();
        let Some(login) = logins.get(request) else {
            return Some(Reason::LoginEnded);
        };
        let bound = login.browser.as_ref().is_some_and(|(bound, incarnation)| {
            bound == &instance.instance_id && *incarnation == instance.incarnation
        });
        (!bound || login.account.as_ref() != Some(&instance.account_id))
            .then_some(Reason::LoginMismatch)
    }

    pub(super) fn login_url(&self, request_id: &str) -> Option<String> {
        let logins = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade())?;
        let logins = logins.lock();
        logins.get(request_id)?.verification_url.clone()
    }

    pub(super) fn bound_instance(&self, request_id: &str) -> Option<Arc<Instance>> {
        self.running()
            .into_iter()
            .find(|instance| instance.login_request() == Some(request_id))
    }
}
