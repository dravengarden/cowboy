//! Research fixture: a bounded IP tunnel, not an application transport or VPN.
use std::{collections::VecDeque, net::SocketAddr, sync::Arc};

use bytes::BytesMut;
use gotatun::{
    noise::{Tunn, TunnResult, index_table::IndexTable, rate_limiter::RateLimiter},
    packet::Packet,
    tun::MtuWatcher,
    x25519::{PublicKey, StaticSecret},
};
use serde::Serialize;
use zerocopy::IntoBytes;

pub const CLIENT_IP: [u8; 4] = [10, 77, 0, 2];
pub const SERVER_IP: [u8; 4] = [10, 77, 0, 1];
pub const MTU: usize = 1280;
const QUEUE_LIMIT: usize = 256;

#[derive(Default, Serialize)]
pub struct Counters {
    pub accepted: u64,
    pub rejected: u64,
    pub address_rejected: u64,
    pub handshakes_sent: u64,
    pub handshakes_received: u64,
    pub network_packets: u64,
    pub error: String,
}

pub fn secret() -> [u8; 32] {
    let mut value = [0; 32];
    getrandom::fill(&mut value).expect("OS/browser CSPRNG unavailable");
    value
}

pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

pub struct Peer {
    tunnel: Tunn,
    limiter: Arc<RateLimiter>,
    remote_ip: [u8; 4],
    local_ip: [u8; 4],
    network: VecDeque<Vec<u8>>,
    payloads: VecDeque<Vec<u8>>,
    pub counters: Counters,
}

impl Peer {
    pub fn new(private: [u8; 32], public: [u8; 32], server: bool) -> Self {
        let private = StaticSecret::from(private);
        let limiter = Arc::new(RateLimiter::new(&PublicKey::from(&private), 100));
        Self {
            tunnel: Tunn::new(
                private,
                PublicKey::from(public),
                None,
                None,
                IndexTable::from_os_rng(),
                limiter.clone(),
            ),
            limiter,
            remote_ip: if server { CLIENT_IP } else { SERVER_IP },
            local_ip: if server { SERVER_IP } else { CLIENT_IP },
            network: VecDeque::new(),
            payloads: VecDeque::new(),
            counters: Counters::default(),
        }
    }

    fn enqueue(&mut self, packet: impl Into<Packet>) {
        let packet: Packet = packet.into();
        let bytes = packet.as_bytes();
        assert!(self.network.len() < QUEUE_LIMIT, "fixture send queue full");
        self.counters.network_packets += 1;
        if bytes.first() == Some(&1) {
            self.counters.handshakes_sent += 1;
        }
        self.network.push_back(bytes.to_vec());
    }

    pub fn send(&mut self, payload: &[u8], forged_source: bool) {
        assert!(payload.len() <= MTU - 28, "fixture payload exceeds MTU");
        let source = if forged_source {
            [10, 77, 0, 99]
        } else {
            self.local_ip
        };
        let builder = etherparse::PacketBuilder::ipv4(source, self.remote_ip, 64).udp(3333, 3333);
        let mut bytes = Vec::with_capacity(builder.size(payload.len()));
        builder
            .write(&mut bytes, payload)
            .expect("valid UDP payload");
        let packet = Packet::from_bytes(BytesMut::from(bytes.as_slice()));
        if let Some(packet) = self
            .tunnel
            .handle_outgoing_packet(packet, Some(&mut MtuWatcher::new(MTU as u16)))
        {
            self.enqueue(packet);
        }
    }

    fn reject(&mut self, error: impl std::fmt::Debug) {
        self.counters.rejected += 1;
        self.counters.error = format!("{error:?}");
    }

