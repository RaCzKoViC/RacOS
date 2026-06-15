// RaCore — IPv4 header (RFC 791) — no options support

pub const IPV4_HDR_LEN: usize = 20;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_TCP: u8 = 6;

#[derive(Debug, Clone, Copy)]
pub struct Ipv4Header {
    pub ihl: u8,
    pub total_len: u16,
    pub identification: u16,
    pub flags_fragment: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src: [u8; 4],
    pub dst: [u8; 4],
}

impl Ipv4Header {
    /// Parse a 20-byte IPv4 header (no options). Returns header + payload slice.
    pub fn parse(packet: &[u8]) -> Option<(Self, &[u8])> {
        if packet.len() < IPV4_HDR_LEN {
            return None;
        }
        let ver_ihl = packet[0];
        let version = ver_ihl >> 4;
        let ihl = ver_ihl & 0x0F;
        if version != 4 || ihl < 5 {
            return None;
        }
        let header_len = (ihl as usize) * 4;
        if packet.len() < header_len {
            return None;
        }

        let total_len = u16::from_be_bytes([packet[2], packet[3]]);
        let identification = u16::from_be_bytes([packet[4], packet[5]]);
        let flags_fragment = u16::from_be_bytes([packet[6], packet[7]]);
        let ttl = packet[8];
        let protocol = packet[9];
        let checksum = u16::from_be_bytes([packet[10], packet[11]]);
        let mut src = [0u8; 4];
        let mut dst = [0u8; 4];
        src.copy_from_slice(&packet[12..16]);
        dst.copy_from_slice(&packet[16..20]);

        // We accept the packet even if length fields and slice disagree; the caller can decide.
        let payload_end = (total_len as usize).min(packet.len());
        let payload = if payload_end > header_len {
            &packet[header_len..payload_end]
        } else {
            &[]
        };
        Some((
            Ipv4Header {
                ihl,
                total_len,
                identification,
                flags_fragment,
                ttl,
                protocol,
                checksum,
                src,
                dst,
            },
            payload,
        ))
    }

    /// Write a 20-byte IPv4 header at the start of `out`, computing the checksum.
    pub fn write(
        out: &mut [u8],
        src: &[u8; 4],
        dst: &[u8; 4],
        protocol: u8,
        payload_len: usize,
        id: u16,
    ) {
        debug_assert!(out.len() >= IPV4_HDR_LEN);
        out[0] = 0x45; // version 4, IHL 5 (20 bytes)
        out[1] = 0; // DSCP/ECN = 0
        let total = (IPV4_HDR_LEN + payload_len) as u16;
        out[2..4].copy_from_slice(&total.to_be_bytes());
        out[4..6].copy_from_slice(&id.to_be_bytes());
        out[6..8].copy_from_slice(&0u16.to_be_bytes()); // flags + fragment offset
        out[8] = 64; // TTL
        out[9] = protocol;
        out[10..12].copy_from_slice(&0u16.to_be_bytes()); // checksum placeholder
        out[12..16].copy_from_slice(src);
        out[16..20].copy_from_slice(dst);

        let csum = checksum(&out[..IPV4_HDR_LEN]);
        out[10..12].copy_from_slice(&csum.to_be_bytes());
    }
}

/// Internet checksum: one's-complement sum of 16-bit words, then bitwise inverted.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}
