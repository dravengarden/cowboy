//! Process-local identities for original Close effects. No record is evicted
//! to admit another close; a receipt query never dispatches a new effect.
use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

const MAX_PENDING: usize = 2048;

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) enum Owner {
    Buffer(BufferOwner, BTreeSet<Key>),
    Unowned(u64),
}

impl Owner {
    pub(super) fn buffer(owner: &BufferOwner, keys: &HashSet<Key>) -> Self {
        Self::Buffer(owner.clone(), keys.iter().cloned().collect())
    }
}

#[derive(Clone)]
pub(super) struct Receipt {
    pub request: wire::CowboyCloseBuffers,
    pub bindings: BTreeMap<Key, u64>,
}

#[derive(Default)]
struct Inner {
    last: u64,
    pending: HashMap<Owner, Receipt>,
    retirements: VecDeque<wire::CowboyCloseBuffers>,
}

#[derive(Default)]
pub(crate) struct Registry(Mutex<Inner>);

impl Registry {
    /// Even an ID excluded from the native batch because it was shared must
    /// keep its other owners until recovery removes the pending owner's pin.
    /// Compare native IDs, including aliases outside the original key set.
    pub(super) fn ensure_unreserved(&self, ids: &HashSet<u64>) -> Result<()> {
        ensure!(
            self.0
                .lock()
                .expect("close receipts poisoned")
                .pending
                .values()
                .all(|receipt| receipt.bindings.values().all(|id| !ids.contains(id))),
            "original shared native close remains unconfirmed"
        );
        Ok(())
    }

    pub(super) fn get(&self, owner: &Owner) -> Option<Receipt> {
        self.0
            .lock()
            .expect("close receipts poisoned")
            .pending
            .get(owner)
            .cloned()
    }

    pub(super) fn begin(
        &self,
        owner: Owner,
        mut request: wire::CowboyCloseBuffers,
        bindings: BTreeMap<Key, u64>,
    ) -> Result<Receipt> {
        let mut inner = self.0.lock().expect("close receipts poisoned");
        ensure!(
            !inner.pending.contains_key(&owner),
            "original native close remains unconfirmed"
        );
        ensure!(
            inner.pending.len() + inner.retirements.len() < MAX_PENDING,
            "native close recovery capacity reached"
        );
        inner.last = inner
            .last
            .checked_add(1)
            .context("native close identities exhausted")?;
        request.operation_id = inner.last;
        let receipt = Receipt { request, bindings };
        inner.pending.insert(owner, receipt.clone());
        Ok(receipt)
    }

    pub(super) fn confirm(&self, owner: &Owner, zed: &ZedRuntime) {
        let mut inner = self.0.lock().expect("close receipts poisoned");
        let receipt = inner
            .pending
            .remove(owner)
            .expect("original native close retained");
        inner.retirements.push_back(receipt.request);
        Self::flush_inner(&mut inner, zed);
    }

    pub(super) fn flush(&self, zed: &ZedRuntime) {
        Self::flush_inner(&mut self.0.lock().expect("close receipts poisoned"), zed);
    }

    fn flush_inner(inner: &mut Inner, zed: &ZedRuntime) {
        while let Some(request) = inner.retirements.front() {
            if zed.sync.forget_close(zed, request.clone()).is_err() {
                break;
            }
            inner.retirements.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn full_transport_retains_receipt_gc_without_replaying_close() {
        let (mut zed, _) = coordinate_queries::fixture().await;
        let (transport, mut receiver) = sync_native::Transport::new();
        Arc::get_mut(&mut zed).unwrap().sync = transport;
        while zed
            .sync
            .forget_close(&zed, request(vec![3; 16], vec![99]))
            .is_ok()
        {}
        let registry = Registry::default();
        let owner = Owner::Unowned(7);
        let original = registry
            .begin(
                owner.clone(),
                request(vec![3; 16], vec![7]),
                BTreeMap::default(),
            )
            .unwrap();
        registry.confirm(&owner, &zed);
        assert!(registry.get(&owner).is_none());
        assert_eq!(registry.0.lock().unwrap().retirements.len(), 1);
        while receiver.try_recv().is_ok() {}
        registry.flush(&zed);
        assert!(registry.0.lock().unwrap().retirements.is_empty());
        let envelope = receiver.recv().await.unwrap();
        let Some(Payload::CloseRequest(request)) = envelope.payload else {
            panic!("not receipt GC");
        };
        assert_eq!(request.action, Action::Forget as i32);
        assert_eq!(request.operation_id, original.request.operation_id);
        assert_eq!(request.buffer_ids, [7]);
        assert!(receiver.try_recv().is_err());
        let next = registry
            .begin(
                owner,
                super::super::request(vec![3; 16], vec![7]),
                BTreeMap::default(),
            )
            .unwrap();
        assert!(next.request.operation_id > original.request.operation_id);
    }

    #[test]
    fn pending_receipts_are_bounded_without_eviction_or_identity_reuse() {
        let registry = Registry::default();
        for id in 1..=MAX_PENDING as u64 {
            let receipt = registry
                .begin(
                    Owner::Unowned(id),
                    request(vec![3; 16], vec![id]),
                    BTreeMap::default(),
                )
                .unwrap();
            assert_eq!(receipt.request.operation_id, id);
        }
        assert!(
            registry
                .begin(
                    Owner::Unowned(1),
                    request(vec![3; 16], vec![1]),
                    BTreeMap::default()
                )
                .is_err()
        );
        assert!(
            registry
                .begin(
                    Owner::Unowned(MAX_PENDING as u64 + 1),
                    request(vec![3; 16], vec![1]),
                    BTreeMap::default()
                )
                .is_err()
        );
        assert_eq!(
            registry
                .get(&Owner::Unowned(1))
                .unwrap()
                .request
                .operation_id,
            1
        );
    }
}
