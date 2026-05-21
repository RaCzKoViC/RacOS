// RaCore — Ethernet II frame parser/builder

pub const ETH_HDR_LEN: usize = 14;
pub const MAC_BROADCAST: [u8; 6] = [0xFF; 6];

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;

/// Parsed Ethernet II header view.
#[derive(Debug, Clone, Copy)]
pub struct EthHeader {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
}

impl EthHeader {
    /// Parse the first 14 bytes of an Ethernet II frame. Returns `None` if too short.
    pub fn parse(frame: &[u8]) -> Option<(Self, &[u8])> {
        if frame.len() < ETH_HDR_LEN { return None; }
        let mut dst = [0u8; 6];
        let mut src = [0u8; 6];
        dst.copy_from_slice(&frame[0..6]);
        src.copy_from_slice(&frame[6..12]);
        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        Some((EthHeader { dst, src, ethertype }, &frame[ETH_HDR_LEN..]))
    }
}

/// Build an Ethernet II header at the start of `out`. Writes 14 bytes.
pub fn write_header(out: &mut [u8], dst: &[u8; 6], src: &[u8; 6], ethertype: u16) {
    debug_assert!(out.len() >= ETH_HDR_LEN);
    out[0..6].copy_from_slice(dst);
    out[6..12].copy_from_slice(src);
    out[12..14].copy_from_slice(&ethertype.to_be_bytes());
}
