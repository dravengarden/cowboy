// SPDX-License-Identifier: GPL-3.0-or-later
//! Original native outcomes. Query/Forget never issue another LSP request or
//! acquire a buffer. No receipt eviction can make an old operation executable.
use super::*;
use parking_lot::Mutex;

const MAX_RECEIPTS: usize = 64;
const MAX_RESULT_BYTES: usize = 1024 * 1024;

struct Saved {
    query: Vec<u8>,
    response: proto::CowboyNavigationResponse,
}

struct Inner {
    instance: [u8; 16],
    receipts: HashMap<(PeerId, u64), Saved>,
    high_water: HashMap<PeerId, u64>,
}

#[derive(Clone)]
pub(super) struct Registry(Arc<Mutex<Inner>>);

impl Default for Registry {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            instance: rand::random(),
            receipts: HashMap::default(),
            high_water: HashMap::default(),
        })))
    }
}

pub(super) enum Decision {
    Reply(proto::CowboyNavigationResponse),
    Execute(Ticket),
}

pub(super) struct Ticket {
    registry: Registry,
    key: (PeerId, u64),
}

impl Registry {
    pub(super) fn probe(&self) -> proto::CowboyNavigationResponse {
        proto::CowboyNavigationResponse {
            protocol: 2,
            instance: self.0.lock().instance.to_vec(),
            outcome: Outcome::Supported as i32,
            ..Default::default()
        }
    }

    pub(super) fn begin(
        &self,
        request: &proto::CowboyNavigation,
        peer: PeerId,
    ) -> Result<Decision> {
        let mut inner = self.0.lock();
        anyhow::ensure!(
            request.protocol == 2
                && request.instance == inner.instance
                && request.operation_id > 0
                && (0..=2).contains(&request.action)
                && !request.query.is_empty()
                && request.query.len() <= 8192,
            "invalid original navigation operation"
        );
        let key = (peer, request.operation_id);
        let mut response = proto::CowboyNavigationResponse {
            protocol: 2,
            instance: inner.instance.to_vec(),
            operation_id: request.operation_id,
            outcome: Outcome::Unknown as i32,
            ..Default::default()
        };
        if let Some(saved) = inner.receipts.get(&key) {
            anyhow::ensure!(saved.query == request.query, "navigation operation changed");
            if request.action != 2 || saved.response.outcome == Outcome::Pending as i32 {
                return Ok(Decision::Reply(saved.response.clone()));
            }
        }
        let admitted = inner
            .high_water
            .get(&peer)
            .is_some_and(|last| request.operation_id <= *last);
        if request.action == 2 {
            if admitted {
                inner.receipts.remove(&key);
                response.outcome = Outcome::Forgotten as i32;
            }
            return Ok(Decision::Reply(response));
        }
        if request.action == 1 || admitted {
            return Ok(Decision::Reply(response));
        }
        if inner.receipts.len() >= MAX_RECEIPTS
            || (!inner.high_water.contains_key(&peer) && inner.high_water.len() >= 64)
        {
            response.outcome = Outcome::Capacity as i32;
            return Ok(Decision::Reply(response));
        }
        response.outcome = Outcome::Pending as i32;
        inner.high_water.insert(peer, request.operation_id);
        inner.receipts.insert(
            key,
            Saved {
                query: request.query.clone(),
                response,
            },
        );
        Ok(Decision::Execute(Ticket {
            registry: self.clone(),
            key,
        }))
    }
}

impl Ticket {
    pub(super) fn complete_result(&self, result: &[u8]) {
        let mut inner = self.registry.0.lock();
        if let Some(saved) = inner.receipts.get_mut(&self.key)
            && saved.response.outcome == Outcome::Pending as i32
        {
            if result.len() > MAX_RESULT_BYTES {
                saved.response.outcome = Outcome::Unknown as i32;
            } else {
                saved.response.outcome = Outcome::Complete as i32;
                saved.response.result = result.to_vec();
            }
        }
    }

