//! The network policy a Machine browser unit enforces (R3-15,
//! `docs/antigravity-real-accounts-design.md`, "Sign-in"), and the Machines
//! a shared browser may live on.
//!
//! An own browser names its account Provider's policy in every `Open`. A
//! share's browser is one unit for several identities, so its policy is not
//! derived from membership at all: a Machine whose Device setting
//! `browser.ipv4_only` is on runs every browser unit IPv4-only, whatever an
//! `Open` names, and reports it in its browser capability (`network`). The
//! setting is read once at the Machine's start, which also stops a surviving
//! unit started without it, so whatever such a Machine runs is IPv4-only.
//!
//! A share with any IPv4-only identity (a current member, or one whose family
//! is still bound to the profile) is therefore placed and opened only on a
//! Machine that reports that enforcement, and a current member of a
//! named-only or native-only Provider only where that Provider is installed
//! and its native login may run. What cannot be determined now (a Machine
//! not connected, an unreadable inventory, an unresolved Provider) refuses as
//! `browser_share_machine_unknown`, never as a downgrade.
use super::{BrowserAccount, Resolved};
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::machine_browser::BrowserNetwork;
use crate::provider_account::{named_only, native_only};
use crate::server::AppState;
use crate::store::MachineRecord;
use crate::store::provider_accounts::browser_shares::{self, Member, Shares};
use crate::store::provider_accounts::{Account, Registry, browser_families};
use cowboy_provider_sdk::RuntimeNetworkPolicy;

/// The share's browser lives (or would be placed) on a Machine that cannot
/// host one of its identities.
pub(in crate::server) const MACHINE_UNSUPPORTED: &str = "browser_share_machine_unsupported";
/// Whether a Machine can host the share could not be determined now (a
/// Machine not connected, an unreadable inventory or an unresolved
/// Provider): retry, nothing changed.
pub(in crate::server) const MACHINE_UNKNOWN: &str = "browser_share_machine_unknown";

/// Whether `machine_id` runs every browser unit IPv4-only, as its current
/// connection's browser capability reports; `None` while it reports none
/// (not connected, or no browser Plugin). For what a read shows only: a
/// command is bound to its connection through `enforces_ipv4_only_on`.
pub(in crate::server) fn enforces_ipv4_only(state: &AppState, machine_id: &str) -> Option<bool> {
    state
        .browsers
        .capability(machine_id)
        .map(|capability| capability.network == Some(BrowserNetwork::Ipv4OnlyV1))
}

/// Whether the Machine process behind `connection` runs every browser unit
/// IPv4-only, as that connection itself reported (the setting is applied
/// at the Machine's start, so it holds for the connection's lifetime);
/// `None` once the connection reports none or was replaced.
pub(in crate::server) fn enforces_ipv4_only_on(
    state: &AppState,
    connection: &ConnectionToken,
) -> Option<bool> {
    state
        .browsers
        .capability_on(connection)
        .map(|capability| capability.network == Some(BrowserNetwork::Ipv4OnlyV1))
}

/// Whether an identity's browser use requires an IPv4-only unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::server) enum Requirement {
    Required,
    NotRequired,
    /// Nothing to decide by: no signed release the Service holds.
    Unknown,
}

/// A Provider's requirement: named-only Providers always (Antigravity
/// signs in only IPv4-only), else what its signed releases declare.
fn provider_requirement(provider: &str, declares: impl Fn(&str) -> Option<bool>) -> Requirement {
    if named_only(provider) {
        return Requirement::Required;
    }
    match declares(provider) {
        Some(true) => Requirement::Required,
        Some(false) => Requirement::NotRequired,
        None => Requirement::Unknown,
    }
}

/// A family requires IPv4-only when any of its Providers does; it is unknown
/// only when none of its Providers has a release to decide by. Used for an
/// identity without a named account (a legacy card, or a deleted account's
/// binding), which may have used any Provider of its family.
pub(in crate::server) fn family_requirement(
    family: &str,
    declares: impl Fn(&str) -> Option<bool>,
) -> Requirement {
    let mut known = false;
    for (provider, _) in browser_families::PROVIDERS
        .iter()
        .filter(|(_, of)| *of == family)
    {
        match provider_requirement(provider, &declares) {
            Requirement::Required => return Requirement::Required,
            Requirement::NotRequired => known = true,
            Requirement::Unknown => {}
        }
    }
    if known {
        Requirement::NotRequired
    } else {
        Requirement::Unknown
    }
}

