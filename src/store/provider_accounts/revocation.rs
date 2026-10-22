//! Immutable revocation work captured under the account admission lock.
//! Locators retain offline/deleted-session work. They are not dispatch authority
//! or proof that a process stopped, a grant was revoked, or credentials erased.
use super::{ProviderAccountId, grants, logins};
use anyhow::Result;
use serde::{Deserialize, Serialize};

mod completion;
pub mod discovery;
mod grant_admission;
#[cfg(test)]
pub mod grant_tests;
pub mod login_stops;
mod readiness;
pub mod remote_wipes;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalWipe {
    Wiped { generation: u64 },
    CancelledDiscarded,
}
pub mod stops;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationTarget {
    pub operation_id: String,
    pub machine_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionTarget {
    pub session_id: String,
    /// Missing placement must be reconciled; it does not mean no worker exists.
    pub machine_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub logins: Vec<OperationTarget>,
    pub grants: Vec<OperationTarget>,
    pub sessions: Vec<SessionTarget>,
}

impl Plan {
    pub(super) fn capture(
        account: &ProviderAccountId,
        logins: Vec<(String, String)>,
        grants: Vec<(String, String)>,
        sessions: Vec<(String, Option<String>)>,
    ) -> Result<Self> {
        let mut plan = Self {
            logins: Vec::new(),
            grants: Vec::new(),
            sessions: sessions
                .into_iter()
                .map(|(session_id, machine_id)| SessionTarget {
                    session_id,
                    machine_id,
                })
                .collect(),
        };
        for (operation_id, value) in logins {
            let receipt: logins::Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(
                &receipt.input.account_id == account,
                "login_account_mismatch"
            );
            plan.logins.push(OperationTarget {
                operation_id,
                machine_id: receipt.input.machine_id,
            });
        }
        for (operation_id, value) in grants {
            let receipt: grants::Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(&receipt.account_id == account, "grant_account_mismatch");
            anyhow::ensure!(
                receipt.operation_id == operation_id,
                "grant_operation_mismatch"
            );
            plan.grants.push(OperationTarget {
                operation_id,
                machine_id: receipt.machine_id,
            });
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_retains_grant_history_and_unresolved_placement() {
        let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let grant = grants::Receipt {
            operation_id: "grant".into(),
            account_id: account.clone(),
            provider_id: "codex".into(),
            machine_id: "offline-machine".into(),
            recipient_fingerprint: "old-enrollment".into(),
            candidate_fingerprint: "candidate".into(),
            identity_fingerprint: "identity".into(),
            expected_generation: 0,
            registry_revision: 1,
            phase: grants::Phase::Cancelled,
        };
        let encoded = serde_json::to_string(&grant).unwrap();
        let plan = Plan::capture(
            &account,
            vec![],
            vec![("grant".into(), encoded.clone())],
            vec![("session".into(), None)],
        )
        .unwrap();
        assert_eq!(
            plan.grants,
            vec![OperationTarget {
                operation_id: "grant".into(),
                machine_id: "offline-machine".into(),
            }]
        );
        assert_eq!(
            plan.sessions,
            vec![SessionTarget {
                session_id: "session".into(),
                machine_id: None
            }]
        );
        assert!(Plan::capture(&account, vec![], vec![("wrong".into(), encoded)], vec![]).is_err());
        assert!(
            Plan::capture(
                &account,
                vec![("login".into(), "broken".into())],
                vec![],
                vec![]
            )
            .is_err()
        );
    }
}
