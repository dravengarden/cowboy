//! Bounded, fair observation of retained operations. This grants no authority
//! to execute, replay, retire, or deliver results to another connection.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub(super) const DEADLINE: Duration = Duration::from_secs(2);
const BATCH: usize = 4;

pub(super) fn candidates<K: Ord + Clone, T>(
    entries: &BTreeMap<K, Arc<Mutex<T>>>,
    cursor: &mut Option<K>,
    eligible: impl Fn(&T) -> bool,
) -> Vec<(K, Arc<Mutex<T>>)> {
    use std::ops::Bound::{Excluded, Unbounded};
    let mut selected = Vec::new();
    let start = cursor.clone();
    let after = entries.range((start.as_ref().map_or(Unbounded, Excluded), Unbounded));
    let before = entries
        .iter()
        .take_while(|(key, _)| start.as_ref().is_some_and(|start| *key <= start));
    for (key, entry) in after.chain(before) {
        if entry.try_lock().is_ok_and(|slot| eligible(&slot)) {
            selected.push((key.clone(), Arc::clone(entry)));
            if selected.len() == BATCH {
                break;
            }
        }
    }
    if let Some((last, _)) = selected.last() {
        *cursor = Some(last.clone());
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_unknowns_rotate_without_blocking_on_busy_entries() {
        let entries: BTreeMap<_, _> = (0..10).map(|id| (id, Arc::new(Mutex::new(true)))).collect();
        let held = entries[&2].try_lock().unwrap();
        let mut cursor = None;
        let mut next = || {
            candidates(&entries, &mut cursor, |pending| *pending)
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        assert_eq!(next(), [0, 1, 3, 4]);
        assert_eq!(next(), [5, 6, 7, 8]);
        assert_eq!(next(), [9, 0, 1, 3]);
        drop(held);
        assert_eq!(next(), [4, 5, 6, 7]);
        assert_eq!(next(), [8, 9, 0, 1]);
        assert_eq!(next(), [2, 3, 4, 5]);
    }
}