/// One identity a share's browser serves: its family (`None` when it has
/// none in the table), the Provider of its named account (`None` for a
/// legacy card or a deleted account), and whether it is a current member
/// that signs in only where its native login may run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) struct Identity {
    pub family: Option<String>,
    pub provider: Option<String>,
    pub pinned: bool,
}

impl Identity {
    /// A named account is decided by its own Provider (an unresolved one is
    /// unknown even when a sibling of its family is known), anything else by
    /// its family.
    fn requirement(&self, declares: impl Fn(&str) -> Option<bool>) -> Requirement {
        match (&self.provider, &self.family) {
            (Some(provider), _) => provider_requirement(provider, declares),
            (None, Some(family)) => family_requirement(family, declares),
            (None, None) => Requirement::Unknown,
        }
    }
}

/// What every Machine hosting a share's browser must offer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::server) struct Needs {
    /// An identity requires an IPv4-only unit.
    pub ipv4_only: bool,
    /// An identity's requirement could not be resolved: only a Machine that
    /// enforces IPv4-only can host it.
    pub unknown: bool,
    /// Providers that must be installed, and whose native login must be
    /// allowed, there.
    pub pinned: Vec<String>,
}

pub(in crate::server) fn needs_of(
    identities: &[Identity],
    declares: impl Fn(&str) -> Option<bool>,
) -> Needs {
    let mut needs = Needs::default();
    for identity in identities {
        match identity.requirement(&declares) {
            Requirement::Required => needs.ipv4_only = true,
            Requirement::NotRequired => {}
            Requirement::Unknown => needs.unknown = true,
        }
        if identity.pinned
            && let Some(provider) = &identity.provider
            && !needs.pinned.contains(provider)
        {
            needs.pinned.push(provider.clone());
        }
    }
    needs
}

/// Whether a share with `needs` may live on a Machine. `enforced`: the
/// Machine runs every unit IPv4-only (`None`: it reports nothing now);
/// `native_allowed(provider)`: that Provider is installed and its native
/// login may run there (`None`: the inventory could not be read). A
/// confirmed mismatch is reported before anything unknown.
pub(in crate::server) fn admission(
    needs: &Needs,
    enforced: Option<bool>,
    native_allowed: impl Fn(&str) -> Option<bool>,
) -> Result<(), &'static str> {
    let mut unknown = false;
    if needs.ipv4_only || needs.unknown {
        match enforced {
            Some(true) => {}
            Some(false) if needs.ipv4_only => return Err(MACHINE_UNSUPPORTED),
            Some(false) | None => unknown = true,
        }
    }
    for provider in &needs.pinned {
        match native_allowed(provider) {
            Some(true) => {}
            Some(false) => return Err(MACHINE_UNSUPPORTED),
            None => unknown = true,
        }
    }
    if unknown {
        return Err(MACHINE_UNKNOWN);
    }
    Ok(())
}

fn account<'a>(registry: &'a Registry, member: &Member) -> Option<&'a Account> {
    let Member::Account(member) = member;
    registry
        .accounts
        .iter()
        .find(|account| account.id == member.account_id)
}

/// The identity of `member` as a share's browser serves it; `current`:
/// it is a member now (a former member's binding pins no Machine).
pub(in crate::server) fn identity_of(
    member: &Member,
    family: Option<String>,
    registry: &Registry,
    current: bool,
) -> Identity {
    let provider = account(registry, member).map(|account| account.provider_id.clone());
    let pinned = current
        && provider
            .as_deref()
            .is_some_and(|provider| named_only(provider) || native_only(provider));
    Identity {
        family,
        provider,
        pinned,
    }
}