    pub fn receive(&mut self, bytes: &[u8]) {
        if bytes.len() > MTU + 64 {
            self.reject("oversized packet");
            return;
        }
        let packet = Packet::from_bytes(BytesMut::from(bytes));
        // A single fixture peer uses one bounded carrier. Verify MACs and apply
        // the engine's handshake limiter before the Noise state machine.
        let remote: SocketAddr = "127.0.0.1:51820".parse().unwrap();
        let result = match self.limiter.verify_packet(remote, packet) {
            Ok(packet) => self.tunnel.handle_incoming_packet(packet),
            Err(result) => result,
        };
        match result {
            TunnResult::WriteToNetwork(packet) => {
                if matches!(bytes.first(), Some(1 | 2)) {
                    self.counters.handshakes_received += 1;
                }
                self.enqueue(packet);
            }
            TunnResult::WriteToTunnel(packet) if !packet.is_empty() => {
                // Trim WireGuard padding to the validated inner IP total length.
                let packet = match packet.try_into_ipvx() {
                    Ok(ip) => match ip.left() {
                        Some(ip) => ip,
                        None => {
                            self.reject("fixture expects IPv4");
                            return;
                        }
                    },
                    Err(error) => {
                        self.reject(error);
                        return;
                    }
                };
                let parsed = etherparse::PacketHeaders::from_ip_slice(packet.as_bytes());
                match parsed {
                    Ok(parsed) => {
                        let allowed = matches!(parsed.ip,
                            Some(etherparse::IpHeader::Version4(ref ip, _))
                            if ip.source == self.remote_ip && ip.destination == self.local_ip)
                            && matches!(parsed.transport,
                                Some(etherparse::TransportHeader::Udp(ref udp))
                                if udp.source_port == 3333 && udp.destination_port == 3333);
                        if allowed && self.payloads.len() < QUEUE_LIMIT {
                            self.counters.accepted += 1;
                            self.payloads.push_back(parsed.payload.to_vec());
                        } else {
                            self.counters.address_rejected += 1;
                        }
                    }
                    Err(error) => self.reject(error),
                }
            }
            TunnResult::Err(error) => self.reject(error),
            _ => {}
        }
        let packets: Vec<_> = self
            .tunnel
            .get_queued_packets(&mut MtuWatcher::new(MTU as u16))
            .collect();
        for packet in packets {
            self.enqueue(packet);
        }
    }

    pub fn tick(&mut self) {
        match self.tunnel.update_timers() {
            Ok(Some(packet)) => self.enqueue(packet),
            Err(error) => self.reject(error),
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.tunnel.reset();
        self.network.clear();
        self.payloads.clear();
    }

    pub fn pop_network(&mut self) -> Option<Vec<u8>> {
        self.network.pop_front()
    }
    pub fn pop_payload(&mut self) -> Option<Vec<u8>> {
        self.payloads.pop_front()
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct BrowserPeer {
        peer: Peer,
        public: [u8; 32],
    }

    #[wasm_bindgen]
    impl BrowserPeer {
        #[wasm_bindgen(constructor)]
        pub fn new(server_public: &[u8]) -> Result<BrowserPeer, JsValue> {
            console_error_panic_hook::set_once();
            let server_public = server_public
                .try_into()
                .map_err(|_| JsValue::from_str("32-byte key required"))?;
            let private = secret();
            let public = public_key(&private);
            Ok(Self {
                peer: Peer::new(private, server_public, false),
                public,
            })
        }
        pub fn public_key(&self) -> Vec<u8> {
            self.public.to_vec()
        }
        pub fn send(&mut self, payload: &[u8], forged_source: bool) {
            self.peer.send(payload, forged_source);
        }
        pub fn receive(&mut self, packet: &[u8]) {
            self.peer.receive(packet);
        }
        pub fn tick(&mut self) {
            self.peer.tick();
        }
        pub fn reset(&mut self) {
            self.peer.reset();
        }
        pub fn network(&mut self) -> Option<Vec<u8>> {
            self.peer.pop_network()
        }
        pub fn payload(&mut self) -> Option<Vec<u8>> {
            self.peer.pop_payload()
        }
        pub fn stats(&self) -> String {
            serde_json::to_string(&self.peer.counters).unwrap()
        }
    }
}
