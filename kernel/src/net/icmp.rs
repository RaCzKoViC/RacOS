// RaCore — ICMP (RFC 792) — Echo only

use super::ipv4;

pub const ICMP_HDR_LEN: usize = 8;

pub const TYPE_ECHO_REPLY: u8 = 0;
pub const TYPE_ECHO_REQUEST: u8 = 8;

/// Build an ICMP Echo message in-place. `payload_len` is the size of the
/// optional data appended after the 8-byte header (caller-prepared).
pub fn write_echo(out: &mut [u8], echo_type: u8, identifier: u16, seq: u16, payload_len: usize) {
    debug_assert!(out.len() >= ICMP_HDR_LEN + payload_len);
    out[0] = echo_type;
    out[1] = 0;
    out[2..4].copy_from_slice(&0u16.to_be_bytes()); // checksum placeholder
    out[4..6].copy_from_slice(&identifier.to_be_bytes());
    out[6..8].copy_from_slice(&seq.to_be_bytes());

    let csum = ipv4::checksum(&out[..ICMP_HDR_LEN + payload_len]);
    out[2..4].copy_from_slice(&csum.to_be_bytes());
}

/// Parse and validate an ICMP message. Returns (type, code, identifier, seq, payload).
pub fn parse(msg: &[u8]) -> Option<(u8, u8, u16, u16, &[u8])> {
    if msg.len() < ICMP_HDR_LEN { return None; }
    if ipv4::checksum(msg) != 0 { return None; }
    let t = msg[0];
    let c = msg[1];
    let id = u16::from_be_bytes([msg[4], msg[5]]);
    let seq = u16::from_be_bytes([msg[6], msg[7]]);
    Some((t, c, id, seq, &msg[ICMP_HDR_LEN..]))
}