/// The identities share `share_id`'s browser serves: its current members
/// (each with the family recorded at join) and every identity whose family
/// is bound to its profile, including a retired legacy card's. A binding
/// whose member cannot be read otherwise may hold any identity's cookies:
/// the share is `browser_share_machine_unknown` until it is readable again or
/// a confirmed reset releases it.
pub(in crate::server) fn share_identities(
    shares: &Shares,
    share_id: &str,
    registry: &Registry,
) -> Result<Vec<Identity>, &'static str> {
    // A retired card's binding is a known identity of its own family (its
    // network needs still apply); any other unreadable one may be anything.
    let mut retired = Vec::new();
    for row in shares
        .unreadable_bindings
        .iter()
        .filter(|row| row.share_id == share_id)
    {
        row.retired_card().ok_or(MACHINE_UNKNOWN)?;
        retired.push(Identity {
            family: Some(row.family.clone()),
            provider: None,
            pinned: false,
        });
    }
    let current = shares
        .members_of(share_id)
        .into_iter()
        .map(|row| identity_of(&row.member, Some(row.family.clone()), registry, true));
    let bound = shares
        .bindings
        .iter()
        .filter(|row| row.share_id == share_id)
        .map(|row| identity_of(&row.member, Some(row.family.clone()), registry, false));
    Ok(current.chain(bound).chain(retired).collect())
}

/// Whether `provider` is an active Agent Provider installation on
/// `machine_id` (a Plugin of another kind under that id never is): `None`
/// when the Machine's reported Plugin inventory is absent or unreadable,
/// so a retry is asked for instead of another group.
fn installed(machines: &[MachineRecord], machine_id: &str, provider: &str) -> Option<bool> {
    let Some(machine) = machines
        .iter()
        .find(|machine| machine.id == machine_id && !machine.revoked)
    else {
        return Some(false);
    };
    let inventory = machine
        .inventory
        .get("plugins")
        .or_else(|| machine.inventory.get("providers"))?;
    let plugins: Vec<crate::machine_protocol::PluginInventory> =
        serde_json::from_value(inventory.clone()).ok()?;
    Some(plugins.iter().any(|plugin| {
        plugin.plugin_id == provider
            && plugin.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
            && plugin.state == crate::machine_protocol::PluginInstallationState::Active
    }))
}

/// The Machine check of a share's browser: its needs and the Machine
/// inventory (read once, only when a member pins its Machine; `None` when
/// it could not be read).
pub(in crate::server) struct ShareGate {
    needs: Needs,
    machines: Option<Vec<MachineRecord>>,
}

impl ShareGate {
    pub(in crate::server) async fn load(
        state: &AppState,
        owner: &str,
        share_id: &str,
    ) -> Result<Self, &'static str> {
        let store = state.store.as_ref().ok_or("unavailable")?;
        let (registry, shares) = (
            store.provider_accounts(owner).await,
            store.browser_shares(owner).await,
        );
        let (Ok(registry), Ok(shares)) = (registry, shares) else {
            return Err(MACHINE_UNKNOWN);
        };
        let identities = share_identities(&shares, share_id, &registry)?;
        Self::for_identities(state, &identities).await
    }

    pub(in crate::server) async fn for_identities(
        state: &AppState,
        identities: &[Identity],
    ) -> Result<Self, &'static str> {
        let needs = needs_of(identities, |provider| {
            state.provider_catalog.declares_ipv4_only(provider)
        });
        let machines = if needs.pinned.is_empty() {
            None
        } else {
            let store = state.store.as_ref().ok_or("unavailable")?;
            store.list_machines().await.ok()
        };
        Ok(Self { needs, machines })
    }

    /// Whether the share's browser may live on `machine_id` now, as its
    /// current connection reports; what a read shows and a join preflight.
    pub(in crate::server) fn check(
        &self,
        state: &AppState,
        machine_id: &str,
    ) -> Result<(), &'static str> {
        self.admit(state, machine_id, enforces_ipv4_only(state, machine_id))
    }

    /// Whether the share's browser may be opened, attached or driven over
    /// exactly `connection`: its enforcement as that connection itself
    /// reported, so a replacement connection is never vouched for by its
    /// predecessor's capability.
    pub(in crate::server) fn check_on(
        &self,
        state: &AppState,
        connection: &ConnectionToken,
    ) -> Result<(), &'static str> {
        self.admit(
            state,
            connection.machine_id(),
            enforces_ipv4_only_on(state, connection),
        )
    }

    fn admit(
        &self,
        state: &AppState,
        machine_id: &str,
        enforced: Option<bool>,
    ) -> Result<(), &'static str> {
        admission(&self.needs, enforced, |provider| {
            let machines = self.machines.as_deref()?;
            Some(
                state.supervisor.runtime_allowed(provider, machine_id)
                    && installed(machines, machine_id, provider)?,
            )
        })
    }
}

