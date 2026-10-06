//! Content-free status of a Machine's durable state, for diagnostics only.
//!
//! The Machine reports counts and writer flags for the datasets it owns; the
//! Controller validates the reply against this closed schema before returning
//! it. It carries no Session ID, lineage value, path or record, grants nothing
//! and is not a snapshot across datasets: each field is read separately.
//! Deliberately kept out of the Machine protocol and runtime wire files, which
//! the host-only Machine releases compare byte for byte with the retained workers.

use serde::{Deserialize, Serialize};

/// Large enough for any real count, small enough to refuse a hostile one.
#[cfg(feature = "full")]
const MAX_COUNT: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Journal {
    pub writer_enabled: bool,
    pub deleted_sessions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Incarnations {
    pub writer_enabled: bool,
    pub lineages: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Continuations {
    pub pending: u64,
}

/// `None` for a dataset means the Machine does not hold it (for example the
/// cleanup continuation namespace on a build without an admitted writer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DurableState {
    pub schema: u8,
    pub deletion_journal: Option<Journal>,
    pub session_incarnations: Option<Incarnations>,
    pub cleanup_continuations: Option<Continuations>,
}

impl DurableState {
    pub(crate) const SCHEMA: u8 = 1;

    /// Decode a Machine reply, refusing unknown fields, another schema and any
    /// count beyond [`MAX_COUNT`].
    #[cfg(feature = "full")]
    pub(crate) fn from_reply(value: serde_json::Value) -> Result<Self, String> {
        let state: Self = serde_json::from_value(value).map_err(|error| error.to_string())?;
        let counts = [
            state
                .deletion_journal
                .map(|journal| journal.deleted_sessions),
            state.session_incarnations.map(|dataset| dataset.lineages),
            state.cleanup_continuations.map(|dataset| dataset.pending),
        ];
        if state.schema != Self::SCHEMA
            || counts.into_iter().flatten().any(|count| count > MAX_COUNT)
        {
            return Err("unsupported durable state report".to_owned());
        }
        Ok(state)
    }
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use serde_json::json;

    fn full() -> serde_json::Value {
        json!({
            "schema": 1,
            "deletionJournal": {"writerEnabled": true, "deletedSessions": 7},
            "sessionIncarnations": {"writerEnabled": true, "lineages": 6},
            "cleanupContinuations": {"pending": 0}
        })
    }

    #[test]
    fn a_complete_and_a_partial_report_round_trip() {
        let state = DurableState::from_reply(full()).unwrap();
        assert_eq!(state.session_incarnations.unwrap().lineages, 6);
        assert_eq!(serde_json::to_value(state).unwrap(), full());
        let partial = json!({
            "schema": 1,
            "deletionJournal": null,
            "sessionIncarnations": {"writerEnabled": false, "lineages": 0},
            "cleanupContinuations": null
        });
        let state = DurableState::from_reply(partial.clone()).unwrap();
        assert!(state.deletion_journal.is_none() && state.cleanup_continuations.is_none());
        assert_eq!(serde_json::to_value(state).unwrap(), partial);
    }

    #[test]
    fn unknown_fields_other_schemas_and_hostile_counts_are_refused() {
        let with = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut value = full();
            edit(&mut value);
            DurableState::from_reply(value)
        };
        assert!(with(&|v| v["schema"] = json!(2)).is_err());
        assert!(with(&|v| v["extra"] = json!(true)).is_err());
        assert!(with(&|v| v["deletionJournal"]["sessionIds"] = json!(["sess-1"])).is_err());
        assert!(with(&|v| v["sessionIncarnations"]["lineages"] = json!(-1)).is_err());
        assert!(with(&|v| v["sessionIncarnations"]["lineages"] = json!(MAX_COUNT + 1)).is_err());
        assert!(with(&|v| v["deletionJournal"]["writerEnabled"] = json!("yes")).is_err());
        assert!(with(&|v| v["cleanupContinuations"] = json!({})).is_err());
        assert!(DurableState::from_reply(json!("not an object")).is_err());
        // Closed means no unknown fields, not that every dataset must be listed: an
        // absent one is the same as one the Machine does not hold.
        let bare = DurableState::from_reply(json!({"schema": 1})).unwrap();
        assert!(bare.deletion_journal.is_none() && bare.session_incarnations.is_none());
        assert!(bare.cleanup_continuations.is_none());
        assert!(DurableState::from_reply(json!({})).is_err());
    }
}
