// RaCore — ARP (RFC 826) for IPv4 over Ethernet

use super::eth::{self, ETHERTYPE_ARP};

pub const ARP_PACKET_LEN: usize = 28;
pub const ARP_FRAME_LEN: usize = eth::ETH_HDR_LEN + ARP_PACKET_LEN; // 42 bytes
const HTYPE_ETHERNET: u16 = 1;
const PTYPE_IPV4: u16 = eth::ETHERTYPE_IPV4;

pub const OP_REQUEST: u16 = 1;
pub const OP_REPLY: u16 = 2;

#[derive(Debug, Clone, Copy)]
pub struct ArpPacket {
    pub op: u16,
    pub sender_mac: [u8; 6],
    pub sender_ip: [u8; 4],
    pub target_mac: [u8; 6],
    pub target_ip: [u8; 4],
}

impl ArpPacket {
    pub fn parse(payload: &[u8]) -> Option<Self> {
        if payload.len() < ARP_PACKET_LEN {
            return None;
        }
        let htype = u16::from_be_bytes([payload[0], payload[1]]);
        let ptype = u16::from_be_bytes([payload[2], payload[3]]);
        let hlen = payload[4];
        let plen = payload[5];
        if htype != HTYPE_ETHERNET || ptype != PTYPE_IPV4 || hlen != 6 || plen != 4 {
            return None;
        }
        let op = u16::from_be_bytes([payload[6], payload[7]]);
        let mut sender_mac = [0u8; 6];
        let mut sender_ip = [0u8; 4];
        let mut target_mac = [0u8; 6];
        let mut target_ip = [0u8; 4];
        sender_mac.copy_from_slice(&payload[8..14]);
        sender_ip.copy_from_slice(&payload[14..18]);
        target_mac.copy_from_slice(&payload[18..24]);
        target_ip.copy_from_slice(&payload[24..28]);
        Some(ArpPacket {
            op,
            sender_mac,
            sender_ip,
            target_mac,
            target_ip,
        })
    }

    /// Serialize this ARP packet (28 bytes) at the start of `out`.
    pub fn write(&self, out: &mut [u8]) {
        debug_assert!(out.len() >= ARP_PACKET_LEN);
        out[0..2].copy_from_slice(&HTYPE_ETHERNET.to_be_bytes());
        out[2..4].copy_from_slice(&PTYPE_IPV4.to_be_bytes());
        out[4] = 6;
        out[5] = 4;
        out[6..8].copy_from_slice(&self.op.to_be_bytes());
        out[8..14].copy_from_slice(&self.sender_mac);
        out[14..18].copy_from_slice(&self.sender_ip);
        out[18..24].copy_from_slice(&self.target_mac);
        out[24..28].copy_from_slice(&self.target_ip);
    }
}

/// Small fixed-size ARP cache.
pub const CACHE_SIZE: usize = 8;

#[derive(Clone, Copy)]
struct CacheEntry {
    ip: [u8; 4],
    mac: [u8; 6],
    valid: bool,
}

pub struct ArpCache {
    entries: [CacheEntry; CACHE_SIZE],
    next_slot: usize,
}

impl ArpCache {
    pub const fn new() -> Self {
        ArpCache {
            entries: [CacheEntry {
                ip: [0; 4],
                mac: [0; 6],
                valid: false,
            }; CACHE_SIZE],
            next_slot: 0,
        }
    }

    pub fn insert(&mut self, ip: [u8; 4], mac: [u8; 6]) {
        // Replace existing entry for this IP if present.
        for e in self.entries.iter_mut() {
            if e.valid && e.ip == ip {
                e.mac = mac;
                return;
            }
        }
        // Otherwise overwrite next slot (round-robin eviction).
        self.entries[self.next_slot] = CacheEntry {
            ip,
            mac,
            valid: true,
        };
        self.next_slot = (self.next_slot + 1) % CACHE_SIZE;
    }

    pub fn lookup(&self, ip: [u8; 4]) -> Option<[u8; 6]> {
        self.entries
            .iter()
            .find(|e| e.valid && e.ip == ip)
            .map(|e| e.mac)
    }
}

/// Build a complete ARP-request frame (Ethernet header + ARP payload).
/// Returns 42 bytes in `out`.
pub fn build_request(out: &mut [u8], src_mac: &[u8; 6], src_ip: &[u8; 4], target_ip: &[u8; 4]) {
    debug_assert!(out.len() >= ARP_FRAME_LEN);
    eth::write_header(out, &eth::MAC_BROADCAST, src_mac, ETHERTYPE_ARP);
    let pkt = ArpPacket {
        op: OP_REQUEST,
        sender_mac: *src_mac,
        sender_ip: *src_ip,
        target_mac: [0; 6],
        target_ip: *target_ip,
    };
    pkt.write(&mut out[eth::ETH_HDR_LEN..]);
}

/// Build an ARP-reply frame in response to an incoming request.
pub fn build_reply(
    out: &mut [u8],
    our_mac: &[u8; 6],
    our_ip: &[u8; 4],
    peer_mac: &[u8; 6],
    peer_ip: &[u8; 4],
) {
    debug_assert!(out.len() >= ARP_FRAME_LEN);
    eth::write_header(out, peer_mac, our_mac, ETHERTYPE_ARP);
    let pkt = ArpPacket {
        op: OP_REPLY,
        sender_mac: *our_mac,
        sender_ip: *our_ip,
        target_mac: *peer_mac,
        target_ip: *peer_ip,
    };
    pkt.write(&mut out[eth::ETH_HDR_LEN..]);
}