/// Whether the browser `resolved` uses may be attached or driven over
/// `connection` now: always for an own browser, else the share's gate
/// checked against that connection. Owner cleanup (hibernate, reset) never
/// asks this.
pub(in crate::server) async fn admitted_on(
    state: &AppState,
    resolved: &Resolved,
    connection: &ConnectionToken,
) -> Result<(), &'static str> {
    match gate(state, resolved).await? {
        Some(gate) => gate.check_on(state, connection),
        None => Ok(()),
    }
}

/// The share gate of the browser `resolved` uses now; `None` for an own
/// browser, which may live wherever its placement rules allow.
pub(in crate::server) async fn gate(
    state: &AppState,
    resolved: &Resolved,
) -> Result<Option<ShareGate>, &'static str> {
    match &resolved.share {
        Some(share) => Ok(Some(
            ShareGate::load(state, &resolved.owner, &share.share_id).await?,
        )),
        None => Ok(None),
    }
}

/// The policy an open of the browser `resolved` uses names over
/// `connection`: `ipv4_only_v1` where that connection's Machine enforces it
/// on every unit anyway;
/// elsewhere none for a share (whose gate admitted that Machine) or a
/// legacy card, and an own named browser's Provider rule as installed there
/// (a named-only Provider whose installed manifest cannot be resolved is
/// restricted too).
pub(super) async fn policy(
    state: &AppState,
    resolved: &Resolved,
    connection: &ConnectionToken,
) -> Option<BrowserNetwork> {
    let machine_id = connection.machine_id();
    if enforces_ipv4_only_on(state, connection) == Some(true) {
        return Some(BrowserNetwork::Ipv4OnlyV1);
    }
    let (None, BrowserAccount::Named(account)) = (&resolved.share, &resolved.account) else {
        return None;
    };
    let declared = match state.store.as_ref() {
        Some(store) => store.list_machines().await.ok().and_then(|machines| {
            let installed = crate::server::plugin_api::active_machine_plugin(
                &machines,
                machine_id,
                &account.provider_id,
            )
            .ok()?;
            let package = state.provider_catalog.package(
                &account.provider_id,
                &installed.plugin_version,
                &installed.generation_digest,
            )?;
            Some(package.manifest.runtime.network == Some(RuntimeNetworkPolicy::Ipv4OnlyV1))
        }),
        None => None,
    };
    declared
        .unwrap_or_else(|| named_only(&account.provider_id))
        .then_some(BrowserNetwork::Ipv4OnlyV1)
}

