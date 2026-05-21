// RaCore — UDP (RFC 768)
//
// The UDP checksum covers a pseudo-header (src/dst IP + protocol + length)
// followed by the UDP header and payload, computed with the standard
// one's-complement Internet checksum. Sending checksum=0 is legal under IPv4
// — but slirp accepts proper checksums too and our test pcaps look saner
// with them populated.

use super::ipv4;

pub const UDP_HDR_LEN: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    pub fn parse(msg: &[u8]) -> Option<(Self, &[u8])> {
        if msg.len() < UDP_HDR_LEN { return None; }
        let src_port = u16::from_be_bytes([msg[0], msg[1]]);
        let dst_port = u16::from_be_bytes([msg[2], msg[3]]);
        let length = u16::from_be_bytes([msg[4], msg[5]]);
        let checksum = u16::from_be_bytes([msg[6], msg[7]]);
        let payload_end = (length as usize).min(msg.len());
        let payload = if payload_end > UDP_HDR_LEN { &msg[UDP_HDR_LEN..payload_end] } else { &[] };
        Some((UdpHeader { src_port, dst_port, length, checksum }, payload))
    }
}

/// Write a UDP header into `out[0..8]` and compute the checksum over the
/// pseudo-header + header + payload (payload is expected to already live
/// at `out[8..8+payload_len]`).
pub fn write(out: &mut [u8], src_ip: &[u8; 4], dst_ip: &[u8; 4],
             src_port: u16, dst_port: u16, payload_len: usize) {
    debug_assert!(out.len() >= UDP_HDR_LEN + payload_len);
    let total_len = (UDP_HDR_LEN + payload_len) as u16;

    out[0..2].copy_from_slice(&src_port.to_be_bytes());
    out[2..4].copy_from_slice(&dst_port.to_be_bytes());
    out[4..6].copy_from_slice(&total_len.to_be_bytes());
    out[6..8].copy_from_slice(&0u16.to_be_bytes()); // checksum placeholder

    // Build pseudo-header onto a small temporary buffer.
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(src_ip);
    pseudo[4..8].copy_from_slice(dst_ip);
    pseudo[8] = 0;
    pseudo[9] = ipv4::PROTO_UDP;
    pseudo[10..12].copy_from_slice(&total_len.to_be_bytes());

    let mut sum: u32 = 0;
    sum = add_ones_complement(sum, &pseudo);
    sum = add_ones_complement(sum, &out[..UDP_HDR_LEN + payload_len]);
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let mut csum = !(sum as u16);
    // RFC 768: a transmitted zero checksum is converted to 0xFFFF so it is
    // distinguishable from "no checksum computed".
    if csum == 0 { csum = 0xFFFF; }
    out[6..8].copy_from_slice(&csum.to_be_bytes());
}

fn add_ones_complement(mut sum: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    sum
}