    pub(super) fn refused(&self, reason: Refusal) {
        let mut inner = self.registry.0.lock();
        if let Some(saved) = inner.receipts.get_mut(&self.key)
            && saved.response.outcome == Outcome::Pending as i32
        {
            saved.response.outcome = Outcome::Refused as i32;
            saved.response.refusal = reason as i32;
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut inner = self.registry.0.lock();
        if let Some(saved) = inner.receipts.get_mut(&self.key)
            && saved.response.outcome == Outcome::Pending as i32
        {
            saved.response.outcome = Outcome::Unknown as i32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(registry: &Registry, id: u64) -> proto::CowboyNavigation {
        proto::CowboyNavigation {
            protocol: 2,
            instance: registry.probe().instance,
            operation_id: id,
            query: vec![1, 2, 3],
            ..Default::default()
        }
    }

    fn reply(
        registry: &Registry,
        request: &proto::CowboyNavigation,
        peer: PeerId,
    ) -> proto::CowboyNavigationResponse {
        match registry.begin(request, peer).unwrap() {
            Decision::Reply(response) => response,
            Decision::Execute(_) => panic!("query/replay must not execute"),
        }
    }

    #[test]
    fn completed_effect_survives_caller_drop_and_retirement_never_replays() {
        let registry = Registry::default();
        let request = request(&registry, 1);
        let peer = PeerId::default();
        let Decision::Execute(ticket) = registry.begin(&request, peer).unwrap() else {
            panic!("first execution")
        };
        assert_eq!(
            reply(&registry, &request, peer).outcome,
            Outcome::Pending as i32
        );
        let mut forget = request.clone();
        forget.action = 2;
        assert_eq!(
            reply(&registry, &forget, peer).outcome,
            Outcome::Pending as i32
        );
        ticket.complete_result(&[5, 6, 7]);
        drop(ticket);
        let mut query = request.clone();
        query.action = 1;
        let observed = reply(&registry, &query, peer);
        assert_eq!(observed.outcome, Outcome::Complete as i32);
        assert_eq!(observed.result, vec![5, 6, 7]);
        assert_eq!(reply(&registry, &request, peer).result, observed.result);
        assert_eq!(
            reply(&registry, &forget, peer).outcome,
            Outcome::Forgotten as i32
        );
        assert_eq!(
            reply(&registry, &request, peer).outcome,
            Outcome::Unknown as i32
        );
        assert_eq!(
            reply(&registry, &query, peer).outcome,
            Outcome::Unknown as i32
        );
    }

    #[test]
    fn exact_instance_peer_and_query_bind_the_original_effect() {
        let registry = Registry::default();
        let request = request(&registry, 1);
        let peer = PeerId::default();
        let Decision::Execute(ticket) = registry.begin(&request, peer).unwrap() else {
            panic!("execute")
        };
        let mut query = request.clone();
        query.action = 1;
        let other = PeerId { owner_id: 1, id: 2 };
        assert_eq!(
            reply(&registry, &query, other).outcome,
            Outcome::Unknown as i32
        );
        query.query.push(4);
        assert!(registry.begin(&query, peer).is_err());
        query = request.clone();
        query.instance[0] ^= 1;
        assert!(registry.begin(&query, peer).is_err());
        drop(ticket);
        assert_eq!(
            reply(&registry, &request, peer).outcome,
            Outcome::Unknown as i32
        );
    }

    #[test]
    fn capacity_never_evicts_original_outcomes_or_advances_on_foreign_forget() {
        let registry = Registry::default();
        let peer = PeerId::default();
        let mut future = request(&registry, 100);
        future.action = 2;
        assert_eq!(
            reply(&registry, &future, peer).outcome,
            Outcome::Unknown as i32
        );
        for id in 1..=MAX_RECEIPTS as u64 {
            let Decision::Execute(ticket) = registry.begin(&request(&registry, id), peer).unwrap()
            else {
                panic!("execute")
            };
            ticket.refused(Refusal::Budget);
        }
        assert_eq!(
            reply(&registry, &request(&registry, 65), peer).outcome,
            Outcome::Capacity as i32
        );
        let mut first = request(&registry, 1);
        first.action = 1;
        assert_eq!(
            reply(&registry, &first, peer).outcome,
            Outcome::Refused as i32
        );
        first.action = 2;
        assert_eq!(
            reply(&registry, &first, peer).outcome,
            Outcome::Forgotten as i32
        );
        assert!(matches!(
            registry.begin(&request(&registry, 65), peer).unwrap(),
            Decision::Execute(_)
        ));
    }
}
