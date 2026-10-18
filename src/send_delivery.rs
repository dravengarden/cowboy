//! User intent for prompt delivery, independent of queue position.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum SendDelivery {
    #[default]
    NextTurn,
    CurrentTurn,
    Interrupt,
}

impl SendDelivery {
    /// Legacy force remains explicit interruption. A conflicting new choice
    /// must fail before admission rather than silently acquiring cancellation.
    pub fn resolve(delivery: Option<Self>, force: bool, front: bool) -> Result<Self, &'static str> {
        let resolved = delivery.unwrap_or(if force {
            Self::Interrupt
        } else {
            Self::NextTurn
        });
        if force && resolved != Self::Interrupt {
            return Err("delivery conflicts with force");
        }
        if front && resolved == Self::CurrentTurn {
            return Err("current_turn has no queue position");
        }
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::SendDelivery::*;
    use super::*;

    #[test]
    fn omission_and_front_never_interrupt() {
        assert_eq!(SendDelivery::resolve(None, false, false), Ok(NextTurn));
        assert_eq!(SendDelivery::resolve(None, false, true), Ok(NextTurn));
        assert_eq!(SendDelivery::resolve(None, true, false), Ok(Interrupt));
    }

    #[test]
    fn contradictory_intent_is_refused() {
        assert!(SendDelivery::resolve(Some(NextTurn), true, false).is_err());
        assert!(SendDelivery::resolve(Some(CurrentTurn), true, false).is_err());
        assert!(SendDelivery::resolve(Some(CurrentTurn), false, true).is_err());
        assert_eq!(
            SendDelivery::resolve(Some(CurrentTurn), false, false),
            Ok(CurrentTurn)
        );
    }

    #[test]
    fn all_surfaces_use_the_same_names_and_reject_typos() {
        for (name, delivery) in [
            ("next_turn", NextTurn),
            ("current_turn", CurrentTurn),
            ("interrupt", Interrupt),
        ] {
            assert_eq!(serde_json::to_value(delivery).unwrap(), name);
            assert_eq!(
                serde_json::from_value::<SendDelivery>(serde_json::json!(name)).unwrap(),
                delivery
            );
        }
        assert!(serde_json::from_value::<SendDelivery>(serde_json::json!("now")).is_err());
    }
}