/// Why `member` may not join share `share_id` now, if it may not: the
/// prospective share (the incoming member with the current members and
/// bindings) cannot be hosted on a Machine the share's browser already
/// lives on. Run under the owner's share fence, which every placement of a
/// share also holds, so no placement lands between this check and the join.
/// An unknown account or family is left to the registry writer, which
/// refuses it with its own code.
pub(in crate::server) async fn join_refusal(
    state: &AppState,
    owner: &str,
    (member, share_id): (&Member, &str),
) -> Result<(), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| MACHINE_UNKNOWN)?;
    let Some(family) = browser_shares::family_of(member, &registry) else {
        return Ok(());
    };
    let placements = store
        .browser_placements(owner, &browser_shares::share_key(owner, share_id))
        .await
        .map_err(|_| MACHINE_UNKNOWN)?;
    if placements.is_empty() {
        return Ok(());
    }
    let shares = store
        .browser_shares(owner)
        .await
        .map_err(|_| MACHINE_UNKNOWN)?;
    let mut identities = share_identities(&shares, share_id, &registry)?;
    identities.push(identity_of(member, Some(family), &registry, true));
    let gate = ShareGate::for_identities(state, &identities).await?;
    for placement in placements {
        gate.check(state, &placement.machine_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(family: &str) -> Identity {
        Identity {
            family: Some(family.to_owned()),
            provider: None,
            pinned: false,
        }
    }

    fn named(family: &str, provider: &str, pinned: bool) -> Identity {
        Identity {
            family: Some(family.to_owned()),
            provider: Some(provider.to_owned()),
            pinned,
        }
    }

    #[test]
    fn a_family_requires_ipv4_only_when_any_provider_does() {
        let none = |_: &str| None;
        assert_eq!(family_requirement("google", none), Requirement::Required);
        assert_eq!(family_requirement("openai", none), Requirement::Unknown);
        assert_eq!(
            family_requirement("openai", |_| Some(false)),
            Requirement::NotRequired
        );
        // One released Provider of a family decides; unreleased ones do not.
        let one = |provider: &str| (provider == "codex-deepseek").then_some(false);
        assert_eq!(
            family_requirement("deepseek", one),
            Requirement::NotRequired
        );
        let declared = |provider: &str| Some(provider == "deepseek-harness");
        assert_eq!(
            family_requirement("deepseek", declared),
            Requirement::Required
        );
        assert_eq!(
            family_requirement("future", |_| Some(false)),
            Requirement::Unknown
        );
    }

    #[test]
    fn a_named_member_is_decided_by_its_own_provider() {
        // A known unrestricted sibling never vouches for an unresolved one.
        let one = |provider: &str| (provider == "codex-deepseek").then_some(false);
        let needs = needs_of(&[named("deepseek", "deepseek-harness", true)], one);
        assert!(needs.unknown);
        assert!(!needs.ipv4_only);
        let needs = needs_of(&[named("deepseek", "codex-deepseek", false)], one);
        assert_eq!(needs, Needs::default());
        // A named-only Provider requires it without any release.
        let needs = needs_of(&[named("google", "antigravity", true)], |_| None);
        assert!(needs.ipv4_only);
        assert_eq!(needs.pinned, vec!["antigravity".to_owned()]);
        // A former member's binding pins nothing but keeps its requirement.
        let needs = needs_of(&[named("google", "antigravity", false)], |_| None);
        assert!(needs.ipv4_only);
        assert!(needs.pinned.is_empty());
    }

    #[test]
    fn a_share_with_an_ipv4_only_identity_lives_only_on_enforcing_machines() {
        let ordinary = |_: &str| Some(false);
        let members = [card("openai"), card("xai")];
        let needs = needs_of(&members, ordinary);
        assert_eq!(needs, Needs::default());
        for enforced in [None, Some(false), Some(true)] {
            assert_eq!(admission(&needs, enforced, |_| None), Ok(()));
        }
        // A bound former Antigravity member still requires it.
        let bound = [card("openai"), card("google")];
        let needs = needs_of(&bound, ordinary);
        assert!(needs.ipv4_only);
        assert_eq!(
            admission(&needs, Some(false), |_| None),
            Err(MACHINE_UNSUPPORTED)
        );
        // A Machine reporting nothing now is not a confirmed mismatch.
        assert_eq!(admission(&needs, None, |_| None), Err(MACHINE_UNKNOWN));
        assert_eq!(admission(&needs, Some(true), |_| None), Ok(()));
    }

    #[test]
    fn an_unresolved_requirement_fails_closed_off_enforcing_machines() {
        let needs = needs_of(&[card("openai")], |_| None);
        assert!(needs.unknown);
        assert_eq!(
            admission(&needs, Some(false), |_| None),
            Err(MACHINE_UNKNOWN)
        );
        assert_eq!(admission(&needs, None, |_| None), Err(MACHINE_UNKNOWN));
        assert_eq!(admission(&needs, Some(true), |_| None), Ok(()));
        let unfamiliar = Identity {
            family: None,
            provider: None,
            pinned: false,
        };
        assert!(needs_of(&[unfamiliar], |_| Some(false)).unknown);
    }

    #[test]
    fn a_pinned_member_needs_its_native_login_machine() {
        let needs = needs_of(
            &[named("google", "antigravity", true), card("openai")],
            |_| Some(false),
        );
        assert_eq!(needs.pinned, vec!["antigravity".to_owned()]);
        assert_eq!(admission(&needs, Some(true), |_| Some(true)), Ok(()));
        assert_eq!(
            admission(&needs, Some(true), |_| Some(false)),
            Err(MACHINE_UNSUPPORTED)
        );
        // An unreadable inventory is retried, never taken as allowed.
        assert_eq!(
            admission(&needs, Some(true), |_| None),
            Err(MACHINE_UNKNOWN)
        );
        // Its native login alone never admits a Machine without enforcement.
        assert_eq!(
            admission(&needs, Some(false), |_| Some(true)),
            Err(MACHINE_UNSUPPORTED)
        );
    }

    #[test]
    fn an_unreadable_inventory_is_unknown_and_a_missing_installation_is_not() {
        let record = |inventory: serde_json::Value| MachineRecord {
            id: "ovh".to_owned(),
            display_name: "OVH".to_owned(),
            connection_mode: "outbound".to_owned(),
            platform: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            status: "online".to_owned(),
            inventory,
            last_seen_at_ms: None,
            revoked: false,
            fingerprint: None,
        };
        let empty = [record(serde_json::json!({"plugins": []}))];
        assert_eq!(installed(&empty, "ovh", "antigravity"), Some(false));
        assert_eq!(installed(&empty, "hawk", "antigravity"), Some(false));
        let absent = [record(serde_json::json!({}))];
        assert_eq!(installed(&absent, "ovh", "antigravity"), None);
        let garbled = [record(serde_json::json!({"plugins": "garbled"}))];
        assert_eq!(installed(&garbled, "ovh", "antigravity"), None); // Only an active Agent Provider installation counts.
        let plugin = |kind: &str| {
            serde_json::json!({"plugins": [{
                "plugin_id": "antigravity", "plugin_version": "1.0.0", "plugin_kind": kind,
                "generation_digest": "d", "contract_fingerprint": "f", "state": "active"}]})
        };
        let provider = [record(plugin("agent_provider"))];
        assert_eq!(installed(&provider, "ovh", "antigravity"), Some(true));
        let other = [record(plugin("workspace_extension"))];
        assert_eq!(installed(&other, "ovh", "antigravity"), Some(false));
    }

    #[test]
    fn an_unreadable_binding_keeps_its_share_unknown_until_released() {
        use crate::store::provider_accounts::browser_shares::UnreadableBinding;
        let registry = Registry::default();
        let mut shares = Shares::default();
        assert_eq!(share_identities(&shares, "s", &registry), Ok(Vec::new()));
        shares.unreadable_bindings.push(UnreadableBinding {
            share_id: "s".into(),
            family: "google".into(),
            member: "account:garbled".into(),
        });
        assert_eq!(
            share_identities(&shares, "s", &registry),
            Err(MACHINE_UNKNOWN)
        );
        // Another share is unaffected.
        assert_eq!(share_identities(&shares, "t", &registry), Ok(Vec::new()));
    }

    #[test]
    fn a_retired_card_binding_is_a_known_identity_of_its_own_family() {
        use crate::store::provider_accounts::browser_shares::UnreadableBinding;
        let registry = Registry::default();
        let mut shares = Shares::default();
        shares.unreadable_bindings.push(UnreadableBinding {
            share_id: "s".into(),
            family: "anthropic".into(),
            member: "card:anthropic".into(),
        });
        assert_eq!(
            share_identities(&shares, "s", &registry),
            Ok(vec![Identity {
                family: Some("anthropic".into()),
                provider: None,
                pinned: false,
            }])
        );
        // A card member under another family is no card identity.
        shares.unreadable_bindings.push(UnreadableBinding {
            share_id: "t".into(),
            family: "openai".into(),
            member: "card:anthropic".into(),
        });
        assert_eq!(
            share_identities(&shares, "t", &registry),
            Err(MACHINE_UNKNOWN)
        );
    }
}
